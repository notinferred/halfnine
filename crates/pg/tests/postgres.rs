//! The adapter against a real Postgres loaded with `fixture.sql`. CI provides one through `HN_TEST_DATABASE_URL`;
//! without that variable these tests report that they were skipped and pass, so a laptop never needs a database.

use hn_core::{
    ByteSource, Cell, CellChange, Changes, ColumnValue, ConnectionSpec, Database, ErrorKind, ExportSource, Filter, Label, NewColumn, PageRequest,
    RelationKind, RowSink, RowUpdate, Sort, StructureChange, TableRef, TlsMode,
};
use hn_pg::{PgDatabase, Secrets};

async fn connect(read_only: bool) -> Option<PgDatabase> {
    let Ok(url) = std::env::var("HN_TEST_DATABASE_URL") else {
        eprintln!("skipped: HN_TEST_DATABASE_URL is not set");
        return None;
    };
    let parts = hn_core::parse_url(&url).expect("HN_TEST_DATABASE_URL is a postgres:// URL");
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
        save_password: false,
        agent_access: None,
        hidden_columns: Vec::new(),
        source: hn_core::Source::Saved,
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
        schema: "hn_fixture".into(),
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

fn column(page: &hn_core::Page, name: &str) -> usize {
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
    let node = schema.iter().find(|n| n.name == "hn_fixture").unwrap();
    let kind = |name: &str| node.relations.iter().find(|r| r.name == name).map(|r| r.kind);
    assert_eq!(kind("invoices"), Some(RelationKind::Table));
    let invoices = node.relations.iter().find(|r| r.name == "invoices").unwrap();
    assert_eq!(invoices.columns[..3], ["id", "customer_id", "amount"]);
    assert_eq!(invoices.column_types[..3], ["bigint", "bigint", "numeric(12,2)"]);
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
        sql: "true; DROP TABLE hn_fixture.customers".into(),
    };
    assert!(db.page(&request("invoices", Some(second), None, 0, 10)).await.is_err());
    assert!(db.structure(&table("customers")).await.is_ok());
}

#[tokio::test]
async fn read_only_connections_are_refused_by_the_server() {
    let Some(writable) = connect(false).await else { return };
    let touch = || {
        Some(Filter::Where {
            sql: "hn_fixture.touch()".into(),
        })
    };
    // Browsing only reads, on every connection, so a filter can't write even where the editor may.
    let error = writable.page(&request("customers", touch(), None, 0, 1)).await.unwrap_err();
    assert_eq!(error.code.as_deref(), Some("25006"));
    assert!(writable.run(&statements(&["SELECT hn_fixture.touch()"])).await.unwrap().failed.is_none());

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
        sql: "hn_fixture.touch()".into(),
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
            "SELECT * FROM hn_fixture.invoices WHERE false",
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
            "UPDATE hn_fixture.customers SET name = name WHERE id = 1",
        ],
        &["COMMIT", "UPDATE hn_fixture.customers SET name = name WHERE id = 1"],
        &[
            "SELECT set_config('default_transaction_read_only', 'off', false)",
            "SELECT hn_fixture.touch()",
        ],
        &["SET SESSION CHARACTERISTICS AS TRANSACTION READ WRITE", "SELECT hn_fixture.touch()"],
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
    let plan = db.explain("SELECT * FROM hn_fixture.invoices WHERE id = 5;", false).await.unwrap();
    assert!(plan.contains("Index Scan") && !plan.contains("Actual Total Time"));

    let measured = db
        .explain("UPDATE hn_fixture.customers SET name = 'changed' WHERE id = 2", true)
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
        db.explain("SELECT 1; DROP TABLE hn_fixture.customers", false).await.unwrap_err().kind,
        ErrorKind::Invalid
    );
}

#[derive(Default)]
struct Collected {
    columns: Vec<String>,
    rows: Vec<Vec<Cell>>,
}

impl RowSink for Collected {
    fn columns(&mut self, columns: &[hn_core::PageColumn]) -> Result<(), hn_core::Error> {
        self.columns = columns.iter().map(|c| c.name.clone()).collect();
        Ok(())
    }

    fn row(&mut self, row: &[Cell]) -> Result<(), hn_core::Error> {
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
        sql: "UPDATE hn_fixture.customers SET name = name RETURNING id".into(),
    };
    assert_eq!(db.export(&write, &mut Collected::default()).await.unwrap_err().kind, ErrorKind::ReadOnly);
}

