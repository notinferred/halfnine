//! Docker databases end to end, against CI's Docker engine, where postgres:18 is already pulled. Without
//! `HN_TEST_DOCKER=1` these tests report that they were skipped and pass. What they make is labelled as a test's, and
//! removed whatever happens.

use std::sync::Arc;

use hn_core::{AgentAccess, Database};
use hn_pg::PgDatabase;

use super::make::{NewDatabase, Origin};
use super::*;

/// A Docker service with Docker databases on, attached to the engine, in a folder of its own.
async fn docker() -> Option<(Arc<Docker>, PathBuf)> {
    if std::env::var("HN_TEST_DOCKER").as_deref() != Ok("1") {
        eprintln!("skipped: HN_TEST_DOCKER is not 1");
        return None;
    }
    let dir = std::env::temp_dir().join(format!("hn-docker-{}", uuid::Uuid::new_v4().simple()));
    let settings = Arc::new(Settings::open(&dir).unwrap());
    settings.change(|c| c.docker = true).unwrap();
    let docker = Arc::new(Docker::new(&dir, settings, || {}));
    docker.attach().await.expect("CI has a Docker engine");
    Some((docker, dir))
}

fn new_database(name: &str) -> NewDatabase {
    NewDatabase {
        name: name.into(),
        major: 18,
        image: None,
        load: None,
        origin: Origin::New,
        copy_of: None,
        agent: None,
        user: None,
        database: None,
        port: None,
        password: None,
    }
}

#[tokio::test]
async fn makes_a_database_that_connects_with_its_own_credentials_then_deletes_it() {
    let Some((docker, dir)) = docker().await else { return };
    let name = format!("hn-made-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let sql = dir.join("load.sql");
    std::fs::write(&sql, "CREATE TABLE greeting (word text); INSERT INTO greeting VALUES ('hello');").unwrap();
    let work = {
        let (docker, name) = (docker.clone(), name.clone());
        tokio::spawn(async move {
            let made = docker
                .make(NewDatabase {
                    load: Some(sql),
                    ..new_database(&name)
                })
                .await
                .unwrap();
            assert_eq!(made.agent_access(), AgentAccess::Full, "Halfnine made it");
            let secrets = docker.secrets(&name).await.unwrap();
            let password = secrets.password.clone().expect("its container was made with a password");
            assert!(!format!("{:?}", docker.state()).contains(&password), "the window never receives it");
            let db = PgDatabase::connect(&made, None, &secrets).await.unwrap();
            let rows = db.read("SELECT word FROM greeting", 5, &[]).await.unwrap();
            assert_eq!(rows.rows, [[Some("hello".to_string())]]);
            db.close().await;
            let again = docker.make(new_database(&name)).await.unwrap_err();
            assert!(again.message.contains("already a container"), "{}", again.message);
            let kept = docker.delete(&name).await.unwrap();
            assert!(!kept, "Halfnine's own volume goes with it");
            assert!(docker.container(&name).is_none());
        })
    };
    let outcome = work.await;
    if let Ok(engine) = docker.engine().await {
        let _ = engine.remove(&name, None).await;
    }
    let _ = std::fs::remove_dir_all(dir);
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic.into_panic());
    }
}

#[tokio::test]
async fn a_copy_has_the_same_rows_and_hides_the_same_columns() {
    let Some((docker, dir)) = docker().await else { return };
    let suffix = &uuid::Uuid::new_v4().simple().to_string()[..8];
    let (source, copy) = (format!("hn-source-{suffix}"), format!("hn-copy-{suffix}"));
    let sql = dir.join("load.sql");
    std::fs::write(
        &sql,
        "CREATE TABLE people (email text); INSERT INTO people VALUES ('a@example.com'), ('b@example.com');",
    )
    .unwrap();
    let work = {
        let (docker, source, copy) = (docker.clone(), source.clone(), copy.clone());
        tokio::spawn(async move {
            let made = docker
                .make(NewDatabase {
                    load: Some(sql),
                    ..new_database(&source)
                })
                .await
                .unwrap();
            let hidden = hn_core::HiddenColumn {
                database: made.database.clone(),
                schema: "public".into(),
                table: "people".into(),
                column: "email".into(),
            };
            docker.set_overrides(&source, |o| o.hidden_columns.push(hidden.clone())).unwrap();
            let copied = docker
                .copy(
                    &source,
                    NewDatabase {
                        origin: Origin::Copy,
                        ..new_database(&copy)
                    },
                )
                .await
                .unwrap();
            assert_eq!(
                (copied.user.as_str(), copied.database.as_str()),
                (made.user.as_str(), made.database.as_str())
            );
            assert_eq!(docker.overrides(&copy).hidden_columns, [hidden]);
            let Source::Docker { made: Some(how), .. } = &copied.source else {
                panic!("Halfnine made the copy")
            };
            assert_eq!(how.how, MadeHow::Copy);
            let db = PgDatabase::connect(&copied, None, &docker.secrets(&copy).await.unwrap()).await.unwrap();
            let rows = db.read("SELECT count(*) FROM people", 5, &[]).await.unwrap();
            assert_eq!(rows.rows, [[Some("2".to_string())]]);
            db.close().await;
        })
    };
    let outcome = work.await;
    if let Ok(engine) = docker.engine().await {
        for name in [&source, &copy] {
            if let Ok(c) = engine.container(name).await {
                let _ = engine.remove(name, c.volume.as_deref()).await;
            }
        }
    }
    let _ = std::fs::remove_dir_all(dir);
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic.into_panic());
    }
}

