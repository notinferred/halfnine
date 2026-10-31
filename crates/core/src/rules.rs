use serde::{Deserialize, Serialize};
use specta::Type;

use crate::data::{ColumnRef, TableRef};

/// What correct data is in one table, as the person set it. An agent's change that breaks a rule doesn't commit.
#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub id: String,
    pub table: TableRef,
    pub check: RuleCheck,
    /// The rule in words, as the person wrote it or Halfnine put it.
    pub words: String,
    pub source: RuleSource,
    pub added_ms: f64,
}

/// What a rule checks, and so what breaks it.
#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RuleCheck {
    /// Never NULL, and never empty text.
    AlwaysFilled {
        column: String,
    },
    OneOf {
        column: String,
        values: Vec<String>,
    },
    /// At least one bound is given; each is read with the column's own type.
    Between {
        column: String,
        at_least: Option<String>,
        at_most: Option<String>,
    },
    /// Every value has a row in `target` with the same value in `target_column`, as a foreign key the schema doesn't
    /// declare would.
    HasMatch {
        column: String,
        target: TableRef,
        target_column: String,
    },
    /// Rows are added, never changed or removed.
    OnlyGrows,
    /// A read-only query the person wrote, which returns the rows that break the rule.
    Custom {
        sql: String,
        /// The tables it reads, found from its plan when it was added, so a change to any of them checks it.
        reads: Vec<TableRef>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RuleSource {
    /// The person wrote it.
    Written,
    /// Halfnine suggested it from patterns in the data, and the person accepted it.
    Suggested { evidence: String },
}

/// A rule Halfnine found in the data, for the person to accept or dismiss.
#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub check: RuleCheck,
    pub words: String,
    /// What in the data suggests it, such as "all 3,812 rows have one of these values".
    pub evidence: String,
}

/// The rules a change must keep, as the transaction that makes it checks them.
#[derive(Clone, Debug, Default)]
pub struct Checks {
    pub rules: Vec<Rule>,
    /// Columns hidden from whoever made the change, whose values a report never shows.
    pub hidden: Vec<ColumnRef>,
    /// Roll the change back once it's checked, as when an agent's change is tried before the person is asked.
    pub dry_run: bool,
}

impl Checks {
    /// No rules: the change commits as it always has.
    pub const NONE: Checks = Checks {
        rules: Vec::new(),
        hidden: Vec::new(),
        dry_run: false,
    };
}

/// Values in a sentence: "a", "a and b", "a, b and c".
fn listed(values: &[String]) -> String {
    match values {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

impl RuleCheck {
    /// The rule in words, with its values written out unless they belong to a column hidden from the reader. A
    /// custom rule has none of its own: the person's words are its sentence.
    pub fn sentence(&self, table: &TableRef, show_values: bool) -> Option<String> {
        const NOT_SHOWN: &str = "not shown because it is hidden from agents";
        let table = &table.name;
        Some(match self {
            RuleCheck::AlwaysFilled { column } => format!("{column} is always filled"),
            RuleCheck::OneOf { column, values } if show_values => format!("{column} is one of {}", listed(values)),
            RuleCheck::OneOf { column, .. } => format!("{column} is one of a set of values, {NOT_SHOWN}"),
            RuleCheck::Between { column, .. } if !show_values => format!("{column} stays within bounds, {NOT_SHOWN}"),
            RuleCheck::Between { column, at_least, at_most } => match (at_least, at_most) {
                (Some(low), Some(high)) => format!("{column} is between {low} and {high}"),
                (Some(low), None) => format!("{column} is at least {low}"),
                (None, Some(high)) => format!("{column} is at most {high}"),
                (None, None) => format!("{column} has no bounds"),
            },
            RuleCheck::HasMatch {
                column,
                target,
                target_column,
            } => format!("every {column} in {table} has a row in {} with that {target_column}", target.name),
            RuleCheck::OnlyGrows => format!("{table} only grows: rows are added, never changed or removed"),
            RuleCheck::Custom { .. } => return None,
        })
    }

    /// The column it is about, if it is about one.
    pub fn column(&self) -> Option<&str> {
        match self {
            RuleCheck::AlwaysFilled { column }
            | RuleCheck::OneOf { column, .. }
            | RuleCheck::Between { column, .. }
            | RuleCheck::HasMatch { column, .. } => Some(column),
            RuleCheck::OnlyGrows | RuleCheck::Custom { .. } => None,
        }
    }

    /// Every table a change to which could break it.
    pub fn tables<'a>(&'a self, table: &'a TableRef) -> Vec<&'a TableRef> {
        match self {
            RuleCheck::HasMatch { target, .. } => vec![table, target],
            RuleCheck::Custom { reads, .. } => std::iter::once(table).chain(reads).collect(),
            _ => vec![table],
        }
    }
}

impl Rule {
    /// The rule as a reader with these hidden columns sees it: its values left out when its column is one of them.
    pub fn shown(&self, hidden: &[ColumnRef]) -> String {
        let is_hidden = self.check.column().is_some_and(|c| {
            hidden
                .iter()
                .any(|h| h.schema == self.table.schema && h.table == self.table.name && h.column == c)
        });
        match self.check.sentence(&self.table, false) {
            Some(masked) if is_hidden => masked,
            _ => self.words.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invoices() -> TableRef {
        TableRef {
            schema: "public".into(),
            name: "invoices".into(),
        }
    }

    fn rule(check: RuleCheck) -> Rule {
        Rule {
            id: "r1".into(),
            words: check.sentence(&invoices(), true).unwrap_or_default(),
            table: invoices(),
            check,
            source: RuleSource::Written,
            added_ms: 0.0,
        }
    }

    #[test]
    fn says_each_rule_in_words() {
        let one_of = RuleCheck::OneOf {
            column: "status".into(),
            values: vec!["draft".into(), "sent".into(), "paid".into()],
        };
        assert_eq!(rule(one_of).words, "status is one of draft, sent and paid");
        let at_least = RuleCheck::Between {
            column: "amount".into(),
            at_least: Some("0".into()),
            at_most: None,
        };
        assert_eq!(rule(at_least).words, "amount is at least 0");
        let matches = RuleCheck::HasMatch {
            column: "customer_id".into(),
            target: TableRef {
                schema: "public".into(),
                name: "customers".into(),
            },
            target_column: "id".into(),
        };
        assert_eq!(rule(matches).words, "every customer_id in invoices has a row in customers with that id");
        assert_eq!(
            rule(RuleCheck::OnlyGrows).words,
            "invoices only grows: rows are added, never changed or removed"
        );
    }

    #[test]
    fn never_shows_a_hidden_columns_values() {
        let rule = rule(RuleCheck::OneOf {
            column: "email".into(),
            values: vec!["a@example.com".into()],
        });
        let hidden = [ColumnRef {
            schema: "public".into(),
            table: "invoices".into(),
            column: "email".into(),
        }];
        let shown = rule.shown(&hidden);
        assert!(!shown.contains("a@example.com"), "{shown}");
        assert!(shown.contains("hidden from agents"));
        assert!(rule.shown(&[]).contains("a@example.com"), "the person sees it");
    }
}
