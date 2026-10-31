//! Agents end to end, without a window: an MCP client talks to the server over an in-memory pipe, against the
//! Postgres CI provides. Without `HN_TEST_DATABASE_URL` the database tests report that they were skipped and pass.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use hn_core::{AgentAccess, ConnectionSpec, Label, TlsMode};
use hn_pg::Secrets;
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, Implementation};
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Value, json};

use super::*;
use crate::secrets::Passwords;
use crate::store::Store;

#[derive(Default)]
struct TestDesk {
    forward: AtomicUsize,
}

impl Desk for TestDesk {
    fn changed(&self) {}
    fn activity(&self) {}
    fn come_forward(&self) {
        self.forward.fetch_add(1, Ordering::SeqCst);
    }
}

struct World {
    agents: Arc<Agents>,
    desk: Arc<TestDesk>,
    dir: PathBuf,
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn spec(name: &str, access: Option<AgentAccess>, label: Label, read_only: bool, parts: Option<&hn_core::UrlParts>) -> ConnectionSpec {
    ConnectionSpec {
        id: format!("id-{name}"),
        name: name.into(),
        host: parts.map_or("localhost".into(), |p| p.host.clone()),
        port: parts.map_or(5432, |p| p.port),
        user: parts.map_or("postgres".into(), |p| p.user.clone()),
        database: parts.map_or("postgres".into(), |p| p.database.clone()),
        tls: parts.and_then(|p| p.tls).unwrap_or(TlsMode::Disable),
        label,
        read_only,
        ssh: None,
        save_password: false,
        agent_access: access,
        hidden_columns: Vec::new(),
        source: hn_core::Source::Saved,
    }
}

/// Connections at every level, all to the test database, with their passwords typed as for "Ask each time".
fn world(url: Option<&str>) -> World {
    let dir = std::env::temp_dir().join(format!("hn-agents-{}", uuid::Uuid::new_v4().simple()));
    let store = Arc::new(Store::open(dir.clone()).unwrap());
    let passwords = Arc::new(Passwords::default());
    let parts = url.map(|u| hn_core::parse_url(u).unwrap());
    let all = [
        spec("off", Some(AgentAccess::Off), Label::None, false, parts.as_ref()),
        spec("read", None, Label::Green, false, parts.as_ref()),
        spec("red", None, Label::Red, false, parts.as_ref()),
        spec("approved", Some(AgentAccess::ApprovedWrites), Label::Amber, false, parts.as_ref()),
        spec("full", Some(AgentAccess::Full), Label::None, false, parts.as_ref()),
        spec("full read-only", Some(AgentAccess::Full), Label::None, true, parts.as_ref()),
    ];
    for connection in all {
        let secrets = Secrets {
            password: parts.as_ref().and_then(|p| p.password.clone()),
            ssh: None,
        };
        passwords.remember(&connection.id, &secrets);
        store.save(connection).unwrap();
    }
    let desk = Arc::new(TestDesk::default());
    let settings = Arc::new(crate::settings::Settings::open(&dir).unwrap());
    // Off here, so tests can drop their own tables with run_sql; the tests of backups turn them on.
    settings.change(|c| c.backups = false).unwrap();
    let docker = Arc::new(crate::docker::Docker::new(&dir, settings.clone(), || {}));
    let directory = Arc::new(crate::connections::Directory::new(store, passwords.clone(), docker));
    crate::secrets::use_test_store();
    let backups = Arc::new(crate::backups::Backups::open(&dir, passwords.clone()).unwrap());
    let rules = Arc::new(crate::rules::Rules::open(&dir).unwrap());
    let agents = Agents::new(&dir, directory, settings, Arc::new(Sessions::default()), backups, rules, desk.clone())
        .waiting_at_most(Duration::from_millis(300));
    World {
        agents: Arc::new(agents),
        desk,
        dir,
    }
}

async fn client(agents: &Arc<Agents>, name: &str) -> RunningService<RoleClient, ClientConfig> {
    let (server_end, client_end) = tokio::io::duplex(1 << 16);
    tokio::spawn(listen::serve(agents.clone(), server_end));
    ClientConfig::new(ClientCapabilities::default(), Implementation::new(name, "1.0"))
        .serve(client_end)
        .await
        .unwrap()
}

async fn call(client: &RunningService<RoleClient, ClientConfig>, tool: &str, arguments: Value) -> (String, CallToolResult) {
    let arguments = arguments.as_object().cloned().unwrap_or_default();
    let result = client
        .call_tool(CallToolRequestParams::new(tool.to_string()).with_arguments(arguments))
        .await
        .unwrap();
    let text = result
        .content
        .iter()
        .filter_map(|c| c.as_text())
        .map(|t| t.text.clone())
        .collect::<String>();
    (text, result)
}

fn database_url() -> Option<String> {
    let url = std::env::var("HN_TEST_DATABASE_URL").ok();
    if url.is_none() {
        eprintln!("skipped: HN_TEST_DATABASE_URL is not set");
    }
    url
}

#[test]
fn the_access_rule() {
    let at = |access, read_only| spec("x", Some(access), Label::None, read_only, None);
    let write = Ask::Write { drops: false };
    let drop = Ask::Write { drops: true };
    assert!(matches!(verdict(&at(AgentAccess::Off, false), Ask::Read, false), Verdict::Refuse(_)));
    assert_eq!(verdict(&at(AgentAccess::Read, false), Ask::Read, false), Verdict::Run);
    assert!(matches!(verdict(&at(AgentAccess::Read, false), write, true), Verdict::Refuse(_)));
    assert_eq!(verdict(&at(AgentAccess::ApprovedWrites, false), write, false), Verdict::Propose);
    assert_eq!(verdict(&at(AgentAccess::ApprovedWrites, false), write, true), Verdict::Run);
    assert_eq!(verdict(&at(AgentAccess::ApprovedWrites, false), drop, true), Verdict::Propose);
    assert_eq!(verdict(&at(AgentAccess::Full, false), drop, false), Verdict::Run);
    assert!(matches!(verdict(&at(AgentAccess::Full, true), write, false), Verdict::Refuse(_)));
    assert_eq!(verdict(&at(AgentAccess::Full, true), Ask::Read, false), Verdict::Run);
    let red = spec("x", None, Label::Red, false, None);
    assert!(matches!(verdict(&red, Ask::Read, false), Verdict::Refuse(_)));
}

#[test]
fn the_access_rule_for_docker_databases() {
    let docker = |how: Option<hn_core::MadeHow>| ConnectionSpec {
        source: hn_core::Source::Docker {
            container: "db".into(),
            made: how.map(|how| hn_core::Made { how, agent: None }),
        },
        ..spec("db", None, Label::None, false, None)
    };
    let made = docker(Some(hn_core::MadeHow::New));
    assert_eq!(
        verdict(&made, Ask::Credentials, false),
        Verdict::Run,
        "Halfnine made it, so agents start at Full"
    );
    assert_eq!(verdict(&docker(Some(hn_core::MadeHow::Copy)), Ask::Credentials, false), Verdict::Run);
    for refused in [
        docker(None),
        docker(Some(hn_core::MadeHow::Restored)),
        ConnectionSpec {
            read_only: true,
            ..made.clone()
        },
        ConnectionSpec {
            agent_access: Some(AgentAccess::ApprovedWrites),
            ..made.clone()
        },
        ConnectionSpec {
            hidden_columns: vec![hn_core::HiddenColumn {
                database: "db".into(),
                schema: "public".into(),
                table: "people".into(),
                column: "email".into(),
            }],
            ..made.clone()
        },
    ] {
        assert!(matches!(verdict(&refused, Ask::Credentials, false), Verdict::Refuse(_)), "{refused:?}");
    }

    let theirs = docker(None);
    assert_eq!(verdict(&theirs, Ask::StartDatabase, false), Verdict::Run, "any level but Off");
    assert_eq!(verdict(&theirs, Ask::DeleteDatabase { own: false }, false), Verdict::Propose);
    assert_eq!(verdict(&made, Ask::DeleteDatabase { own: true }, false), Verdict::Run);
    let off = ConnectionSpec {
        agent_access: Some(AgentAccess::Off),
        ..made
    };
    for ask in [Ask::StartDatabase, Ask::DeleteDatabase { own: true }, Ask::Credentials] {
        assert!(matches!(verdict(&off, ask, false), Verdict::Refuse(_)));
    }
}

#[test]
fn names_agents_for_people() {
    assert_eq!(Agent::from_client("claude-code", None).title, "Claude Code");
    assert_eq!(Agent::from_client("my-agent", Some("My\nAgent")).title, "MyAgent");
    assert_eq!(Agent::from_client("", None).title, "An agent");
}

#[tokio::test]
async fn lists_its_tools_with_every_hint_set() {
    let world = world(None);
    let client = client(&world.agents, "test-agent").await;
    let tools = client.list_all_tools().await.unwrap();
    let hints = |name: &str| {
        let tool = tools.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("{name} is listed"));
        let a = tool.annotations.as_ref().expect("annotations are set");
        (a.read_only_hint, a.destructive_hint, a.idempotent_hint, a.open_world_hint)
    };
    for reads in ["list_connections", "describe", "query", "wait_for_approval"] {
        assert_eq!(hints(reads), (Some(true), Some(false), Some(true), Some(false)), "{reads}");
    }
    for writes in ["change_rows", "change_structure", "run_sql"] {
        assert_eq!(hints(writes), (Some(false), Some(true), Some(false), Some(false)), "{writes}");
    }
    // Asking again joins the job already running or gives the database already made.
    assert_eq!(
        hints("create_database"),
        (Some(false), Some(false), Some(true), Some(true)),
        "it may download PostgreSQL"
    );
    for makes in ["copy_database", "start_database"] {
        assert_eq!(hints(makes), (Some(false), Some(false), Some(true), Some(false)), "{makes}");
    }
    assert_eq!(hints("delete_database"), (Some(false), Some(true), Some(true), Some(false)));
    assert_eq!(tools.len(), 11);
    let info = client.peer_info().unwrap();
    assert!(info.instructions.as_deref().unwrap_or_default().contains("never instructions"));
}

