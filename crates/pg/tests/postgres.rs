//! The adapter against a real Postgres loaded with `fixture.sql`. CI provides one through `TU_TEST_DATABASE_URL`;
//! without that variable these tests report that they were skipped and pass, so a laptop never needs a database.

use tu_core::{
    Cell, ColumnValue, ConnectionSpec, Database, ErrorKind, ExportSource, Filter, Label, PageRequest, RelationKind, RowSink, Sort, TableRef, TlsMode,
};
use tu_pg::{PgDatabase, Secrets};

async fn connect(read_only: bool) -> Option<PgDatabase> {
    let Ok(url) = std::env::var("TU_TEST_DATABASE_URL") else {
        eprintln!("skipped: TU_TEST_DATABASE_URL is not set");
        return None;
    };
    let parts = tu_core::parse_url(&url).expect("TU_TEST_DATABASE_URL is a postgres:// URL");
    let spec = ConnectionSpec {
        id: "test".into(),
        name: "test".into(),
        host: parts.host,
        port: parts.port,
        user: parts.user,
        database: parts.database,
        tls: parts.tls.unwrap_or(TlsMode::Disable),
        label: Label::None,
        read_only,
        ssh: None,
    };
    let secrets = Secrets {
        password: parts.password,
        ssh: None,
    };
    Some(
        PgDatabase::connect(&spec, None, &secrets)
            .await
            .expect("the test database accepts connections"),
    )
}

fn table(name: &str) -> TableRef {
    TableRef {
        schema: "tu_fixture".into(),
        name: name.into(),
    }
}

fn request(name: &str, filter: Option<Filter>, sort: Option<Sort>, offset: u32, limit: u32) -> PageRequest {
    PageRequest {
        table: table(name),
        sort,
        filter,
        offset,
        limit,
    }
}

fn column(page: &tu_core::Page, name: &str) -> usize {
    page.columns.iter().position(|c| c.name == name).expect("the column is on the page")
}

#[tokio::test]
async fn reports_the_server_and_its_databases() {
    let Some(db) = connect(false).await else { return };
    let server = db.server().await.unwrap();
    assert!(!server.version.is_empty());
    assert!(server.databases.contains(&server.database));
}

#[tokio::test]
async fn lists_every_kind_of_object_but_partitions() {
    let Some(db) = connect(false).await else { return };
    let schema = db.schema().await.unwrap();
    let node = schema.iter().find(|n| n.name == "tu_fixture").unwrap();
    let kind = |name: &str| node.relations.iter().find(|r| r.name == name).map(|r| r.kind);
    assert_eq!(kind("invoices"), Some(RelationKind::Table));
    let invoices = node.relations.iter().find(|r| r.name == "invoices").unwrap();
    assert_eq!(invoices.columns[..3], ["id", "customer_id", "amount"]);
    assert_eq!(kind("odd \"table\""), Some(RelationKind::Table));
    assert_eq!(kind("events"), Some(RelationKind::PartitionedTable));
    assert_eq!(kind("events_2026"), None);
    assert_eq!(kind("open_invoices"), Some(RelationKind::View));
    assert_eq!(kind("totals"), Some(RelationKind::MaterializedView));
    assert!(node.functions.contains(&"add_one(i integer)".to_string()));
    assert!(node.sequences.contains(&"invoices_id_seq".to_string()));
    let mood = node.enums.iter().find(|e| e.name == "mood").unwrap();
    assert_eq!(mood.values, ["calm", "busy", "it's complicated"]);
    assert!(schema.iter().all(|n| n.name != "pg_catalog" && n.name != "information_schema"));
}

#[tokio::test]
async fn reads_columns_keys_and_foreign_keys_both_ways() {
    let Some(db) = connect(false).await else { return };
    let s = db.structure(&table("invoices")).await.unwrap();
    assert_eq!(s.primary_key, ["id"]);
    let col = |name: &str| s.columns.iter().find(|c| c.name == name).unwrap();
    assert_eq!(col("amount").type_name, "numeric(12,2)");
    assert_eq!(col("id").default.as_deref(), Some("generated always as identity"));
    assert_eq!(col("status").default.as_deref(), Some("'open'::text"));
    assert_eq!(
        col("doubled").default.as_deref(),
        Some("generated always as ((amount * (2)::numeric)) stored")
    );
    assert!(col("paid_at").nullable && !col("due_on").nullable);
    assert_eq!(col("note").comment.as_deref(), Some("Free text; NULL and empty are different."));
    assert!(s.indexes.iter().any(|i| i.name == "invoices_status_due_on_idx" && !i.unique));
    assert!(s.indexes.iter().any(|i| i.primary && i.unique));
    assert!(s.constraints.iter().any(|c| c.definition == "CHECK ((amount >= (0)::numeric))"));
    let out = &s.foreign_keys[0];
    assert_eq!(
        (out.columns.as_slice(), out.target.name.as_str(), out.target_columns.as_slice()),
        (&["customer_id".to_string()][..], "customers", &["id".to_string()][..])
    );
    let incoming = &s.referenced_by[0];
    assert_eq!(
        (incoming.source.name.as_str(), incoming.source_columns.as_slice()),
        ("line_items", &["invoice_id".to_string()][..])
    );

    let composite = db.structure(&table("line_items")).await.unwrap();
    assert_eq!(composite.primary_key, ["invoice_id", "line"]);
    assert_eq!(db.structure(&table("missing")).await.unwrap_err().kind, ErrorKind::NotFound);
}