#[tokio::test]
async fn lists_sessions_and_stops_them_only_when_allowed() {
    let Some(db) = connect(false).await else { return };
    let sessions = db.activity().await.unwrap();
    assert!(sessions.iter().any(|a| a.application == "Halfnine"));
    assert!(!db.stop(999_999, false).await.unwrap());

    let Some(read_only) = connect(true).await else { return };
    assert_eq!(read_only.stop(999_999, true).await.unwrap_err().kind, ErrorKind::ReadOnly);
}

fn value(column: &str, value: Option<&str>) -> ColumnValue {
    ColumnValue {
        column: column.into(),
        value: value.map(str::to_string),
    }
}

fn new_column(name: &str, type_name: &str, default: Option<&str>, key: bool) -> NewColumn {
    NewColumn {
        name: name.into(),
        type_name: type_name.into(),
        nullable: !key,
        default: default.map(str::to_string),
        key,
    }
}

fn structure(table: &TableRef, changes: Vec<StructureChange>) -> Changes {
    Changes::Structure {
        table: table.clone(),
        changes,
    }
}

/// A table of the test's own, so tests that write never touch what other tests read.
async fn scratch(db: &PgDatabase, name: &str) -> TableRef {
    let scratch = table(name);
    let _ = db.commit(&structure(&scratch, vec![StructureChange::Drop])).await;
    let columns = vec![
        new_column("id", "bigint", Some("generated always as identity"), true),
        new_column("body", "text", None, false),
        new_column("n", "integer", None, false),
    ];
    db.commit(&structure(&scratch, vec![StructureChange::CreateTable { columns }]))
        .await
        .unwrap();
    scratch
}

fn bodies(page: &hn_core::Page) -> Vec<Cell> {
    page.rows.iter().map(|r| r[column(page, "body")].clone()).collect()
}

#[tokio::test]
async fn commits_staged_rows_and_refuses_a_row_that_moved_on() {
    let Some(db) = connect(false).await else { return };
    let notes = scratch(&db, "scratch_rows").await;
    let rows = |deletes: Vec<Vec<ColumnValue>>, updates: Vec<RowUpdate>, inserts: Vec<Vec<ColumnValue>>| Changes::Rows {
        table: notes.clone(),
        deletes,
        updates,
        inserts,
        restores: vec![],
    };
    let page = || async { db.page(&request("scratch_rows", None, None, 0, 10)).await };

    let insert = rows(
        vec![],
        vec![],
        vec![
            vec![value("body", Some("first"))],
            vec![value("body", None)],
            vec![value("body", Some(""))],
        ],
    );
    assert_eq!(db.commit(&insert).await.unwrap().statements, 3);
    assert_eq!(bodies(&page().await.unwrap()), [Some("first".into()), None, Some(String::new())]);

    let update = |from: &str, to: &str| {
        let cells = vec![CellChange {
            column: "body".into(),
            from: Some(from.into()),
            to: Some(to.into()),
        }];
        rows(
            vec![],
            vec![RowUpdate {
                key: vec![value("id", Some("1"))],
                cells,
            }],
            vec![],
        )
    };
    db.commit(&update("first", "second")).await.unwrap();
    // Loaded as "first", but it says "second" now, as if another session had changed it.
    assert_eq!(db.commit(&update("first", "third")).await.unwrap_err().kind, ErrorKind::Conflict);
    assert_eq!(bodies(&page().await.unwrap())[0].as_deref(), Some("second"));

    let failing = rows(vec![vec![value("id", Some("2"))]], vec![], vec![vec![value("n", Some("many"))]]);
    let error = db.commit(&failing).await.unwrap_err();
    assert_eq!(error.code.as_deref(), Some("22P02"));
    assert!(error.message.starts_with("Adding new row 1 failed, so nothing was written"));
    assert_eq!(page().await.unwrap().rows.len(), 3, "the delete before it was rolled back");

    let twice = rows(vec![vec![value("id", Some("2"))], vec![value("id", Some("2"))]], vec![], vec![]);
    assert_eq!(db.commit(&twice).await.unwrap_err().kind, ErrorKind::Conflict);
    let generated = rows(vec![], vec![], vec![vec![value("id", Some("9"))]]);
    assert_eq!(db.commit(&generated).await.unwrap_err().kind, ErrorKind::Invalid);

    let Some(read_only) = connect(true).await else { return };
    assert_eq!(read_only.commit(&update("second", "fifth")).await.unwrap_err().kind, ErrorKind::ReadOnly);
    db.commit(&structure(&notes, vec![StructureChange::Drop])).await.unwrap();
}

