//! Columns hidden from agents. Postgres says which table column each result column comes from, so a hidden one is
//! masked wherever a query returns it as it is. A statement that filters on a hidden column, or computes with it, is
//! found in Postgres's own plan and refused. This stops accidental exposure; a statement that hides the column behind
//! a view, or a subquery Postgres keeps apart, can still get past it.

use hn_core::ColumnRef;
use serde_json::Value;

use crate::changes::shown_ident;

/// What a hidden column's value is replaced with.
pub(crate) const MASK: &str = "[hidden]";

/// How a statement may use a hidden column.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Allow {
    /// Returned as it is, to be masked: a query.
    Returned,
    /// Not at all: a write, which could copy it somewhere visible.
    Nothing,
}

/// Keys of a plan node that name things rather than hold expressions.
const NAMES: [&str; 12] = [
    "Node Type",
    "Relation Name",
    "Schema",
    "Alias",
    "Index Name",
    "CTE Name",
    "Subplan Name",
    "Parent Relationship",
    "Join Type",
    "Strategy",
    "Operation",
    "Scan Direction",
];

/// The first hidden column the plan uses in a way `allow` doesn't permit.
pub(crate) fn misuse(plan: &Value, hidden: &[ColumnRef], allow: Allow) -> Option<ColumnRef> {
    let mut scans = Vec::new();
    collect_scans(plan, &mut scans);
    let mut texts = Vec::new();
    collect_texts(plan, &mut texts);
    for (schema, relation, alias) in &scans {
        for column in hidden.iter().filter(|h| &h.schema == schema && &h.table == relation) {
            let name = shown_ident(&column.column);
            let qualified = format!("{}.{name}", shown_ident(alias));
            let whole_row = format!("{}.*", shown_ident(alias));
            for text in &texts {
                let code = without_literals(text);
                if code.contains(&whole_row) {
                    return Some(column.clone());
                }
                let mentions = mentions(&code, &qualified, false) || mentions(&code, &name, true);
                let returned = code.trim() == qualified || code.trim() == name;
                if mentions && (allow == Allow::Nothing || !returned) {
                    return Some(column.clone());
                }
            }
        }
    }
    None
}

/// Every table a plan scans: its schema, name and the alias its expressions use.
fn collect_scans(node: &Value, scans: &mut Vec<(String, String, String)>) {
    match node {
        Value::Object(map) => {
            if let (Some(Value::String(schema)), Some(Value::String(relation)), Some(Value::String(alias))) =
                (map.get("Schema"), map.get("Relation Name"), map.get("Alias"))
            {
                scans.push((schema.clone(), relation.clone(), alias.clone()));
            }
            map.values().for_each(|v| collect_scans(v, scans));
        }
        Value::Array(items) => items.iter().for_each(|v| collect_scans(v, scans)),
        _ => {}
    }
}

fn collect_texts<'a>(node: &'a Value, texts: &mut Vec<&'a str>) {
    match node {
        Value::Object(map) => {
            for (key, value) in map {
                if !NAMES.contains(&key.as_str()) {
                    collect_texts(value, texts);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|v| collect_texts(v, texts)),
        Value::String(text) => texts.push(text),
        _ => {}
    }
}

/// `text` with every string literal emptied, so a column's name inside one isn't taken for the column.
fn without_literals(text: &str) -> String {
    let mut code = String::with_capacity(text.len());
    let mut inside = false;
    for c in text.chars() {
        if c == '\'' {
            inside = !inside;
            code.push(c);
        } else if !inside {
            code.push(c);
        }
    }
    code
}

/// Whether `code` names `token` as a whole name. `bare` asks for it unqualified, not after a dot.
fn mentions(code: &str, token: &str, bare: bool) -> bool {
    let part_of_name = |c: char| c.is_alphanumeric() || c == '_' || c == '$' || c == '"';
    code.match_indices(token).any(|(at, _)| {
        let before = code[..at].chars().next_back();
        let after = code[at + token.len()..].chars().next();
        before.is_none_or(|c| !part_of_name(c) && !(bare && c == '.')) && after.is_none_or(|c| !part_of_name(c))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn email() -> Vec<ColumnRef> {
        vec![ColumnRef {
            schema: "public".into(),
            table: "users".into(),
            column: "email".into(),
        }]
    }

    fn scan(output: &[&str], filter: Option<&str>) -> Value {
        let mut node = json!({"Node Type": "Seq Scan", "Relation Name": "users", "Schema": "public", "Alias": "u", "Output": output});
        if let Some(filter) = filter {
            node["Filter"] = json!(filter);
        }
        json!([{"Plan": node}])
    }

    #[test]
    fn lets_a_query_return_a_hidden_column_to_be_masked() {
        assert!(misuse(&scan(&["u.id", "u.email"], None), &email(), Allow::Returned).is_none());
        assert!(misuse(&scan(&["id", "email"], None), &email(), Allow::Returned).is_none());
        assert!(misuse(&scan(&["u.id"], Some("(u.note = 'email me'::text)")), &email(), Allow::Returned).is_none());
        assert!(misuse(&scan(&["u.email_sent"], None), &email(), Allow::Returned).is_none());
    }

    #[test]
    fn refuses_filtering_or_computing_with_one() {
        assert!(misuse(&scan(&["u.id"], Some("(u.email ~~ 'a%'::text)")), &email(), Allow::Returned).is_some());
        assert!(misuse(&scan(&["lower(email)"], None), &email(), Allow::Returned).is_some());
        assert!(misuse(&scan(&["to_jsonb(u.*)"], None), &email(), Allow::Returned).is_some());
        let join = json!([{"Plan": {"Node Type": "Hash Join", "Hash Cond": "(o.contact = u.email)", "Plans": [
            {"Node Type": "Seq Scan", "Relation Name": "users", "Schema": "public", "Alias": "u", "Output": ["u.email"]}
        ]}}]);
        assert!(misuse(&join, &email(), Allow::Returned).is_some());
    }

    #[test]
    fn refuses_a_write_that_reads_one_at_all() {
        assert!(misuse(&scan(&["u.email"], None), &email(), Allow::Nothing).is_some());
        assert!(misuse(&scan(&["'x'::text", "u.ctid"], None), &email(), Allow::Nothing).is_none());
    }

    #[test]
    fn matches_names_as_postgres_quotes_them() {
        let hidden = vec![ColumnRef {
            schema: "public".into(),
            table: "users".into(),
            column: "E-mail".into(),
        }];
        assert!(misuse(&scan(&["upper(u.\"E-mail\")"], None), &hidden, Allow::Returned).is_some());
        assert!(misuse(&scan(&["u.\"E-mail\""], None), &hidden, Allow::Returned).is_none());
    }
}