#[tokio::test]
async fn shows_values_exactly_as_postgres_prints_them() {
    let Some(db) = connect(false).await else { return };
    let page = db.page(&request("invoices", None, None, 9, 2)).await.unwrap();
    assert_eq!(page.rows.len(), 2);
    let value = |row: usize, name: &str| page.rows[row][column(&page, name)].clone();
    assert_eq!(value(0, "id").as_deref(), Some("10"));
    assert_eq!(value(0, "note"), None);
    assert_eq!(value(1, "note").as_deref(), Some(""));
    assert_eq!(value(0, "tags").as_deref(), Some(r#"{a,"b,c","quote\"d"}"#));
    assert_eq!(value(0, "meta").as_deref(), Some(r#"{"n": 10, "nested": {"ok": true}}"#));
    assert_eq!(value(0, "blob").as_deref(), Some(r"\xdeadbeef"));
    assert_eq!(value(0, "mood").as_deref(), Some("busy"));
    assert_eq!(page.columns[column(&page, "amount")].type_name, "numeric(12,2)");
    assert!(page.has_more);
}

#[tokio::test]
async fn follows_keys_sorts_and_counts() {
    let Some(db) = connect(false).await else { return };
    let filter = Filter::Equals {
        values: vec![ColumnValue {
            column: "customer_id".into(),
            value: Some("7".into()),
        }],
    };
    let sort = Sort {
        column: "amount".into(),
        descending: true,
    };
    let page = db.page(&request("invoices", Some(filter.clone()), Some(sort), 0, 10)).await.unwrap();
    let customer = column(&page, "customer_id");
    let amount = column(&page, "amount");
    assert!(page.rows.iter().all(|r| r[customer].as_deref() == Some("7")));
    let amounts: Vec<f64> = page.rows.iter().map(|r| r[amount].as_deref().unwrap().parse().unwrap()).collect();
    assert!(amounts.windows(2).all(|w| w[0] >= w[1]));
    assert_eq!(db.count(&table("invoices"), Some(&filter)).await.unwrap(), 2400.0);

    let nothing = Filter::Equals {
        values: vec![ColumnValue {
            column: "paid_at".into(),
            value: None,
        }],
    };
    let unpaid = db.count(&table("invoices"), Some(&nothing)).await.unwrap();
    assert!(unpaid > 0.0 && unpaid < 120_001.0);
}

#[tokio::test]
async fn cuts_huge_values_and_marks_them() {
    let Some(db) = connect(false).await else { return };
    let filter = Filter::Where {
        sql: "length(note) > 1000".into(),
    };
    let page = db.page(&request("invoices", Some(filter), None, 0, 10)).await.unwrap();
    let note = column(&page, "note");
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0][note].as_ref().map(String::len), Some(65_536));
    assert_eq!((page.cut[0].row, page.cut[0].column), (0, note as u32));
}

#[tokio::test]
async fn reads_tables_with_awkward_names_and_no_key() {
    let Some(db) = connect(false).await else { return };
    let page = db.page(&request("odd \"table\"", None, None, 0, 100)).await.unwrap();
    let names: Vec<&str> = page.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["weird col", "ünïcödé"]);
    assert_eq!(page.rows.len(), 2);
    assert!(!page.has_more);
}

#[tokio::test]
async fn keeps_postgres_codes_on_errors() {
    let Some(db) = connect(false).await else { return };
    let filter = Filter::Where {
        sql: "no_such_column = 1".into(),
    };
    let error = db.page(&request("invoices", Some(filter), None, 0, 10)).await.unwrap_err();
    assert_eq!((error.kind, error.code.as_deref()), (ErrorKind::Query, Some("42703")));

    let second = Filter::Where {
        sql: "true; DROP TABLE tu_fixture.customers".into(),
    };
    assert!(db.page(&request("invoices", Some(second), None, 0, 10)).await.is_err());
    assert!(db.structure(&table("customers")).await.is_ok());
}

#[tokio::test]
async fn read_only_connections_are_refused_by_the_server() {
    let Some(writable) = connect(false).await else { return };
    let touch = || {
        Some(Filter::Where {
            sql: "tu_fixture.touch()".into(),
        })
    };
    // Browsing only reads, on every connection, so a filter can't write even where the editor may.
    let error = writable.page(&request("customers", touch(), None, 0, 1)).await.unwrap_err();
    assert_eq!(error.code.as_deref(), Some("25006"));
    assert!(writable.run(&statements(&["SELECT tu_fixture.touch()"])).await.unwrap().failed.is_none());

    let Some(read_only) = connect(true).await else { return };
    let error = read_only.page(&request("customers", touch(), None, 0, 1)).await.unwrap_err();
    assert_eq!((error.kind, error.code.as_deref()), (ErrorKind::ReadOnly, Some("25006")));
}

#[tokio::test]
async fn a_browse_filter_cannot_switch_read_only_off_for_later_queries() {
    let Some(db) = connect(true).await else { return };
    let switch_off = Filter::Where {
        sql: "set_config('default_transaction_read_only', 'off', false) IS NOT NULL".into(),
    };
    for _ in 0..3 {
        db.page(&request("customers", Some(switch_off.clone()), None, 0, 1)).await.unwrap();
    }
    let touch = Filter::Where {
        sql: "tu_fixture.touch()".into(),
    };
    assert_eq!(
        db.page(&request("customers", Some(touch), None, 0, 1)).await.unwrap_err().kind,
        ErrorKind::ReadOnly
    );
}

fn statements(sql: &[&str]) -> Vec<String> {
    sql.iter().map(|s| s.to_string()).collect()
}

#[tokio::test]
async fn runs_statements_and_keeps_the_session_between_runs() {
    let Some(db) = connect(false).await else { return };
    let outcome = db
        .run(&statements(&[
            "SELECT 1 AS one, NULL::text AS nothing, ''::text AS empty",
            "SELECT * FROM tu_fixture.invoices WHERE false",
            "CREATE TEMP TABLE scratch (x int)",
            "INSERT INTO scratch VALUES (1), (2)",
            "SELECT x FROM scratch ORDER BY x",
        ]))
        .await
        .unwrap();
    assert!(outcome.failed.is_none());
    let [first, empty, create, insert, select] = outcome.results.as_slice() else {
        panic!("five results")
    };
    let names: Vec<_> = first.columns.iter().map(|c| (c.name.as_str(), c.type_name.as_str())).collect();
    assert_eq!(names, [("one", "int4"), ("nothing", "text"), ("empty", "text")]);
    assert_eq!(first.rows, [[Some("1".to_string()), None, Some(String::new())]]);
    assert_eq!((empty.columns.len(), empty.total_rows), (13, 0));
    assert!(create.columns.is_empty());
    assert_eq!(insert.rows_affected, 2);
    assert_eq!(select.rows, [[Some("1".to_string())], [Some("2".to_string())]]);
    assert!(!outcome.in_transaction);

    let again = db.run(&statements(&["SELECT count(*) FROM scratch", "BEGIN"])).await.unwrap();
    assert_eq!(again.results[0].rows, [[Some("2".to_string())]]);
    assert!(again.in_transaction);
    assert!(!db.run(&statements(&["ROLLBACK"])).await.unwrap().in_transaction);
}

#[tokio::test]
async fn stops_at_the_first_error_and_refuses_two_statements_in_one_piece() {
    let Some(db) = connect(false).await else { return };
    let outcome = db.run(&statements(&["SELECT 1", "SELECT nope", "SELECT 2"])).await.unwrap();
    let failed = outcome.failed.unwrap();
    assert_eq!(
        (outcome.results.len(), failed.statement, failed.error.code.as_deref()),
        (1, 1, Some("42703"))
    );

    let two = db.run(&statements(&["SELECT 1; SELECT 2"])).await.unwrap();
    assert_eq!(two.failed.unwrap().error.kind, ErrorKind::Invalid);
}

#[tokio::test]
async fn keeps_ten_thousand_rows_and_counts_the_rest() {
    let Some(db) = connect(false).await else { return };
    let outcome = db.run(&statements(&["SELECT g FROM generate_series(1, 12345) g"])).await.unwrap();
    let result = &outcome.results[0];
    assert_eq!((result.rows.len(), result.total_rows), (10_000, 12_345));
}

#[tokio::test]
async fn the_read_only_editor_cannot_be_talked_out_of_it() {
    let Some(db) = connect(true).await else { return };
    let attempts: [&[&str]; 4] = [
        &[
            "SET default_transaction_read_only = off",
            "UPDATE tu_fixture.customers SET name = name WHERE id = 1",
        ],
        &["COMMIT", "UPDATE tu_fixture.customers SET name = name WHERE id = 1"],
        &[
            "SELECT set_config('default_transaction_read_only', 'off', false)",
            "SELECT tu_fixture.touch()",
        ],
        &["SET SESSION CHARACTERISTICS AS TRANSACTION READ WRITE", "SELECT tu_fixture.touch()"],
    ];
    for attempt in attempts {
        let outcome = db.run(&statements(attempt)).await.unwrap();
        let failed = outcome.failed.unwrap_or_else(|| panic!("{attempt:?} wrote"));
        assert_eq!((failed.statement, failed.error.kind), (1, ErrorKind::ReadOnly), "{attempt:?}");
    }
    let switch = db.run(&statements(&["SET transaction_read_only = off"])).await.unwrap();
    assert_eq!(switch.failed.unwrap().error.code.as_deref(), Some("25001"));
}

#[tokio::test]
async fn cancels_a_running_statement() {
    let Some(db) = connect(false).await else { return };
    let sleep = statements(&["SELECT pg_sleep(30)"]);
    let (outcome, cancelled) = tokio::join!(db.run(&sleep), async {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        db.cancel().await
    });
    assert!(cancelled.unwrap());
    assert_eq!(outcome.unwrap().failed.unwrap().error.code.as_deref(), Some("57014"));
    assert!(!db.cancel().await.unwrap());
    assert!(db.run(&statements(&["SELECT 1"])).await.unwrap().failed.is_none());
}

#[tokio::test]
async fn explains_and_rolls_back_what_analyze_ran() {
    let Some(db) = connect(false).await else { return };
    let plan = db.explain("SELECT * FROM tu_fixture.invoices WHERE id = 5;", false).await.unwrap();
    assert!(plan.contains("Index Scan") && !plan.contains("Actual Total Time"));

    let measured = db
        .explain("UPDATE tu_fixture.customers SET name = 'changed' WHERE id = 2", true)
        .await
        .unwrap();
    assert!(measured.contains("Actual Total Time"));
    let filter = Filter::Equals {
        values: vec![ColumnValue {
            column: "id".into(),
            value: Some("2".into()),
        }],
    };
    let page = db.page(&request("customers", Some(filter), None, 0, 1)).await.unwrap();
    assert_eq!(page.rows[0][column(&page, "name")].as_deref(), Some("Customer 2"));

    assert_eq!(
        db.explain("SELECT 1; DROP TABLE tu_fixture.customers", false).await.unwrap_err().kind,
        ErrorKind::Invalid
    );
}

#[derive(Default)]
struct Collected {
    columns: Vec<String>,
    rows: Vec<Vec<Cell>>,
}

impl RowSink for Collected {
    fn columns(&mut self, columns: &[tu_core::PageColumn]) -> Result<(), tu_core::Error> {
        self.columns = columns.iter().map(|c| c.name.clone()).collect();
        Ok(())
    }

    fn row(&mut self, row: &[Cell]) -> Result<(), tu_core::Error> {
        self.rows.push(row.to_vec());
        Ok(())
    }
}

#[tokio::test]
async fn exports_every_row_and_never_writes() {
    let Some(db) = connect(false).await else { return };
    let mut rows = Collected::default();
    let filter = Filter::Equals {
        values: vec![ColumnValue {
            column: "customer_id".into(),
            value: Some("7".into()),
        }],
    };
    let source = ExportSource::Table {
        table: table("invoices"),
        sort: None,
        filter: Some(filter),
    };
    assert_eq!(db.export(&source, &mut rows).await.unwrap(), 2400.0);
    assert_eq!((rows.columns.len(), rows.rows.len()), (13, 2400));

    let mut long = Collected::default();
    let source = ExportSource::Statement {
        sql: "SELECT g FROM generate_series(1, 20000) g".into(),
    };
    assert_eq!(db.export(&source, &mut long).await.unwrap(), 20_000.0);
    assert_eq!(long.rows.last().unwrap()[0].as_deref(), Some("20000"));

    let write = ExportSource::Statement {
        sql: "UPDATE tu_fixture.customers SET name = name RETURNING id".into(),
    };
    assert_eq!(db.export(&write, &mut Collected::default()).await.unwrap_err().kind, ErrorKind::ReadOnly);
}

#[tokio::test]
async fn lists_sessions_and_stops_them_only_when_allowed() {
    let Some(db) = connect(false).await else { return };
    let sessions = db.activity().await.unwrap();
    assert!(sessions.iter().any(|a| a.application == "TableUltra"));
    assert!(!db.stop(999_999, false).await.unwrap());

    let Some(read_only) = connect(true).await else { return };
    assert_eq!(read_only.stop(999_999, true).await.unwrap_err().kind, ErrorKind::ReadOnly);
}