#[tokio::test]
async fn changes_structure_in_one_transaction() {
    let Some(db) = connect(false).await else { return };
    let t = scratch(&db, "scratch_structure").await;
    let changes = vec![
        StructureChange::CreateIndex {
            columns: vec!["state".into()],
            unique: false,
        },
        StructureChange::RenameColumn {
            column: "body".into(),
            to: "state".into(),
        },
        StructureChange::SetType {
            column: "body".into(),
            to: "varchar(20)".into(),
        },
        StructureChange::SetNullable {
            column: "body".into(),
            nullable: false,
        },
        StructureChange::SetDefault {
            column: "body".into(),
            to: Some("'open'".into()),
        },
        StructureChange::AddColumn {
            column: new_column("made", "timestamptz", Some("now()"), false),
        },
    ];
    assert_eq!(db.commit(&structure(&t, changes)).await.unwrap().statements, 6);
    let s = db.structure(&t).await.unwrap();
    let state = s.columns.iter().find(|c| c.name == "state").unwrap();
    assert_eq!((state.type_name.as_str(), state.nullable), ("character varying(20)", false));
    assert!(state.default.as_deref().is_some_and(|d| d.starts_with("'open'")));
    assert!(s.columns[0].generated && !state.generated);
    assert!(s.indexes.iter().any(|i| i.definition.ends_with("(state)")));

    let broken = vec![
        StructureChange::DropColumn { column: "made".into() },
        StructureChange::SetType {
            column: "state".into(),
            to: "no_such_type".into(),
        },
    ];
    let error = db.commit(&structure(&t, broken)).await.unwrap_err();
    assert!(error.message.starts_with("Changing the type of state failed"));
    assert!(
        db.structure(&t).await.unwrap().columns.iter().any(|c| c.name == "made"),
        "the drop before it was rolled back"
    );

    let smuggled = vec![StructureChange::SetDefault {
        column: "state".into(),
        to: Some("'x'; DROP TABLE hn_fixture.customers".into()),
    }];
    assert!(db.commit(&structure(&t, smuggled)).await.is_err());
    assert!(db.structure(&table("customers")).await.is_ok());

    db.commit(&structure(&t, vec![StructureChange::Truncate])).await.unwrap();
    db.commit(&structure(&t, vec![StructureChange::Drop])).await.unwrap();
    assert_eq!(db.structure(&t).await.unwrap_err().kind, ErrorKind::NotFound);
}

#[tokio::test]
async fn offers_foreign_key_values_by_value_or_label() {
    let Some(db) = connect(false).await else { return };
    let all = db.choices(&table("customers"), "id", "").await.unwrap();
    assert_eq!(all.len(), 50);
    assert_eq!((all[0].value.as_str(), all[0].label.as_deref()), ("1", Some("Customer 1")));
    // Customer 4 and Customer 40 to 49.
    assert_eq!(db.choices(&table("customers"), "id", "customer 4").await.unwrap().len(), 11);
}

/// A file arriving in pieces, split mid-row, as a real one read in chunks would.
struct Pieces(Vec<Vec<u8>>);

impl Pieces {
    fn of(text: &str) -> Self {
        let (a, b) = text.as_bytes().split_at(text.len() / 2);
        Self(vec![a.to_vec(), b.to_vec()])
    }
}

impl ByteSource for Pieces {
    fn next(&mut self) -> Result<Option<Vec<u8>>, hn_core::Error> {
        Ok(if self.0.is_empty() { None } else { Some(self.0.remove(0)) })
    }
}

