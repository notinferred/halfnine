//! The SQL the app builds. Identifiers only ever enter it through [`ident`]; values only as bound parameters.

use hn_core::{ColumnValue, Error, ErrorKind, Filter, PageRequest, Sort, TableRef};

/// The longest value the grid receives; longer ones are cut and marked, so one huge cell can't stall a page.
pub(crate) const CELL_LIMIT: usize = 65_536;
/// The most rows one page may ask for.
pub(crate) const PAGE_LIMIT: u32 = 1_000;

/// Quotes an identifier as Postgres does: in double quotes, with any double quote inside doubled.
pub(crate) fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

pub(crate) fn table(t: &TableRef) -> String {
    format!("{}.{}", ident(&t.schema), ident(&t.name))
}

/// A column as the catalog describes it: its name, its type as Postgres writes it, its place in the primary key, and
/// whether Postgres always fills it itself.
#[derive(Clone, Debug)]
pub(crate) struct ColumnMeta {
    pub name: String,
    pub type_name: String,
    pub key_position: Option<i32>,
    pub generated: bool,
    /// Generated always as identity: written only with `OVERRIDING SYSTEM VALUE`, as when a deleted row is put back.
    pub identity: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Built {
    pub sql: String,
    pub params: Vec<Option<String>>,
}

/// `SELECT` for one page: every column as text and cut to [`CELL_LIMIT`], in a stable order, one row more than asked
/// so the caller knows whether another page follows.
pub(crate) fn page(request: &PageRequest, columns: &[ColumnMeta]) -> Result<Built, Error> {
    let window = format!(" LIMIT {} OFFSET {}", request.limit.clamp(1, PAGE_LIMIT) + 1, request.offset);
    select(
        &request.table,
        request.filter.as_ref(),
        request.sort.as_ref(),
        columns,
        |c| format!("left({c}::text, {})", CELL_LIMIT + 1),
        &window,
    )
}

/// `SELECT` for an export: every row and the whole of every value, in the same order the grid shows.
pub(crate) fn export(table_ref: &TableRef, filter: Option<&Filter>, sort: Option<&Sort>, columns: &[ColumnMeta]) -> Result<Built, Error> {
    select(table_ref, filter, sort, columns, |c| format!("{c}::text"), "")
}

fn select(
    table_ref: &TableRef,
    filter: Option<&Filter>,
    sort: Option<&Sort>,
    columns: &[ColumnMeta],
    as_text: impl Fn(&str) -> String,
    window: &str,
) -> Result<Built, Error> {
    let select = columns.iter().map(|c| as_text(&ident(&c.name))).collect::<Vec<_>>().join(", ");
    let mut params = Vec::new();
    let condition = condition(filter, columns, &mut params)?;
    let order = order(sort, columns)?;
    Ok(Built {
        sql: format!("SELECT {select} FROM {}{condition}{order}{window}", table(table_ref)),
        params,
    })
}

/// Cuts `text` to [`CELL_LIMIT`] characters, and says whether it did.
pub(crate) fn cut(text: &mut String) -> bool {
    match text.char_indices().nth(CELL_LIMIT) {
        Some((at, _)) => {
            text.truncate(at);
            true
        }
        None => false,
    }
}

pub(crate) fn count(table_ref: &TableRef, filter: Option<&Filter>, columns: &[ColumnMeta]) -> Result<Built, Error> {
    let mut params = Vec::new();
    let condition = condition(filter, columns, &mut params)?;
    Ok(Built {
        sql: format!("SELECT count(*)::float8 FROM {}{condition}", table(table_ref)),
        params,
    })
}

fn condition(filter: Option<&Filter>, columns: &[ColumnMeta], params: &mut Vec<Option<String>>) -> Result<String, Error> {
    match filter {
        None => Ok(String::new()),
        Some(Filter::Where { sql }) if sql.trim().is_empty() => Ok(String::new()),
        // The condition is the person's own SQL, as in the query editor. Postgres runs it as one prepared statement,
        // which refuses a second statement after it.
        Some(Filter::Where { sql }) => Ok(format!(" WHERE ({sql})")),
        Some(Filter::Equals { values }) if values.is_empty() => Ok(String::new()),
        Some(Filter::Equals { values }) => Ok(format!(" WHERE {}", equals(values, columns, params)?)),
        Some(Filter::Keys { keys }) if keys.is_empty() => Ok(" WHERE false".to_string()),
        Some(Filter::Keys { keys }) if keys.len() > KEYS => Err(Error::new(
            ErrorKind::Invalid,
            format!("Halfnine shows at most {KEYS} rows picked by their key at once."),
        )),
        Some(Filter::Keys { keys }) => {
            let rows = keys
                .iter()
                .map(|key| equals(key, columns, params).map(|terms| format!("({terms})")))
                .collect::<Result<Vec<_>, Error>>()?;
            Ok(format!(" WHERE {}", rows.join(" OR ")))
        }
    }
}

/// The most rows a [`Filter::Keys`] may name.
pub(crate) const KEYS: usize = 500;

fn equals(values: &[ColumnValue], columns: &[ColumnMeta], params: &mut Vec<Option<String>>) -> Result<String, Error> {
    let terms = values
        .iter()
        .map(|v| {
            let column = find(columns, &v.column)?;
            Ok(match &v.value {
                None => format!("{} IS NULL", ident(&column.name)),
                Some(text) => {
                    params.push(Some(text.clone()));
                    // Cast through text with the catalog's own type name, so Postgres parses the value and an index still applies.
                    format!("{} = ${}::text::{}", ident(&column.name), params.len(), column.type_name)
                }
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(terms.join(" AND "))
}

/// The requested sort, then the primary key, so paging through equal values never skips or repeats a row.
fn order(sort: Option<&Sort>, columns: &[ColumnMeta]) -> Result<String, Error> {
    let mut terms = Vec::new();
    if let Some(sort) = sort {
        let column = find(columns, &sort.column)?;
        terms.push(format!("{} {}", ident(&column.name), if sort.descending { "DESC" } else { "ASC" }));
    }
    let mut key: Vec<&ColumnMeta> = columns.iter().filter(|c| c.key_position.is_some()).collect();
    key.sort_by_key(|c| c.key_position);
    for column in key {
        if sort.is_none_or(|s| s.column != column.name) {
            terms.push(ident(&column.name));
        }
    }
    Ok(if terms.is_empty() {
        String::new()
    } else {
        format!(" ORDER BY {}", terms.join(", "))
    })
}

pub(crate) fn find<'a>(columns: &'a [ColumnMeta], name: &str) -> Result<&'a ColumnMeta, Error> {
    columns
        .iter()
        .find(|c| c.name == name)
        .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("There is no column named {name}.")))
}

/// The most values the foreign-key picker shows at once; typing narrows them.
pub(crate) const CHOICES: u32 = 50;

/// Values of `column` to pick from, each beside the row's first text column, matching `search` in either.
pub(crate) fn choices(table_ref: &TableRef, column: &str, search: &str, columns: &[ColumnMeta]) -> Result<Built, Error> {
    let key = ident(&find(columns, column)?.name);
    let label = columns
        .iter()
        .find(|c| c.name != column && ["text", "character", "citext", "name"].iter().any(|t| c.type_name.starts_with(t)))
        .map(|c| ident(&c.name));
    let mut sql = format!(
        "SELECT {key}::text, {} FROM {} WHERE {key} IS NOT NULL",
        label.as_ref().map_or("NULL::text".to_string(), |l| format!("left({l}::text, 200)")),
        table(table_ref)
    );
    let mut params = Vec::new();
    let search = search.trim();
    if !search.is_empty() {
        let escaped = search.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        params.push(Some(format!("%{escaped}%")));
        sql.push_str(&format!(" AND ({key}::text ILIKE $1"));
        if let Some(label) = &label {
            sql.push_str(&format!(" OR {label}::text ILIKE $1"));
        }
        sql.push(')');
    }
    sql.push_str(&format!(" ORDER BY {key} LIMIT {CHOICES}"));
    Ok(Built { sql, params })
}

/// An import's three statements: a temporary table with one text column per file column, the `COPY` that fills it
/// from the file, and the `INSERT` that moves the chosen columns into the table, each read with its column's type.
pub(crate) struct Import {
    pub create: String,
    pub copy: String,
    pub insert: String,
}

pub(crate) fn import(table_ref: &TableRef, targets: &[Option<String>], delimiter: u8, columns: &[ColumnMeta]) -> Result<Import, Error> {
    let delimiter = match delimiter {
        b',' => "','",
        b';' => "';'",
        b'\t' => "E'\\t'",
        b'|' => "'|'",
        _ => {
            return Err(Error::new(
                ErrorKind::Invalid,
                "Halfnine reads files whose values are separated by commas, semicolons, tabs or bars.",
            ));
        }
    };
    let mut into = Vec::new();
    let mut from = Vec::new();
    for (i, target) in targets.iter().enumerate() {
        let Some(name) = target else { continue };
        let column = find(columns, name)?;
        if column.generated {
            return Err(Error::new(
                ErrorKind::Invalid,
                format!("Postgres fills {name} itself, so nothing can be imported into it."),
            ));
        }
        if into.contains(&ident(name)) {
            return Err(Error::new(
                ErrorKind::Invalid,
                format!("Two columns of the file go into {name}. Choose one."),
            ));
        }
        into.push(ident(name));
        from.push(format!("{}::{}", ident(&format!("c{}", i + 1)), column.type_name));
    }
    if into.is_empty() {
        return Err(Error::new(ErrorKind::Invalid, "Choose at least one column to import."));
    }
    let staging = (1..=targets.len())
        .map(|i| format!("{} text", ident(&format!("c{i}"))))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(Import {
        create: format!("CREATE TEMPORARY TABLE hn_import ({staging}) ON COMMIT DROP"),
        copy: format!("COPY hn_import FROM STDIN WITH (FORMAT csv, HEADER true, DELIMITER {delimiter})"),
        insert: format!(
            "INSERT INTO {} ({}) SELECT {} FROM hn_import",
            table(table_ref),
            into.join(", "),
            from.join(", ")
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn columns() -> Vec<ColumnMeta> {
        vec![
            ColumnMeta {
                name: "id".into(),
                type_name: "bigint".into(),
                key_position: Some(1),
                generated: false,
                identity: false,
            },
            ColumnMeta {
                name: "say \"hi\"".into(),
                type_name: "text".into(),
                key_position: None,
                generated: false,
                identity: false,
            },
        ]
    }

    fn request(sort: Option<Sort>, filter: Option<Filter>) -> PageRequest {
        PageRequest {
            table: TableRef {
                schema: "public".into(),
                name: "odd\"table".into(),
            },
            sort,
            filter,
            offset: 500,
            limit: 5_000,
        }
    }

    #[test]
    fn quotes_identifiers_by_doubling_quotes() {
        assert_eq!(ident("plain"), "\"plain\"");
        assert_eq!(ident("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn pages_in_key_order_with_a_capped_limit() {
        let built = page(&request(None, None), &columns()).unwrap();
        assert_eq!(
            built.sql,
            "SELECT left(\"id\"::text, 65537), left(\"say \"\"hi\"\"\"::text, 65537) FROM \"public\".\"odd\"\"table\" \
             ORDER BY \"id\" LIMIT 1001 OFFSET 500"
        );
        assert!(built.params.is_empty());
    }

    #[test]
    fn sorts_then_breaks_ties_by_key() {
        let sort = Sort {
            column: "say \"hi\"".into(),
            descending: true,
        };
        let built = page(&request(Some(sort), None), &columns()).unwrap();
        assert!(built.sql.contains("ORDER BY \"say \"\"hi\"\"\" DESC, \"id\" LIMIT"));
    }

    #[test]
    fn follows_keys_with_bound_values_and_nulls() {
        let filter = Filter::Equals {
            values: vec![
                ColumnValue {
                    column: "id".into(),
                    value: Some("42".into()),
                },
                ColumnValue {
                    column: "say \"hi\"".into(),
                    value: None,
                },
            ],
        };
        let built = page(&request(None, Some(filter)), &columns()).unwrap();
        assert!(
            built
                .sql
                .contains("WHERE \"id\" = $1::text::bigint AND \"say \"\"hi\"\"\" IS NULL ORDER BY")
        );
        assert_eq!(built.params, vec![Some("42".to_string())]);
    }

    #[test]
    fn picks_rows_by_key_with_bound_values() {
        let key = |id: &str| {
            vec![ColumnValue {
                column: "id".into(),
                value: Some(id.into()),
            }]
        };
        let built = page(
            &request(
                None,
                Some(Filter::Keys {
                    keys: vec![key("1"), key("2")],
                }),
            ),
            &columns(),
        )
        .unwrap();
        assert!(
            built
                .sql
                .contains("WHERE (\"id\" = $1::text::bigint) OR (\"id\" = $2::text::bigint) ORDER BY")
        );
        assert_eq!(built.params, vec![Some("1".to_string()), Some("2".to_string())]);
        let none = page(&request(None, Some(Filter::Keys { keys: vec![] })), &columns()).unwrap();
        assert!(none.sql.contains(" WHERE false ORDER BY"));
        let too_many = Filter::Keys {
            keys: (0..=KEYS).map(|i| key(&i.to_string())).collect(),
        };
        assert_eq!(page(&request(None, Some(too_many)), &columns()).unwrap_err().kind, ErrorKind::Invalid);
    }

    #[test]
    fn refuses_columns_the_table_lacks() {
        let sort = Sort {
            column: "missing".into(),
            descending: false,
        };
        assert_eq!(page(&request(Some(sort), None), &columns()).unwrap_err().kind, ErrorKind::NotFound);
    }

    #[test]
    fn exports_every_row_whole_in_the_same_order() {
        let built = export(
            &TableRef {
                schema: "s".into(),
                name: "t".into(),
            },
            None,
            None,
            &columns(),
        )
        .unwrap();
        assert_eq!(
            built.sql,
            "SELECT \"id\"::text, \"say \"\"hi\"\"\"::text FROM \"s\".\"t\" ORDER BY \"id\""
        );
    }

    #[test]
    fn cuts_long_text_at_the_limit_only() {
        let mut short = "é".repeat(CELL_LIMIT);
        assert!(!cut(&mut short));
        let mut long = "é".repeat(CELL_LIMIT + 1);
        assert!(cut(&mut long));
        assert_eq!(long.chars().count(), CELL_LIMIT);
    }

    #[test]
    fn counts_with_the_same_condition() {
        let filter = Filter::Where { sql: "id > 3".into() };
        let built = count(
            &TableRef {
                schema: "s".into(),
                name: "t".into(),
            },
            Some(&filter),
            &columns(),
        )
        .unwrap();
        assert_eq!(built.sql, "SELECT count(*)::float8 FROM \"s\".\"t\" WHERE (id > 3)");
    }

    #[test]
    fn picks_keys_by_their_value_or_their_label() {
        let built = choices(
            &TableRef {
                schema: "s".into(),
                name: "t".into(),
            },
            "id",
            " 50%_off ",
            &columns(),
        )
        .unwrap();
        assert_eq!(
            built.sql,
            "SELECT \"id\"::text, left(\"say \"\"hi\"\"\"::text, 200) FROM \"s\".\"t\" WHERE \"id\" IS NOT NULL \
             AND (\"id\"::text ILIKE $1 OR \"say \"\"hi\"\"\"::text ILIKE $1) ORDER BY \"id\" LIMIT 50"
        );
        assert_eq!(built.params, vec![Some("%50\\%\\_off%".to_string())]);
    }

    #[test]
    fn imports_through_a_staging_table_skipping_columns() {
        let targets = vec![None, Some("say \"hi\"".to_string()), Some("id".to_string())];
        let import = import(
            &TableRef {
                schema: "s".into(),
                name: "t".into(),
            },
            &targets,
            b';',
            &columns(),
        )
        .unwrap();
        assert_eq!(
            import.create,
            "CREATE TEMPORARY TABLE hn_import (\"c1\" text, \"c2\" text, \"c3\" text) ON COMMIT DROP"
        );
        assert_eq!(import.copy, "COPY hn_import FROM STDIN WITH (FORMAT csv, HEADER true, DELIMITER ';')");
        assert_eq!(
            import.insert,
            "INSERT INTO \"s\".\"t\" (\"say \"\"hi\"\"\", \"id\") SELECT \"c2\"::text, \"c3\"::bigint FROM hn_import"
        );
        let twice = vec![Some("id".to_string()), Some("id".to_string())];
        assert_eq!(
            super::import(
                &TableRef {
                    schema: "s".into(),
                    name: "t".into()
                },
                &twice,
                b',',
                &columns()
            )
            .err()
            .unwrap()
            .kind,
            ErrorKind::Invalid
        );
    }
}
