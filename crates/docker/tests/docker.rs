//! The adapter against a real Docker engine. CI's Linux runner has one, with postgres:18 already pulled for the
//! Postgres service, so nothing is downloaded. Without `HN_TEST_DOCKER=1` these tests report that they were skipped
//! and pass, so a laptop never starts containers by surprise. Every container they make is labelled as a test's and
//! removed, whatever happens.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use hn_docker::{Engine, NewContainer, discover, label, pick_port};

async fn engine() -> Option<Engine> {
    if std::env::var("HN_TEST_DOCKER").as_deref() != Ok("1") {
        eprintln!("skipped: HN_TEST_DOCKER is not 1");
        return None;
    }
    Some(Engine::find(&discover::here()).await.expect("CI has a Docker engine"))
}

fn new_container(name: &str, port: u16) -> NewContainer {
    NewContainer {
        name: name.into(),
        image: "postgres:18".into(),
        major: 18,
        port,
        user: "app".into(),
        password: "test-password".into(),
        database: "shop".into(),
        volume: format!("{name}-data"),
        labels: HashMap::from([
            (label::TEST.to_string(), "true".to_string()),
            (label::MANAGED.to_string(), "true".to_string()),
        ]),
    }
}

/// Waits until Postgres inside answers on its TCP socket, not just the socket the first setup uses.
async fn ready(engine: &Engine, name: &str) {
    for _ in 0..120 {
        if let Ok((ran, _)) = engine.run(name, &["pg_isready", "-h", "127.0.0.1", "-U", "app", "-d", "shop"]).await
            && ran.succeeded()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("{name} never became ready");
}

async fn psql(engine: &Engine, name: &str, sql: &str) -> String {
    let (ran, out) = engine.run(name, &["psql", "-U", "app", "-d", "shop", "-tAc", sql]).await.unwrap();
    assert!(ran.succeeded(), "psql failed: {}", ran.stderr);
    String::from_utf8(out).unwrap().trim().to_string()
}

#[tokio::test]
async fn makes_dumps_restores_and_removes_postgres_containers() {
    let Some(engine) = engine().await else { return };
    let suffix = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let (first, second) = (format!("hn-test-a-{suffix}"), format!("hn-test-b-{suffix}"));
    assert!(
        engine.has_image("postgres:18").await.unwrap(),
        "the Postgres service already pulled postgres:18"
    );

    let work = {
        let (first, second) = (first.clone(), second.clone());
        tokio::spawn(async move {
            let engine = Engine::find(&discover::here()).await.unwrap();
            let taken: HashSet<u16> = engine.containers().await.unwrap().iter().filter_map(|c| c.port).collect();
            let free = |port| std::net::TcpListener::bind(("127.0.0.1", port)).is_ok();
            let port = pick_port(&taken, free).unwrap();
            engine.create(&new_container(&first, port)).await.unwrap();
            engine.start(&first).await.unwrap();
            ready(&engine, &first).await;
            psql(&engine, &first, "CREATE TABLE t (x int); INSERT INTO t VALUES (1), (2)").await;

            let seen = engine.container(&first).await.unwrap();
            assert_eq!((seen.major, seen.port, seen.running), (Some(18), Some(port), true));
            assert_eq!((seen.credentials.user.as_str(), seen.credentials.database.as_str()), ("app", "shop"));
            assert_eq!(seen.volume.as_deref(), Some(format!("{first}-data").as_str()));
            assert!(engine.containers().await.unwrap().iter().any(|c| c.name == first));

            let dump = std::env::temp_dir().join(format!("{first}.dump"));
            let mut file = tokio::fs::File::create(&dump).await.unwrap();
            let ran = engine
                .run_to(&first, &["pg_dump", "-Fc", "-U", "app", "-d", "shop"], &mut file)
                .await
                .unwrap();
            assert!(ran.succeeded(), "pg_dump failed: {}", ran.stderr);

            let taken: HashSet<u16> = [port].into();
            let second_port = pick_port(&taken, free).unwrap();
            engine.create(&new_container(&second, second_port)).await.unwrap();
            engine.start(&second).await.unwrap();
            ready(&engine, &second).await;
            engine.upload(&second, "/tmp", "shop.dump", &dump).await.unwrap();
            let (ran, _) = engine
                .run(&second, &["pg_restore", "--no-owner", "-U", "app", "-d", "shop", "/tmp/shop.dump"])
                .await
                .unwrap();
            assert!(ran.succeeded(), "pg_restore failed: {}", ran.stderr);
            assert_eq!(psql(&engine, &second, "SELECT count(*) FROM t").await, "2");

            engine.stop(&second).await.unwrap();
            engine.stop(&second).await.unwrap();
            assert!(!engine.container(&second).await.unwrap().running);
            assert_eq!(
                engine.container(&second).await.unwrap().port,
                Some(second_port),
                "a stopped container keeps its port"
            );
            let _ = std::fs::remove_file(dump);
        })
    };
    let outcome = work.await;
    for name in [&first, &second] {
        let _ = engine.remove(name, Some(&format!("{name}-data"))).await;
    }
    assert!(engine.container(&first).await.is_err(), "the container is gone");
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic.into_panic());
    }
}