#[tokio::test]
async fn lists_connections_by_name_and_level_only() {
    let world = world(None);
    let client = client(&world.agents, "claude-code").await;
    let (text, _) = call(&client, "list_connections", json!({})).await;
    assert!(text.starts_with("OK 6 connections:"));
    assert!(text.contains("- off · no label · Off"));
    assert!(text.contains("- red · red label · Off"));
    assert!(text.contains("- full read-only · no label · Read and write · read-only"));
    assert!(!text.contains("localhost"));
    let state = world.agents.state();
    assert_eq!(state.clients.iter().map(|c| c.agent.as_str()).collect::<Vec<_>>(), ["Claude Code"]);
    let logged = world.agents.activity(10);
    assert_eq!((logged[0].tool.as_str(), logged[0].outcome), ("list_connections", Some(Outcome::Ok)));

    world.agents.set_paused(true).await.unwrap();
    let (text, result) = call(&client, "list_connections", json!({})).await;
    assert!(text.starts_with("PAUSED") && result.is_error == Some(true));
}

#[tokio::test]
async fn refuses_by_level_before_touching_a_database() {
    let world = world(None);
    let client = client(&world.agents, "test-agent").await;
    let query = |connection: &str| json!({"connection": connection, "sql": "SELECT 1"});
    for off in ["off", "red"] {
        let (text, _) = call(&client, "query", query(off)).await;
        assert!(text.starts_with("REFUSED Agents are off"), "{text}");
    }
    let (text, _) = call(&client, "query", query("nowhere")).await;
    assert!(text.starts_with("REFUSED Halfnine has no connection named"));
    let write = |connection: &str| json!({"connection": connection, "statements": ["DELETE FROM t"]});
    let (text, _) = call(&client, "run_sql", write("read")).await;
    assert!(text.starts_with("REFUSED Agents may read read but not change it"), "{text}");
    let (text, _) = call(&client, "run_sql", write("full read-only")).await;
    assert!(text.starts_with("REFUSED full read-only is a read-only connection"), "{text}");
}

