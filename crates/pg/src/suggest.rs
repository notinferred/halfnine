//! Rules suggested from patterns in a table's data, each with what in the data suggests it. Postgres's own statistics
//! are read when the table has been analyzed, and otherwise the first rows of it. These only suggest: accepting one tries
//! it on the data first, as adding any rule does, so a suggestion from rough statistics can't become a rule rows break.

use std::collections::HashSet;

use hn_core::{Error, RuleCheck, Suggestion, TableRef};
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, Row};

use crate::PgDatabase;
use crate::changes::Writer;
use crate::errors::from_sqlx;

/// The most rows read when Postgres has no statistics for the table.
const SAMPLE: i64 = 10_000;
/// The most values a "one of" rule lists.
const CHOICES: usize = 10;
/// Fewer rows than this say too little to suggest anything from.
const TOO_FEW: f64 = 20.0;

const COLUMNS: &str = "SELECT a.attname::text, t.typcategory::text, NOT a.attnotnull, \
     a.attgenerated <> '' OR a.attidentity <> '', coalesce(a.attnum = ANY (i.indkey::int2[]), false), format_type(a.atttypid, a.atttypmod) \
     FROM pg_attribute a JOIN pg_type t ON t.oid = a.atttypid \
     LEFT JOIN pg_index i ON i.indrelid = a.attrelid AND i.indisprimary \
     WHERE a.attrelid = $1::oid AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum";

const STATS: &str = "SELECT attname::text, null_frac::float8, n_distinct::float8, most_common_vals::text::text[], \
     most_common_freqs::float8[], histogram_bounds::text::text[] \
     FROM pg_stats WHERE schemaname = $1 AND tablename = $2 \
     AND inherited = (SELECT relkind = 'p' FROM pg_class WHERE oid = $3::oid)";

const KEYED: &str = "SELECT a.attname::text FROM pg_constraint k JOIN pg_attribute a ON a.attrelid = k.conrelid AND a.attnum = ANY (k.conkey) \
     WHERE k.conrelid = $1::oid AND k.contype = 'f'";

const TABLES_IN_SCHEMA: &str = "SELECT c.relname::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
     WHERE n.nspname = $1 AND c.relkind IN ('r', 'p')";

const WRITES: &str = "SELECT n_tup_ins, n_tup_upd + n_tup_del FROM pg_stat_user_tables WHERE relid = $1::oid";

/// What is known about one column's values.
#[derive(Clone, Debug, Default, PartialEq)]
struct Seen {
    name: String,
    /// Postgres's type category: N for numbers, S for text, B for booleans, E for enums.
    category: String,
    type_name: String,
    nullable: bool,
    /// Part of the primary key, or filled by Postgres itself.
    skip: bool,
    /// The share of rows with no value, or empty text.
    empty: f64,
    /// Every value, when there are few enough to list.
    values: Option<Vec<String>>,
    /// The smallest value, for numbers.
    min: Option<f64>,
}

/// How the values were looked at, as the evidence says it.
#[derive(Clone, Debug, PartialEq)]
enum Basis {
    /// Every row, when the table is small enough to read whole.
    All(i64),
    /// The first rows of a bigger table.
    First { read: i64, about: f64 },
    /// Postgres's statistics, from a sample of the whole table.
    Statistics { about: f64 },
}

impl Basis {
    fn rows(&self) -> f64 {
        match self {
            Basis::All(n) => *n as f64,
            Basis::First { read, .. } => *read as f64,
            Basis::Statistics { about } => *about,
        }
    }

    fn words(&self) -> String {
        match self {
            Basis::All(n) => format!("All {} rows", thousands(*n as f64)),
            Basis::First { read, about } => format!("The first {} of about {} rows", thousands(*read as f64), thousands(*about)),
            Basis::Statistics { about } => format!("Postgres's statistics for about {} rows", thousands(*about)),
        }
    }
}