#[tokio::test]
async fn imports_csv_in_one_transaction_keeping_null_apart_from_empty() {
    let Some(db) = connect(false).await else { return };
    let t = scratch(&db, "scratch_import").await;
    let page = || async { db.page(&request("scratch_import", None, None, 0, 10)).await };
    let columns = vec![None, Some("n".to_string()), Some("body".to_string())];
    let file = "skipped,n,body\nx,1,first\nx,2,\nx,3,\"\"\n";
    assert_eq!(db.import(&t, &columns, b',', &mut Pieces::of(file)).await.unwrap(), 3.0);
    assert_eq!(bodies(&page().await.unwrap()), [Some("first".into()), None, Some(String::new())]);

    let error = db
        .import(&t, &[Some("n".into())], b',', &mut Pieces::of("n\n4\nfive\n"))
        .await
        .unwrap_err();
    assert_eq!(error.code.as_deref(), Some("22P02"));
    assert_eq!(page().await.unwrap().rows.len(), 3, "the row before it was rolled back");

    let Some(read_only) = connect(true).await else { return };
    let refused = read_only.import(&t, &[Some("n".into())], b',', &mut Pieces::of("n\n4\n")).await;
    assert_eq!(refused.unwrap_err().kind, ErrorKind::ReadOnly);
    db.commit(&structure(&t, vec![StructureChange::Drop])).await.unwrap();
}

#[tokio::test]
async fn shows_every_keyword_quoted_as_postgres_requires() {
    let Some(db) = connect(false).await else { return };
    let listed = db
        .run(&statements(&["SELECT word FROM pg_get_keywords() WHERE catcode <> 'U' ORDER BY 1"]))
        .await
        .unwrap();
    let words: Vec<String> = listed.results[0].rows.iter().filter_map(|r| r[0].clone()).collect();
    assert!(words.len() > 100);
    let renames = words
        .iter()
        .map(|w| StructureChange::RenameColumn {
            column: "a".into(),
            to: w.clone(),
        })
        .collect();
    let shown = db.review(&structure(&table("customers"), renames)).await.unwrap();
    for (word, line) in words.iter().zip(&shown) {
        assert!(line.ends_with(&format!("TO \"{word}\";")), "{word} is shown bare");
    }
}

#[tokio::test]
async fn picks_rows_by_their_keys() {
    let Some(db) = connect(false).await else { return };
    let key = |invoice: &str| vec![value("invoice_id", Some(invoice)), value("line", Some("1"))];
    let keys = Filter::Keys {
        keys: vec![key("3"), key("7"), key("100000")],
    };
    let page = db.page(&request("line_items", Some(keys), None, 0, 10)).await.unwrap();
    let invoices: Vec<Cell> = page.rows.iter().map(|r| r[column(&page, "invoice_id")].clone()).collect();
    assert_eq!(invoices, [Some("3".to_string()), Some("7".to_string())]);
}

#[tokio::test]
async fn an_agent_query_only_ever_reads() {
    let Some(db) = connect(false).await else { return };
    for refused in [
        "UPDATE hn_fixture.customers SET name = name",
        "COPY hn_fixture.customers TO STDOUT",
        "SET search_path = x",
    ] {
        assert_eq!(db.read(refused, 10, &[]).await.unwrap_err().kind, ErrorKind::Invalid, "{refused}");
    }
    let writes = db.read("SELECT hn_fixture.touch()", 10, &[]).await.unwrap_err();
    assert_eq!(writes.code.as_deref(), Some("25006"));
    assert!(db.read("SELECT 1; SELECT 2", 10, &[]).await.is_err());
    let hidden = "WITH gone AS (DELETE FROM hn_fixture.customers RETURNING 1) SELECT count(*) FROM gone";
    assert!(db.read(hidden, 10, &[]).await.is_err());
    assert_eq!(
        db.read("SELECT count(*) FROM hn_fixture.customers", 1, &[]).await.unwrap().rows,
        [[Some("50".to_string())]]
    );
    let shown = db.read("show transaction_read_only", 1, &[]).await.unwrap();
    assert_eq!(shown.rows, [[Some("on".to_string())]]);
}

#[tokio::test]
async fn an_agent_query_cannot_switch_read_only_off_for_later_ones() {
    let Some(db) = connect(true).await else { return };
    for _ in 0..3 {
        db.read("SELECT set_config('default_transaction_read_only', 'off', false)", 1, &[])
            .await
            .unwrap();
    }
    for _ in 0..3 {
        let setting = db.read("SHOW default_transaction_read_only", 1, &[]).await.unwrap();
        assert_eq!(setting.rows, [[Some("on".to_string())]]);
    }
}