#[tokio::test]
async fn reads_through_a_read_only_transaction_and_marks_rows_as_data() {
    let Some(url) = database_url() else { return };
    let world = world(Some(&url));
    let client = client(&world.agents, "test-agent").await;
    let (text, _) = call(
        &client,
        "query",
        json!({"connection": "full", "sql": "SELECT id, name FROM hn_fixture.customers ORDER BY id", "limit": 2}),
    )
    .await;
    assert!(text.starts_with("OK 2 rows in"), "{text}");
    assert!(text.contains("and more rows exist"));
    assert!(text.contains(results::DATA_NOTE));
    assert!(text.contains("[\"1\",\"Customer 1\"]"));
    let (text, _) = call(&client, "query", json!({"connection": "full", "sql": "SELECT hn_fixture.touch()"})).await;
    assert!(text.starts_with("REFUSED") && text.contains("25006"), "{text}");
    let (text, _) = call(&client, "describe", json!({"connection": "read", "schema": "hn_fixture"})).await;
    assert!(
        text.contains("- table invoices (about 120001 rows): id bigint, customer_id bigint"),
        "{text}"
    );
    assert!(text.contains("- hn_fixture.invoices(customer_id) → hn_fixture.customers(id)"));
    let (text, _) = call(
        &client,
        "describe",
        json!({"connection": "read", "schema": "hn_fixture", "table": "line_items"}),
    )
    .await;
    assert!(text.contains("Primary key: invoice_id, line"), "{text}");
}