fn thousands(n: f64) -> String {
    let digits = format!("{:.0}", n.max(0.0));
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// What the column values suggest, as pure rules of thumb: always filled, one of a few text values, never negative.
fn from_values(table: &TableRef, columns: &[Seen], basis: &Basis) -> Vec<Suggestion> {
    if basis.rows() < TOO_FEW {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut add = |check: RuleCheck, evidence: String| {
        if let Some(words) = check.sentence(table, true) {
            found.push(Suggestion { check, words, evidence });
        }
    };
    for c in columns.iter().filter(|c| !c.skip && c.category != "B" && c.category != "E") {
        let column = c.name.clone();
        if c.nullable && c.empty == 0.0 {
            add(
                RuleCheck::AlwaysFilled { column: column.clone() },
                format!("{} have a {column}.", basis.words()),
            );
        }
        if c.category == "S"
            && let Some(values) = c.values.as_ref().filter(|v| (2..=CHOICES).contains(&v.len()))
        {
            add(
                RuleCheck::OneOf {
                    column: column.clone(),
                    values: values.clone(),
                },
                format!("{} have one of these {} values.", basis.words(), values.len()),
            );
        }
        let an_id = column == "id" || column.ends_with("_id");
        if c.category == "N" && !an_id && c.min.is_some_and(|m| m >= 0.0) {
            add(
                RuleCheck::Between {
                    column,
                    at_least: Some("0".into()),
                    at_most: None,
                },
                format!("None of {} is below 0.", basis.words().to_lowercase()),
            );
        }
    }
    found
}

/// The tables a column named `<x>_id` most likely points at: `<x>`, `<x>s`, `<x>es`, or `<x>` ending in y as `ies`.
fn targets_for(column: &str) -> Vec<String> {
    let Some(base) = column.strip_suffix("_id").filter(|b| !b.is_empty()) else {
        return Vec::new();
    };
    let mut names = vec![base.to_string(), format!("{base}s"), format!("{base}es")];
    if let Some(stem) = base.strip_suffix('y') {
        names.push(format!("{stem}ies"));
    }
    names
}

/// Whether a key column of `target_type` can hold every value of a column of `column_type`.
fn compatible(column_type: &str, target_type: &str) -> bool {
    let integer = |t: &str| ["smallint", "integer", "bigint"].contains(&t);
    column_type == target_type || (integer(column_type) && integer(target_type))
}

pub(crate) async fn suggest(conn: &mut PgConnection, table: &TableRef, db: &PgDatabase) -> Result<Vec<Suggestion>, Error> {
    sqlx::raw_sql("SET LOCAL statement_timeout = '5s'")
        .execute(&mut *conn)
        .await
        .map_err(from_sqlx)?;
    let relation = sqlx::query(crate::catalog::RELATION)
        .bind(&table.schema)
        .bind(&table.name)
        .fetch_optional(&mut *conn)
        .await
        .map_err(from_sqlx)?
        .ok_or_else(|| crate::not_found(table))?;
    let oid: i64 = relation.try_get(0).map_err(from_sqlx)?;
    let about: f64 = relation.try_get::<Option<f64>, _>(1).map_err(from_sqlx)?.unwrap_or(-1.0);
    let rows = sqlx::query(COLUMNS).bind(oid).fetch_all(&mut *conn).await.map_err(from_sqlx)?;
    let mut columns: Vec<Seen> = rows
        .iter()
        .map(|r| {
            Ok(Seen {
                name: r.try_get(0)?,
                category: r.try_get(1)?,
                nullable: r.try_get(2)?,
                skip: r.try_get::<bool, _>(3)? || r.try_get::<bool, _>(4)?,
                type_name: r.try_get(5)?,
                ..Seen::default()
            })
        })
        .collect::<Result<_, sqlx::Error>>()
        .map_err(from_sqlx)?;
    let stats = sqlx::query(STATS)
        .bind(&table.schema)
        .bind(&table.name)
        .bind(oid)
        .fetch_all(&mut *conn)
        .await
        .map_err(from_sqlx)?;
    let basis = if about > 0.0 && stats.len() >= columns.iter().filter(|c| !c.skip).count() {
        read_statistics(&mut columns, &stats).map_err(from_sqlx)?;
        Basis::Statistics { about }
    } else {
        read_sample(conn, table, &mut columns, about, db).await?
    };
    let mut found = from_values(table, &columns, &basis);
    found.extend(matches(conn, table, oid, &columns, db).await?);
    found.extend(only_grows(conn, table, oid).await?);
    Ok(found)
}

fn read_statistics(columns: &mut [Seen], stats: &[PgRow]) -> Result<(), sqlx::Error> {
    for row in stats {
        let name: String = row.try_get(0)?;
        let Some(c) = columns.iter_mut().find(|c| c.name == name) else {
            continue;
        };
        let null_frac: f64 = row.try_get(1)?;
        let distinct: f64 = row.try_get(2)?;
        let common: Option<Vec<String>> = row.try_get(3)?;
        let freqs: Option<Vec<f64>> = row.try_get(4)?;
        let histogram: Option<Vec<String>> = row.try_get(5)?;
        let has_empty = common.as_ref().is_some_and(|v| v.iter().any(String::is_empty));
        c.empty = if has_empty { null_frac.max(f64::MIN_POSITIVE) } else { null_frac };
        let covered = null_frac + freqs.as_ref().map_or(0.0, |f| f.iter().sum::<f64>());
        if distinct > 0.0 && distinct <= CHOICES as f64 && covered >= 0.999 {
            c.values = common.clone();
        }
        let numbers = common
            .iter()
            .flatten()
            .chain(histogram.iter().flatten())
            .filter_map(|v| v.parse::<f64>().ok());
        c.min = numbers.fold(None, |least: Option<f64>, n| Some(least.map_or(n, |l| l.min(n))));
    }
    Ok(())
}

/// Reads the first rows of a table Postgres has no statistics for: how many are empty, the values when there are few,
/// and the smallest number, in one query.
async fn read_sample(conn: &mut PgConnection, table: &TableRef, columns: &mut [Seen], about: f64, db: &PgDatabase) -> Result<Basis, Error> {
    let looked: Vec<usize> = (0..columns.len()).filter(|&i| !columns[i].skip).collect();
    let mut w = Writer::default();
    w.text("WITH hn_sample AS MATERIALIZED (SELECT ");
    if looked.is_empty() {
        w.text("1");
    }
    for (n, &i) in looked.iter().enumerate() {
        if n > 0 {
            w.text(", ");
        }
        w.name(&columns[i].name);
    }
    w.text(" FROM ").table(table).text(&format!(" LIMIT {SAMPLE}) SELECT count(*)"));
    for &i in &looked {
        let name = columns[i].name.clone();
        w.text(", count(*) FILTER (WHERE ")
            .name(&name)
            .text(" IS NULL OR ")
            .name(&name)
            .text("::text = '')");
        w.text(", ARRAY(SELECT DISTINCT ")
            .name(&name)
            .text("::text FROM hn_sample WHERE ")
            .name(&name);
        w.text(&format!(" IS NOT NULL LIMIT {})", CHOICES + 1));
        if columns[i].category == "N" {
            w.text(", min(").name(&name).text(")::float8");
        } else {
            w.text(", NULL::float8");
        }
    }
    w.text(" FROM hn_sample");
    let row = db
        .query(w.step(String::new(), None).run)
        .persistent(false)
        .fetch_one(&mut *conn)
        .await
        .map_err(from_sqlx)?;
    let read: i64 = row.try_get(0).map_err(from_sqlx)?;
    for (n, &i) in looked.iter().enumerate() {
        let at = 1 + n * 3;
        let empties: i64 = row.try_get(at).map_err(from_sqlx)?;
        let values: Vec<String> = row.try_get(at + 1).map_err(from_sqlx)?;
        let c = &mut columns[i];
        c.empty = if read > 0 { empties as f64 / read as f64 } else { 0.0 };
        c.values = (values.len() <= CHOICES).then_some(values);
        c.min = row.try_get(at + 2).map_err(from_sqlx)?;
    }
    Ok(if read < SAMPLE { Basis::All(read) } else { Basis::First { read, about } })
}

/// "Has a matching row" for a column named like `customer_id`, when a table such as `customers` has a one-column key
/// of a type that fits, no foreign key says so already, and none of the column's first values lacks a match.
async fn matches(conn: &mut PgConnection, table: &TableRef, oid: i64, columns: &[Seen], db: &PgDatabase) -> Result<Vec<Suggestion>, Error> {
    let keyed: HashSet<String> = sqlx::query_scalar(KEYED)
        .bind(oid)
        .fetch_all(&mut *conn)
        .await
        .map_err(from_sqlx)?
        .into_iter()
        .collect();
    let tables: HashSet<String> = sqlx::query_scalar(TABLES_IN_SCHEMA)
        .bind(&table.schema)
        .fetch_all(&mut *conn)
        .await
        .map_err(from_sqlx)?
        .into_iter()
        .collect();
    let mut found = Vec::new();
    for c in columns.iter().filter(|c| !keyed.contains(&c.name)) {
        let Some(name) = targets_for(&c.name).into_iter().find(|t| tables.contains(t) && *t != table.name) else {
            continue;
        };
        let target = TableRef {
            schema: table.schema.clone(),
            name,
        };
        let target_columns = crate::table_columns(&mut *conn, &target).await?;
        let key: Vec<_> = target_columns.iter().filter(|t| t.key_position.is_some()).collect();
        let [key] = key.as_slice() else { continue };
        if !compatible(&c.type_name, &key.type_name) {
            continue;
        }
        let mut w = Writer::default();
        w.text("SELECT count(*), count(*) FILTER (WHERE NOT EXISTS (SELECT FROM ")
            .table(&target)
            .text(" AS m WHERE m.");
        w.name(&key.name)
            .text(" = s.")
            .name(&c.name)
            .text(")) FROM (SELECT ")
            .name(&c.name)
            .text(" FROM ")
            .table(table);
        w.text(" WHERE ").name(&c.name).text(&format!(" IS NOT NULL LIMIT {SAMPLE}) AS s"));
        let row = db
            .query(w.step(String::new(), None).run)
            .persistent(false)
            .fetch_one(&mut *conn)
            .await
            .map_err(from_sqlx)?;
        let (looked, missing): (i64, i64) = (row.try_get(0).map_err(from_sqlx)?, row.try_get(1).map_err(from_sqlx)?);
        if looked < TOO_FEW as i64 || missing > 0 {
            continue;
        }
        let check = RuleCheck::HasMatch {
            column: c.name.clone(),
            target: target.clone(),
            target_column: key.name.clone(),
        };
        if let Some(words) = check.sentence(table, true) {
            let evidence = format!(
                "Each of the {} {} values looked at has a row in {}.",
                thousands(looked as f64),
                c.name,
                target.name
            );
            found.push(Suggestion { check, words, evidence });
        }
    }
    Ok(found)
}

/// "Only grows" when Postgres has counted rows added to the table, and none changed or removed, since its statistics
/// were last reset.
async fn only_grows(conn: &mut PgConnection, table: &TableRef, oid: i64) -> Result<Vec<Suggestion>, Error> {
    let Some(row) = sqlx::query(WRITES).bind(oid).fetch_optional(&mut *conn).await.map_err(from_sqlx)? else {
        return Ok(Vec::new());
    };
    let (added, changed): (i64, i64) = (row.try_get(0).map_err(from_sqlx)?, row.try_get(1).map_err(from_sqlx)?);
    if added <= 100 || changed > 0 {
        return Ok(Vec::new());
    }
    let check = RuleCheck::OnlyGrows;
    Ok(check
        .sentence(table, true)
        .map(|words| Suggestion {
            check,
            words,
            evidence: format!(
                "Since Postgres last reset its statistics, {} rows were added and none changed or removed.",
                thousands(added as f64)
            ),
        })
        .into_iter()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> TableRef {
        TableRef {
            schema: "public".into(),
            name: "invoices".into(),
        }
    }

    fn seen(name: &str, category: &str) -> Seen {
        Seen {
            name: name.into(),
            category: category.into(),
            type_name: if category == "N" { "numeric".into() } else { "text".into() },
            nullable: true,
            ..Seen::default()
        }
    }

    #[test]
    fn suggests_what_the_values_show() {
        let columns = vec![
            Seen {
                values: Some(vec!["draft".into(), "paid".into(), "sent".into()]),
                ..seen("status", "S")
            },
            Seen {
                min: Some(0.0),
                empty: 0.1,
                ..seen("amount", "N")
            },
            Seen {
                min: Some(1.0),
                ..seen("customer_id", "N")
            },
            Seen {
                skip: true,
                ..seen("id", "N")
            },
            seen("paid", "B"),
        ];
        let found = from_values(&table(), &columns, &Basis::All(3812));
        let words: Vec<&str> = found.iter().map(|s| s.words.as_str()).collect();
        assert_eq!(
            words,
            [
                "status is always filled",
                "status is one of draft, paid and sent",
                "amount is at least 0",
                "customer_id is always filled",
            ]
        );
        assert_eq!(found[0].evidence, "All 3,812 rows have a status.");
        assert!(from_values(&table(), &columns, &Basis::All(5)).is_empty(), "too few rows to say");
        let about = Basis::First {
            read: 10_000,
            about: 1_234_567.0,
        };
        assert_eq!(about.words(), "The first 10,000 of about 1,234,567 rows");
    }

    #[test]
    fn guesses_the_table_an_id_points_at() {
        assert_eq!(targets_for("customer_id"), ["customer", "customers", "customeres"]);
        assert!(targets_for("company_id").contains(&"companies".to_string()));
        assert!(targets_for("id").is_empty() && targets_for("name").is_empty());
        assert!(compatible("integer", "bigint") && compatible("uuid", "uuid") && !compatible("text", "integer"));
    }
}