#[tokio::test]
async fn deleting_backs_a_database_up_first_and_it_comes_back_as_it_was() {
    let Some((docker, dir)) = docker().await else { return };
    let name = format!("hn-kept-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let sql = dir.join("load.sql");
    std::fs::write(&sql, "CREATE TABLE notes (body text); INSERT INTO notes VALUES ('kept'), ('');").unwrap();
    let work = {
        let (docker, name, dir) = (docker.clone(), name.clone(), dir.clone());
        tokio::spawn(async move {
            let made = docker
                .make(NewDatabase {
                    load: Some(sql),
                    ..new_database(&name)
                })
                .await
                .unwrap();
            docker
                .set_overrides(&name, |o| {
                    o.label = Some(Label::Amber);
                    o.read_only = true;
                })
                .unwrap();
            crate::secrets::use_test_store();
            let passwords = Arc::new(crate::secrets::Passwords::default());
            let backups = crate::backups::Backups::open(&dir, passwords.clone()).unwrap();
            let password = docker.secrets(&name).await.unwrap().password;
            let delete = async || docker.delete(&name).await;
            let off = crate::settings::Choices {
                backups: false,
                ..Default::default()
            };
            let refused = backups.delete_database(&off, &docker, &name, None, false, delete).await;
            assert_eq!(refused.unwrap_err().kind, ErrorKind::NoBackup);
            assert!(docker.container(&name).is_some(), "nothing was deleted");
            let small = crate::settings::Choices {
                backup_limit_bytes: Some(1.0),
                ..Default::default()
            };
            let delete = async || docker.delete(&name).await;
            let refused = backups.delete_database(&small, &docker, &name, None, false, delete).await.unwrap_err();
            assert!(
                refused.message.contains("Settings can raise that"),
                "a big database asks: {}",
                refused.message
            );
            assert!(docker.container(&name).is_some() && backups.list().is_empty());

            let delete = async || docker.delete(&name).await;
            let done = backups
                .delete_database(&Default::default(), &docker, &name, None, false, delete)
                .await
                .unwrap();
            assert!(!done.kept_volume, "Halfnine's own volume goes with it");
            assert!(docker.container(&name).is_none());
            let backup = done.backup.unwrap();
            let crate::backups::Holds::Database { container: kept } = &backup.holds else {
                panic!("a database backup")
            };
            assert!(kept.databases.contains(&made.database), "{:?}", kept.databases);

            let folder = dir.join("backups").join(&backup.id);
            let kept_password = passwords.for_backup(&backup.id).unwrap();
            assert_eq!(kept_password, password, "its password is kept with the backup");
            let back = docker
                .put_back(kept, &docker.free_name(&name).await.unwrap(), &folder, kept_password)
                .await
                .unwrap();
            assert_eq!(
                docker.secrets(&name).await.unwrap().password,
                password,
                "it comes back with the same password"
            );
            assert_eq!((back.name.as_str(), back.label, back.read_only), (name.as_str(), Label::Amber, true));
            let Source::Docker { made: Some(how), .. } = &back.source else {
                panic!("Halfnine made it")
            };
            assert_eq!(how.how, MadeHow::New, "it keeps how it was first made");
            let db = PgDatabase::connect(&back, None, &docker.secrets(&name).await.unwrap()).await.unwrap();
            let rows = db.read("SELECT body FROM notes ORDER BY body DESC", 5, &[]).await.unwrap();
            assert_eq!(rows.rows, [[Some("kept".to_string())], [Some(String::new())]]);
            db.close().await;
        })
    };
    let outcome = work.await;
    if let Ok(engine) = docker.engine().await
        && let Ok(c) = engine.container(&name).await
    {
        let _ = engine.remove(&name, c.volume.as_deref()).await;
    }
    let _ = std::fs::remove_dir_all(dir);
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic.into_panic());
    }
}