/// A table of the test's own, created through the full connection.
async fn scratch(client: &RunningService<RoleClient, ClientConfig>, name: &str) {
    let statements = [
        format!("DROP TABLE IF EXISTS hn_fixture.{name}"),
        format!("CREATE TABLE hn_fixture.{name} (id int PRIMARY KEY, body text)"),
        format!("INSERT INTO hn_fixture.{name} VALUES (1, 'one'), (2, 'two')"),
    ];
    let (text, _) = call(client, "run_sql", json!({"connection": "full", "statements": statements})).await;
    assert!(text.starts_with("OK Committed 3 statements"), "{text}");
}

fn pending(agents: &Agents) -> ProposalView {
    agents.state().proposals.into_iter().next().expect("a proposal waits")
}

#[tokio::test]
async fn stages_a_change_and_waits_for_the_person() {
    let Some(url) = database_url() else { return };
    let world = world(Some(&url));
    let client = client(&world.agents, "claude-code").await;
    scratch(&client, "agent_approved").await;
    let change = json!({
        "connection": "approved",
        "table": {"schema": "hn_fixture", "name": "agent_approved"},
        "update": [{"key": {"id": 1}, "set": {"body": "uno"}}],
        "delete": [{"id": 2}],
    });
    let (text, _) = call(&client, "change_rows", change.clone()).await;
    assert!(text.starts_with("WAITING Proposal p-"), "{text}");
    assert_eq!(world.desk.forward.load(Ordering::SeqCst), 1);
    let proposal = pending(&world.agents);
    assert_eq!(proposal.summary, "change 2 rows in agent_approved");
    assert!(proposal.shown.iter().any(|s| s.contains("'uno'")));

    let (again, _) = call(&client, "change_rows", change).await;
    assert!(again.contains(&proposal.id), "the same change waits on the same proposal");
    let other = json!({"connection": "approved", "statements": ["UPDATE hn_fixture.agent_approved SET body = 'x'"]});
    let (busy, _) = call(&client, "run_sql", other).await;
    assert!(busy.starts_with("BUSY"));

    world.agents.approve(&proposal.id, false).await.unwrap();
    world.agents.approve(&proposal.id, false).await.unwrap();
    let (text, _) = call(&client, "wait_for_approval", json!({"proposal": proposal.id})).await;
    assert!(text.starts_with("APPROVED") && text.contains("2 rows"), "{text}");
    let (rows, _) = call(
        &client,
        "query",
        json!({"connection": "full", "sql": "SELECT id, body FROM hn_fixture.agent_approved"}),
    )
    .await;
    assert!(rows.contains("[\"1\",\"uno\"]") && !rows.contains("\"2\""), "{rows}");
}