#[tokio::test]
async fn an_agent_query_stops_at_its_limit() {
    let Some(db) = connect(false).await else { return };
    let started = std::time::Instant::now();
    let endless = db
        .read("SELECT a.id, b.id FROM hn_fixture.invoices a, hn_fixture.invoices b", 10, &[])
        .await
        .unwrap();
    assert_eq!((endless.rows.len(), endless.total_rows), (10, 11));
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    let none = db.read("SELECT 1 WHERE false", 10, &[]).await.unwrap();
    assert_eq!((none.rows.len(), none.total_rows, none.columns.len()), (0, 0, 1));
    let slow = db.read("SELECT pg_sleep(20)", 1, &[]).await.unwrap_err();
    assert_eq!((slow.kind, slow.code.as_deref()), (ErrorKind::Timeout, Some("57014")));
}

#[tokio::test]
async fn an_agent_change_is_all_or_nothing_on_its_own_connection() {
    let Some(db) = connect(false).await else { return };
    let scratch = scratch(&db, "agent_scratch").await;
    let name = format!("hn_fixture.{}", scratch.name);
    let done = db
        .execute(
            &statements(&[
                &format!("INSERT INTO {name} (body) VALUES ('a'), ('b') RETURNING body"),
                &format!("UPDATE {name} SET n = 1 WHERE body = 'a'"),
                "SET search_path = nowhere",
            ]),
            &[],
        )
        .await
        .unwrap();
    assert_eq!((done[0].rows.len(), done[1].rows_affected), (2, 1));
    let search_path = db.read("SELECT current_setting('search_path')", 1, &[]).await.unwrap();
    assert_ne!(search_path.rows, [[Some("nowhere".to_string())]]);

    let failed = db
        .execute(
            &statements(&[
                &format!("INSERT INTO {name} (body) VALUES ('c')"),
                "INSERT INTO hn_fixture.nowhere VALUES (1)",
            ]),
            &[],
        )
        .await
        .unwrap_err();
    assert!(failed.message.starts_with("Statement 2 failed, so nothing was written"));
    let page = db.page(&request(&scratch.name, None, None, 0, 10)).await.unwrap();
    assert_eq!(bodies(&page), [Some("a".to_string()), Some("b".to_string())]);

    for refused in ["COMMIT", "  /* why */ begin", "SAVEPOINT x", "COPY hn_fixture.customers TO STDOUT"] {
        let error = db.execute(&statements(&[refused]), &[]).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::Invalid, "{refused}");
    }

    let Some(read_only) = connect(true).await else { return };
    let refused = read_only
        .execute(&statements(&[&format!("INSERT INTO {name} (body) VALUES ('d')")]), &[])
        .await
        .unwrap_err();
    assert_eq!(refused.code.as_deref(), Some("25006"));
}

#[tokio::test]
async fn lists_every_foreign_key_once() {
    let Some(db) = connect(false).await else { return };
    let references = db.references().await.unwrap();
    let invoices = references.iter().find(|r| r.source == table("invoices")).unwrap();
    assert_eq!(
        (invoices.source_columns.as_slice(), &invoices.target, invoices.target_columns.as_slice()),
        (&["customer_id".to_string()][..], &table("customers"), &["id".to_string()][..])
    );
    assert!(
        references
            .iter()
            .any(|r| r.source == table("line_items") && r.target == table("invoices"))
    );
    assert!(references.iter().all(|r| !r.source.schema.starts_with("pg_")));
}

fn email() -> Vec<hn_core::ColumnRef> {
    vec![hn_core::ColumnRef {
        schema: "hn_fixture".into(),
        table: "customers".into(),
        column: "email".into(),
    }]
}

#[tokio::test]
async fn masks_a_hidden_column_wherever_a_query_returns_it() {
    let Some(db) = connect(false).await else { return };
    for query in [
        "SELECT * FROM hn_fixture.customers WHERE id = 7",
        "SELECT c.email AS contact, c.name FROM hn_fixture.customers c WHERE c.id = 7",
        "SELECT i.id, c.email FROM hn_fixture.invoices i JOIN hn_fixture.customers c ON c.id = i.customer_id WHERE i.id = 7",
    ] {
        let result = db.read(query, 5, &email()).await.unwrap();
        let text = format!("{:?}", result.rows);
        assert!(text.contains("[hidden]") && !text.contains("@example.com"), "{query}: {text}");
    }
    let plain = db.read("SELECT name FROM hn_fixture.customers WHERE id = 7", 5, &email()).await.unwrap();
    assert_eq!(plain.rows, [[Some("Customer 7".to_string())]]);
}

