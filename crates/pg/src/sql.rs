//! The SQL the app builds. Identifiers only ever enter it through [`ident`]; values only as bound parameters.

use tu_core::{Error, ErrorKind, Filter, PageRequest, Sort, TableRef};

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

/// A column as the catalog describes it: its name, its type as Postgres writes it, and its place in the primary key.
#[derive(Clone, Debug)]
pub(crate) struct ColumnMeta {
    pub name: String,
    pub type_name: String,
    pub key_position: Option<i32>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Built {
    pub sql: String,
    pub params: Vec<Option<String>>,
}

/// `SELECT` for one page: every column as text and cut to [`CELL_LIMIT`], in a stable order, one row more than asked
/// so the caller knows whether another page follows.
pub(crate) fn page(request: &PageRequest, columns: &[ColumnMeta]) -> Result<Built, Error> {
    let select = columns
        .iter()
        .map(|c| format!("left({}::text, {})", ident(&c.name), CELL_LIMIT + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let mut params = Vec::new();
    let condition = condition(request.filter.as_ref(), columns, &mut params)?;
    let order = order(request.sort.as_ref(), columns)?;
    let limit = request.limit.clamp(1, PAGE_LIMIT) + 1;
    Ok(Built {
        sql: format!(
            "SELECT {select} FROM {}{condition}{order} LIMIT {limit} OFFSET {}",
            table(&request.table),
            request.offset
        ),
        params,
    })
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
        Some(Filter::Equals { values }) => {
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
            Ok(format!(" WHERE {}", terms.join(" AND ")))
        }
    }
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

fn find<'a>(columns: &'a [ColumnMeta], name: &str) -> Result<&'a ColumnMeta, Error> {
    columns
        .iter()
        .find(|c| c.name == name)
        .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("There is no column named {name}.")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tu_core::ColumnValue;

    fn columns() -> Vec<ColumnMeta> {
        vec![
            ColumnMeta {
                name: "id".into(),
                type_name: "bigint".into(),
                key_position: Some(1),
            },
            ColumnMeta {
                name: "say \"hi\"".into(),
                type_name: "text".into(),
                key_position: None,
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
    fn refuses_columns_the_table_lacks() {
        let sort = Sort {
            column: "missing".into(),
            descending: false,
        };
        assert_eq!(page(&request(Some(sort), None), &columns()).unwrap_err().kind, ErrorKind::NotFound);
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
}