#[tokio::test]
async fn a_decline_reaches_the_agent_with_the_reason() {
    let Some(url) = database_url() else { return };
    let world = world(Some(&url));
    let client = client(&world.agents, "claude-code").await;
    scratch(&client, "agent_declined").await;
    let agents = world.agents.clone();
    let decline = tokio::spawn(async move {
        loop {
            if let Some(p) = agents.state().proposals.first() {
                agents.decline(&p.id, "Not on Fridays.");
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let change = json!({"connection": "approved", "table": {"schema": "hn_fixture", "name": "agent_declined"}, "delete": [{"id": 1}]});
    let (text, _) = call(&client, "change_rows", change).await;
    decline.await.unwrap();
    assert!(text.starts_with("DECLINED") && text.contains("\"Not on Fridays.\""), "{text}");
    let logged = world.agents.activity(1);
    assert_eq!(
        (logged[0].outcome, logged[0].reason.as_deref()),
        (Some(Outcome::Declined), Some("Not on Fridays."))
    );
}

#[tokio::test]
async fn a_row_that_changes_before_approval_rolls_the_change_back() {
    let Some(url) = database_url() else { return };
    let world = world(Some(&url));
    let client = client(&world.agents, "claude-code").await;
    scratch(&client, "agent_conflict").await;
    let change = json!({"connection": "approved", "table": {"schema": "hn_fixture", "name": "agent_conflict"}, "update": [{"key": {"id": 1}, "set": {"body": "agent"}}]});
    let (text, _) = call(&client, "change_rows", change).await;
    assert!(text.starts_with("WAITING"));
    let moved = json!({"connection": "full", "statements": ["UPDATE hn_fixture.agent_conflict SET body = 'person' WHERE id = 1"]});
    call(&client, "run_sql", moved).await;
    let proposal = pending(&world.agents);
    let error = world.agents.approve(&proposal.id, false).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Conflict);
    let (text, _) = call(&client, "wait_for_approval", json!({"proposal": proposal.id})).await;
    assert!(text.starts_with("CONFLICT"), "{text}");
    assert!(
        matches!(pending(&world.agents).state, ProposalState::Failed { .. }),
        "the failure stays for the person to see"
    );
}

#[tokio::test]
async fn dont_ask_again_covers_this_agent_here_but_never_a_drop() {
    let Some(url) = database_url() else { return };
    let world = world(Some(&url));
    let claude = client(&world.agents, "claude-code").await;
    scratch(&claude, "agent_allowed").await;
    let update =
        |body: &str| json!({"connection": "approved", "statements": [format!("UPDATE hn_fixture.agent_allowed SET body = '{body}' WHERE id = 1")]});
    call(&claude, "run_sql", update("a")).await;
    world.agents.approve(&pending(&world.agents).id, true).await.unwrap();
    let (text, _) = call(&claude, "run_sql", update("b")).await;
    assert!(text.starts_with("OK Committed"), "{text}");

    let other = client(&world.agents, "another-agent").await;
    let (text, _) = call(&other, "run_sql", update("c")).await;
    assert!(text.starts_with("WAITING"), "{text}");
    let truncate = json!({"connection": "approved", "statements": ["TRUNCATE hn_fixture.agent_allowed"]});
    let (text, _) = call(&claude, "run_sql", truncate).await;
    assert!(text.starts_with("WAITING"), "{text}");
}

#[tokio::test]
async fn a_proposal_ends_when_its_agent_leaves() {
    let Some(url) = database_url() else { return };
    let world = world(Some(&url));
    let client = client(&world.agents, "claude-code").await;
    let change = json!({"connection": "approved", "statements": ["SELECT 1"]});
    call(&client, "run_sql", change).await;
    let proposal = pending(&world.agents);
    client.cancel().await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(world.agents.state().proposals.is_empty());
    let logged = world.agents.activity(1);
    assert_eq!(logged[0].outcome, Some(Outcome::Withdrawn));
    assert!(
        world.agents.approve(&proposal.id, false).await.is_ok(),
        "approving a withdrawn change does nothing"
    );
}

#[tokio::test]
async fn logs_each_call_before_it_runs_and_never_the_rows() {
    let Some(url) = database_url() else { return };
    let world = world(Some(&url));
    let client = client(&world.agents, "claude-code").await;
    call(
        &client,
        "query",
        json!({"connection": "read", "sql": "SELECT email FROM hn_fixture.customers WHERE id = 7"}),
    )
    .await;
    let logged = std::fs::read_to_string(world.dir.join("agent-activity.jsonl")).unwrap();
    assert!(logged.contains("SELECT email FROM hn_fixture.customers WHERE id = 7"));
    assert!(!logged.contains("customer7@example.com"));
}

#[tokio::test]
async fn asks_the_person_for_a_password_and_waits() {
    let Some(url) = database_url() else { return };
    let world = world(Some(&url));
    let parts = hn_core::parse_url(&url).unwrap();
    world.agents.directory.passwords.forget_typed("id-read");
    let agents = world.agents.clone();
    let answer = tokio::spawn(async move {
        loop {
            if let Some(request) = agents.state().passwords.first() {
                let typed = Secrets {
                    password: parts.password.clone(),
                    ssh: None,
                };
                agents.answer_password(&request.id, typed).await.unwrap();
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let client = client(&world.agents, "claude-code").await;
    let (text, _) = call(&client, "query", json!({"connection": "read", "sql": "SELECT 1 AS one"})).await;
    answer.await.unwrap();
    assert!(text.starts_with("OK 1 row"), "{text}");
    assert!(world.desk.forward.load(Ordering::SeqCst) >= 1);
}

#[tokio::test]
async fn an_agent_sees_a_hidden_column_only_as_hidden() {
    let Some(url) = database_url() else { return };
    let world = world(Some(&url));
    let mut full = world.agents.directory.store.get("id-full").unwrap();
    full.hidden_columns.push(hn_core::HiddenColumn {
        database: full.database.clone(),
        schema: "hn_fixture".into(),
        table: "customers".into(),
        column: "email".into(),
    });
    world.agents.directory.store.save(full).unwrap();
    let client = client(&world.agents, "claude-code").await;
    let (text, _) = call(
        &client,
        "query",
        json!({"connection": "full", "sql": "SELECT * FROM hn_fixture.customers WHERE id = 7"}),
    )
    .await;
    assert!(text.contains("[hidden]") && !text.contains("@example.com"), "{text}");
    let (text, _) = call(
        &client,
        "query",
        json!({"connection": "full", "sql": "SELECT upper(email) FROM hn_fixture.customers"}),
    )
    .await;
    assert!(text.starts_with("REFUSED") && text.contains("is hidden from agents"), "{text}");
    let (text, _) = call(
        &client,
        "describe",
        json!({"connection": "full", "schema": "hn_fixture", "table": "customers"}),
    )
    .await;
    assert!(text.contains("- email text (hidden from agents"), "{text}");
}

#[tokio::test]
async fn keeps_the_reverse_of_a_row_change_to_undo() {
    let Some(url) = database_url() else { return };
    let world = world(Some(&url));
    let client = client(&world.agents, "claude-code").await;
    scratch(&client, "agent_undo").await;
    let change = json!({
        "connection": "full",
        "table": {"schema": "hn_fixture", "name": "agent_undo"},
        "update": [{"key": {"id": 1}, "set": {"body": "uno"}}],
        "delete": [{"id": 2}],
        "insert": [{"id": 3, "body": "tres"}],
    });
    let (text, _) = call(&client, "change_rows", change).await;
    assert!(text.starts_with("OK Committed"), "{text}");
    let made = world.agents.activity(1).remove(0);
    assert!(made.undo, "the change can be undone");
    let record = world.agents.undo(&made.call).unwrap();
    let info = world.agents.databases.lock().await.values().next().cloned().unwrap();
    let session = world.agents.sessions.get(&info.session_id).await.unwrap();
    session.db.commit(&record.changes, &hn_core::Checks::NONE).await.unwrap();
    world.agents.undone(&made.call).unwrap();
    assert!(!world.agents.activity(1)[0].undo);
    let (rows, _) = call(
        &client,
        "query",
        json!({"connection": "full", "sql": "SELECT id, body FROM hn_fixture.agent_undo ORDER BY id"}),
    )
    .await;
    assert!(rows.contains("[\"1\",\"one\"]\n[\"2\",\"two\"]\n</rows>"), "{rows}");
}

#[tokio::test]
async fn an_agent_makes_and_deletes_its_own_database_and_asks_to_delete_another() {
    if std::env::var("HN_TEST_DOCKER").as_deref() != Ok("1") {
        eprintln!("skipped: HN_TEST_DOCKER is not 1");
        return;
    }
    let world = world(None);
    world
        .agents
        .settings
        .change(|c| {
            c.docker = true;
            c.backups = true;
        })
        .unwrap();
    let docker = world.agents.directory.docker.clone();
    docker.attach().await.expect("CI has a Docker engine");
    let name = format!("hn-agent-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let work = {
        let (agents, name) = (world.agents.clone(), name.clone());
        tokio::spawn(async move {
            let claude = client(&agents, "claude-code").await;
            let (made, _) = call(&claude, "create_database", json!({ "name": name })).await;
            assert!(
                made.starts_with("OK ") && made.contains("URL, for a program you run: postgres://"),
                "{made}"
            );
            let (again, _) = call(&claude, "create_database", json!({ "name": name })).await;
            assert!(again.starts_with("OK "), "asking again gives the same one: {again}");
            let (listed, _) = call(&claude, "list_connections", json!({})).await;
            assert!(
                listed.contains(&format!("{name} · ")) && listed.contains("made for Claude Code"),
                "{listed}"
            );

            let cursor = client(&agents, "cursor").await;
            let (asked, _) = call(&cursor, "delete_database", json!({ "connection": name })).await;
            assert!(asked.starts_with("WAITING "), "another agent's delete asks: {asked}");
            let request = agents.state().deletions.into_iter().next().expect("the window shows it");
            assert!(request.backed_up);
            assert_eq!(agents.answer_deletion(&request.id, false).await.unwrap().map(|d| d.kept_volume), None);
            let (declined, _) = call(&cursor, "delete_database", json!({ "connection": name })).await;
            assert!(declined.starts_with("DECLINED "), "{declined}");

            let (deleted, _) = call(&claude, "delete_database", json!({ "connection": name })).await;
            assert!(deleted.starts_with("OK Deleted") && deleted.contains("backed it up first"), "{deleted}");
            assert!(agents.directory.docker.container(&name).is_none());
            assert_eq!(agents.backups.list().len(), 1);
        })
    };
    let outcome = work.await;
    if let Ok(engine) = docker.engine().await
        && let Ok(c) = engine.container(&name).await
    {
        let _ = engine.remove(&name, c.volume.as_deref()).await;
    }
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic.into_panic());
    }
}

#[tokio::test]
async fn backups_come_before_an_agent_drops_or_empties_anything() {
    let Some(url) = database_url() else { return };
    let world = world(Some(&url));
    let claude = client(&world.agents, "claude-code").await;
    scratch(&claude, "agent_backed_up").await;
    world.agents.settings.change(|c| c.backups = true).unwrap();

    let run_sql = |statement: &str| json!({"connection": "full", "statements": [statement]});
    for lost in ["DROP TABLE hn_fixture.agent_backed_up", "/* old */ truncate hn_fixture.agent_backed_up"] {
        let (text, _) = call(&claude, "run_sql", run_sql(lost)).await;
        assert!(text.starts_with("REFUSED") && text.contains("change_structure"), "{text}");
    }
    let (text, _) = call(&claude, "run_sql", run_sql("DROP FUNCTION IF EXISTS hn_fixture.agent_no_such_function()")).await;
    assert!(text.starts_with("OK Committed"), "a drop that loses no data runs: {text}");

    let drop = json!({"connection": "full", "table": {"schema": "hn_fixture", "name": "agent_backed_up"}, "changes": [{"kind": "drop"}]});
    let (text, _) = call(&claude, "change_structure", drop).await;
    assert!(
        text.starts_with("OK Committed") && text.contains("backed up agent_backed_up first"),
        "{text}"
    );
    let backup = world.agents.backups.list().into_iter().next().expect("a backup");
    assert_eq!(backup.agent.map(|a| a.title).as_deref(), Some("Claude Code"));
    let entry = world
        .agents
        .activity(10)
        .into_iter()
        .find(|e| e.tool == "change_structure")
        .expect("the drop is in the activity list");
    assert_eq!(entry.backup.as_deref(), Some(backup.id.as_str()), "and offers to put it back");
}

#[tokio::test]
async fn an_agents_change_that_breaks_a_rule_never_commits() {
    let Some(url) = database_url() else { return };
    let database = hn_core::parse_url(&url).unwrap().database;
    let world = world(Some(&url));
    let client = client(&world.agents, "claude-code").await;
    scratch(&client, "agent_ruled").await;
    let table = hn_core::TableRef {
        schema: "hn_fixture".into(),
        name: "agent_ruled".into(),
    };
    let check = hn_core::RuleCheck::OneOf {
        column: "body".into(),
        values: vec!["one".into(), "two".into(), "three".into()],
    };
    for connection in ["id-full", "id-approved"] {
        let rule = hn_core::Rule {
            id: format!("rule-{connection}"),
            words: check.sentence(&table, true).unwrap(),
            table: table.clone(),
            check: check.clone(),
            source: hn_core::RuleSource::Written,
            added_ms: 0.0,
        };
        world.agents.rules.add(connection, &database, rule).unwrap();
    }
    let insert = |connection: &str, body: &str| {
        json!({
            "connection": connection,
            "table": {"schema": "hn_fixture", "name": "agent_ruled"},
            "insert": [{"id": 3, "body": body}],
        })
    };

    let (text, _) = call(&client, "change_rows", insert("full", "nope")).await;
    assert!(text.starts_with("REFUSED Nothing was written"), "{text}");
    assert!(text.contains("body is one of one, two and three") && text.contains("id = 3"), "{text}");
    assert!(text.contains("Change it so the rule holds"), "{text}");

    let (text, _) = call(&client, "change_rows", insert("approved", "nope")).await;
    assert!(text.starts_with("REFUSED") && text.contains("the person wasn't asked"), "{text}");
    assert_eq!(world.desk.forward.load(Ordering::SeqCst), 0, "the window didn't come forward");
    assert!(world.agents.state().proposals.is_empty());

    // Statements an agent wrote never run before approval, so the rule stops them at commit.
    let statements = json!({"connection": "approved", "statements": ["INSERT INTO hn_fixture.agent_ruled VALUES (4, 'nope')"]});
    let (text, _) = call(&client, "run_sql", statements).await;
    assert!(text.starts_with("WAITING"), "{text}");
    let proposal = pending(&world.agents);
    assert_eq!(world.agents.approve(&proposal.id, false).await.unwrap_err().kind, ErrorKind::BrokenRule);

    let (text, _) = call(&client, "change_rows", insert("full", "three")).await;
    assert!(text.starts_with("OK Committed"), "a change that keeps the rule commits: {text}");
}