#[tokio::test]
async fn refuses_a_query_that_computes_with_a_hidden_column() {
    let Some(db) = connect(false).await else { return };
    for query in [
        "SELECT lower(email) FROM hn_fixture.customers",
        "SELECT id FROM hn_fixture.customers WHERE email LIKE 'customer1%'",
        "SELECT to_jsonb(c) FROM hn_fixture.customers c",
        "SELECT count(*) FROM hn_fixture.invoices i JOIN hn_fixture.customers c ON c.email = i.note",
    ] {
        let error = db.read(query, 5, &email()).await.unwrap_err();
        assert!(
            error.message.contains("hn_fixture.customers.email is hidden from agents"),
            "{query}: {}",
            error.message
        );
    }
    let error = db
        .execute(&statements(&["UPDATE hn_fixture.customers SET name = email WHERE id = 1"]), &email())
        .await
        .unwrap_err();
    assert!(error.message.contains("is hidden from agents"), "{}", error.message);
    db.execute(
        &statements(&["UPDATE hn_fixture.customers SET name = name WHERE id = 1", "SELECT 1"]),
        &email(),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn returns_the_keys_it_adds_and_puts_a_deleted_row_back_as_it_was() {
    let Some(db) = connect(false).await else { return };
    let scratch = scratch(&db, "restored").await;
    let insert = Changes::Rows {
        table: scratch.clone(),
        deletes: vec![],
        updates: vec![],
        inserts: vec![vec![value("body", Some("first"))], vec![value("body", Some("second"))]],
        restores: vec![],
    };
    let added = db.commit(&insert).await.unwrap().inserted;
    assert_eq!(added.len(), 2);
    let first = added[0].clone();
    let delete = Changes::Rows {
        table: scratch.clone(),
        deletes: vec![first.clone()],
        updates: vec![],
        inserts: vec![],
        restores: vec![],
    };
    db.commit(&delete).await.unwrap();
    let mut back = first.clone();
    back.push(value("body", Some("first")));
    let restore = Changes::Rows {
        table: scratch.clone(),
        deletes: vec![],
        updates: vec![],
        inserts: vec![],
        restores: vec![back],
    };
    assert_eq!(db.commit(&restore).await.unwrap().inserted, [first]);
    let page = db.page(&request(&scratch.name, None, None, 0, 10)).await.unwrap();
    assert_eq!(bodies(&page), [Some("first".to_string()), Some("second".to_string())]);
}

/// A backup kept in memory: its definition, and its rows as the CSV a restore reads back.
#[derive(Default)]
struct Kept {
    definition: Option<hn_core::TableDefinition>,
    csv: String,
    finished: bool,
    full_disk: bool,
}

impl RowSink for Kept {
    fn columns(&mut self, columns: &[hn_core::PageColumn]) -> Result<(), hn_core::Error> {
        self.csv.push_str(&hn_core::csv_line(columns.iter().map(|c| Some(c.name.as_str()))));
        Ok(())
    }

    fn row(&mut self, row: &[Cell]) -> Result<(), hn_core::Error> {
        if self.full_disk {
            return Err(hn_core::Error::new(ErrorKind::Storage, "The disk is full."));
        }
        self.csv.push_str(&hn_core::csv_line(row.iter().map(Option::as_deref)));
        Ok(())
    }
}

impl hn_core::TableSink for Kept {
    fn definition(&mut self, definition: &hn_core::TableDefinition) -> Result<(), hn_core::Error> {
        self.definition = Some(definition.clone());
        Ok(())
    }

    fn finish(&mut self) -> Result<(), hn_core::Error> {
        self.finished = true;
        Ok(())
    }
}

/// A table with most of what a backup must keep: identity, serial, a collation, a default, stored and virtual generated
/// columns, a foreign key, check and unique constraints, an expression index with a WHERE, comments and a trigger.
/// Its rows hold NULL apart from empty text, quotes, a newline, `\.` and a 70 KB value.
async fn keepsake(db: &PgDatabase, name: &str) -> TableRef {
    let t = format!("hn_fixture.{name}");
    let outcome = db
        .run(&[
            format!("DROP TABLE IF EXISTS {t}"),
            format!("DROP TABLE IF EXISTS {t}_codes"),
            format!("CREATE TABLE {t}_codes (code text PRIMARY KEY)"),
            format!("INSERT INTO {t}_codes VALUES ('a'), ('b')"),
            format!(
                "CREATE TABLE {t} (id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY, serial_no serial UNIQUE, \
                 body text COLLATE \"C\" NOT NULL DEFAULT '', note text, size integer GENERATED ALWAYS AS (length(body)) STORED, \
                 doubled integer GENERATED ALWAYS AS (length(body) * 2) VIRTUAL, code text REFERENCES {t}_codes (code), \
                 CONSTRAINT {name}_note_short CHECK (length(note) < 100000))"
            ),
            format!("CREATE INDEX {name}_lower ON {t} (lower(note)) WHERE note IS NOT NULL"),
            format!("COMMENT ON TABLE {t} IS 'Kept'"),
            format!("COMMENT ON COLUMN {t}.note IS 'it''s a note'"),
            format!(
                "INSERT INTO {t} (body, note, code) VALUES ('first', NULL, 'a'), ('', '', 'b'), ('say \"hi\", twice', E'line\\nbreak', NULL), \
                 (repeat('x', 70000), '\\.', 'a')"
            ),
            format!(
                "CREATE OR REPLACE FUNCTION {t}_stamp() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN NEW.note := coalesce(NEW.note, 'stamped'); \
                 RETURN NEW; END $$"
            ),
            format!("CREATE TRIGGER {name}_stamp BEFORE INSERT ON {t} FOR EACH ROW EXECUTE FUNCTION {t}_stamp()"),
        ])
        .await
        .unwrap();
    assert!(outcome.failed.is_none(), "{:?}", outcome.failed);
    table(name)
}

async fn every_row(db: &PgDatabase, t: &TableRef) -> Vec<Vec<Cell>> {
    let mut rows = Collected::default();
    let sort = Sort {
        column: "id".into(),
        descending: false,
    };
    let source = ExportSource::Table {
        table: t.clone(),
        sort: Some(sort),
        filter: None,
    };
    db.export(&source, &mut rows).await.unwrap();
    rows.rows
}

fn shape(s: &hn_core::Structure) -> String {
    format!("{:?}", (&s.columns, &s.primary_key, &s.indexes, &s.constraints, &s.foreign_keys))
}

#[tokio::test]
async fn puts_a_dropped_table_back_as_it_was() {
    let Some(db) = connect(false).await else { return };
    let t = keepsake(&db, "scratch_backup_drop").await;
    let (rows, before) = (every_row(&db, &t).await, db.structure(&t).await.unwrap());
    assert!(db.backup_size(&t, &hn_core::RestoreInto::Recreate).await.unwrap() > 0.0);

    let mut kept = Kept::default();
    db.commit_with_backup(&structure(&t, vec![StructureChange::Drop]), &t, &mut kept)
        .await
        .unwrap();
    assert!(kept.finished);
    assert_eq!(db.structure(&t).await.unwrap_err().kind, ErrorKind::NotFound);
    let definition = kept.definition.unwrap();
    assert!(definition.not_kept.is_empty());

    let review = db.review_restore(&definition, &hn_core::RestoreInto::Recreate).await.unwrap();
    assert!(review[0].starts_with("CREATE SEQUENCE hn_fixture.scratch_backup_drop_serial_no_seq AS integer"));
    let restored = db
        .restore(&definition, &hn_core::RestoreInto::Recreate, &mut Pieces::of(&kept.csv))
        .await
        .unwrap();
    assert_eq!(restored, 4.0);
    assert_eq!(every_row(&db, &t).await, rows);
    assert_eq!(shape(&db.structure(&t).await.unwrap()), shape(&before));

    let next = db
        .run(&statements(&[
            "INSERT INTO hn_fixture.scratch_backup_drop (body) VALUES ('next') RETURNING id, serial_no, note",
        ]))
        .await
        .unwrap();
    let next = &next.results[0].rows[0];
    assert_eq!(
        next,
        &[Some("5".to_string()), Some("5".to_string()), Some("stamped".to_string())],
        "{next:?}"
    );

    let again = db.restore(&definition, &hn_core::RestoreInto::Recreate, &mut Pieces::of(&kept.csv)).await;
    assert_eq!(again.unwrap_err().kind, ErrorKind::Invalid);
}

#[tokio::test]
async fn refills_an_emptied_table_without_firing_its_triggers() {
    let Some(db) = connect(false).await else { return };
    let t = keepsake(&db, "scratch_backup_truncate").await;
    let rows = every_row(&db, &t).await;

    let mut kept = Kept::default();
    db.commit_with_backup(&structure(&t, vec![StructureChange::Truncate]), &t, &mut kept)
        .await
        .unwrap();
    assert!(every_row(&db, &t).await.is_empty());
    let definition = kept.definition.unwrap();
    let refill = hn_core::RestoreInto::Refill;
    assert_eq!(db.restore(&definition, &refill, &mut Pieces::of(&kept.csv)).await.unwrap(), 4.0);
    assert_eq!(every_row(&db, &t).await, rows, "the NULL note wasn't stamped");

    let full = db.restore(&definition, &refill, &mut Pieces::of(&kept.csv)).await;
    assert_eq!(full.unwrap_err().kind, ErrorKind::Invalid);
}

#[tokio::test]
async fn puts_dropped_columns_back_with_their_index_check_and_foreign_key() {
    let Some(db) = connect(false).await else { return };
    let t = keepsake(&db, "scratch_backup_column").await;
    let (rows, before) = (every_row(&db, &t).await, db.structure(&t).await.unwrap());

    let mut kept = Kept::default();
    let drops = ["note", "code"].map(|c| StructureChange::DropColumn { column: c.into() });
    db.commit_with_backup(&structure(&t, drops.to_vec()), &t, &mut kept).await.unwrap();
    let definition = kept.definition.unwrap();
    let into = hn_core::RestoreInto::Columns {
        names: vec!["note".into(), "code".into()],
    };
    assert!(db.backup_size(&t, &into).await.is_ok());
    let key = hn_core::RestoreInto::Columns { names: vec!["id".into()] };
    assert_eq!(db.backup_size(&t, &key).await.unwrap_err().kind, ErrorKind::Invalid);
    assert_eq!(db.restore(&definition, &into, &mut Pieces::of(&kept.csv)).await.unwrap(), 4.0);

    let after = db.structure(&t).await.unwrap();
    let now = every_row(&db, &t).await;
    for name in ["note", "code"] {
        let column = |s: &hn_core::Structure| format!("{:?}", s.columns.iter().find(|c| c.name == name));
        assert_eq!(column(&after), column(&before));
        let at = |s: &hn_core::Structure| s.columns.iter().position(|c| c.name == name).unwrap();
        let values = |rows: &[Vec<Cell>], at: usize| rows.iter().map(|r| r[at].clone()).collect::<Vec<_>>();
        assert_eq!(values(&now, at(&after)), values(&rows, at(&before)), "{name}");
    }
    assert_eq!(format!("{:?}", after.indexes), format!("{:?}", before.indexes));
    assert_eq!(format!("{:?}", after.constraints), format!("{:?}", before.constraints));
    assert_eq!(format!("{:?}", after.foreign_keys), format!("{:?}", before.foreign_keys));
}

#[tokio::test]
async fn a_backup_that_fails_leaves_the_table_untouched() {
    let Some(db) = connect(false).await else { return };
    let t = keepsake(&db, "scratch_backup_full").await;
    let mut kept = Kept {
        full_disk: true,
        ..Kept::default()
    };
    let error = db
        .commit_with_backup(&structure(&t, vec![StructureChange::Drop]), &t, &mut kept)
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::NoBackup);
    assert!(error.message.starts_with("The backup failed, so nothing changed"), "{}", error.message);
    assert_eq!(every_row(&db, &t).await.len(), 4);

    let partitioned = db.backup_size(&table("events"), &hn_core::RestoreInto::Recreate).await.unwrap_err();
    assert_eq!(partitioned.kind, ErrorKind::Invalid);
    assert!(partitioned.message.contains("partitioned"), "{}", partitioned.message);
}
