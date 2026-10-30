//! Backups made before something is dropped, emptied or deleted, kept until someone deletes them. Each is a folder
//! in `backups/` that only this account can read, listed in `backups/backups.json`. A table's folder holds its
//! definition and its rows as CSV; a Docker database's holds its roles and a dump of each database. A backup is
//! listed once its files are on disk and before the drop or delete happens, so a crash can leave an extra backup but
//! never lose one.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use hn_core::{
    AgentMark, Cell, Changes, Committed, Database, Error, ErrorKind, PageColumn, RestoreInto, RowSink, StructureChange, TableDefinition, TableRef,
    TableSink, csv_line,
};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::commands::now_ms;
use crate::docker::Docker;
use crate::docker::make::KeptContainer;
use crate::files::{self, private_dir, storage};
use crate::import::FileSource;
use crate::sessions::Session;
use crate::settings::Choices;

pub mod commands;

const INDEX: &str = "backups.json";
const VERSION: u32 = 1;
const DEFINITION: &str = "definition.json";
const ROWS: &str = "rows.csv";

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    pub id: String,
    pub made_at_ms: f64,
    pub holds: Holds,
    /// The agent whose change it was, or `None` for the person's own.
    pub agent: Option<AgentMark>,
    /// The agent call that made the change, as the activity log names it.
    #[serde(default)]
    pub call: Option<String>,
    /// The size of its files.
    pub bytes: f64,
    /// What it couldn't keep, in words, such as grants.
    pub not_kept: Vec<String>,
    /// When it was last put back.
    pub restored_at_ms: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Holds {
    #[serde(rename_all = "camelCase")]
    Table {
        connection_id: String,
        connection_name: String,
        database: String,
        table: TableRef,
        /// How it goes back, which also says what was done to it.
        into: RestoreInto,
        rows: f64,
    },
    /// A Docker database, deleted.
    Database { container: KeptContainer },
}

#[derive(Default, Serialize, Deserialize)]
struct Index {
    backups: Vec<Backup>,
}

/// A deleted Docker database, and the backup made before, if one was.
#[derive(Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeleteDone {
    /// Its data volume was kept, because Halfnine didn't make it.
    pub kept_volume: bool,
    pub backup: Option<Backup>,
}

/// An agent's call that changed something, which its backup names so the Agents page can offer to put it back.
pub struct AgentCall {
    pub agent: AgentMark,
    pub call: String,
}

/// A commit, and the backup made before it, if one was.
#[derive(Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CommitDone {
    pub committed: Committed,
    pub backup: Option<Backup>,
}

pub struct Backups {
    dir: PathBuf,
    index: Mutex<Vec<Backup>>,
}

/// What `changes` would lose, as the restore that puts it back: the table they drop, empty or take columns from.
pub fn loses(changes: &Changes) -> Option<(TableRef, RestoreInto)> {
    let Changes::Structure { table, changes } = changes else { return None };
    let into = if changes.iter().any(|c| matches!(c, StructureChange::Drop)) {
        RestoreInto::Recreate
    } else if changes.iter().any(|c| matches!(c, StructureChange::Truncate)) {
        RestoreInto::Refill
    } else {
        let names: Vec<String> = changes
            .iter()
            .filter_map(|c| match c {
                StructureChange::DropColumn { column } => Some(column.clone()),
                _ => None,
            })
            .collect();
        if names.is_empty() {
            return None;
        }
        RestoreInto::Columns { names }
    };
    Some((table.clone(), into))
}

impl Backups {
    pub fn open(config: &Path) -> Result<Self, Error> {
        let dir = config.join("backups");
        let index = files::load::<Index>(&dir.join(INDEX), VERSION, "the list of backups")?;
        Ok(Self {
            dir,
            index: Mutex::new(index.backups),
        })
    }

    pub fn list(&self) -> Vec<Backup> {
        let mut backups = self.lock().clone();
        backups.reverse();
        backups
    }

    pub fn get(&self, id: &str) -> Result<Backup, Error> {
        self.lock()
            .iter()
            .find(|b| b.id == id)
            .cloned()
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "That backup has been deleted."))
    }

    /// Commits `changes`, backing up first what they would lose, as `choices` allow. Refuses with
    /// [`ErrorKind::NoBackup`], saying why, when that can't be backed up, unless `without_backup` says someone
    /// confirmed going ahead without one.
    pub async fn commit(
        &self,
        choices: &Choices,
        session: &Session,
        changes: &Changes,
        by: Option<AgentCall>,
        without_backup: bool,
    ) -> Result<CommitDone, Error> {
        let Some((table, into)) = loses(changes).filter(|_| !without_backup) else {
            let committed = session.db.commit(changes).await?;
            return Ok(CommitDone { committed, backup: None });
        };
        let refused = |why: String| Error::new(ErrorKind::NoBackup, why);
        if !choices.backups {
            return Err(refused("Backups are off in Settings.".into()));
        }
        let size = session.db.backup_size(&table, &into).await.map_err(|e| match e.kind {
            ErrorKind::Invalid => refused(e.message),
            _ => e,
        })?;
        if let Some(limit) = choices.backup_limit_bytes.filter(|limit| size > *limit) {
            return Err(refused(format!(
                "{} takes {}, more than the {} Halfnine backs up. Settings can raise that.",
                table.name,
                size_in_words(size),
                size_in_words(limit)
            )));
        }
        let mut files = self.start(session, table, into, by)?;
        let id = files.backup.id.clone();
        let table = files.table.clone();
        match session.db.commit_with_backup(changes, &table, &mut files).await {
            Ok(committed) => Ok(CommitDone {
                committed,
                backup: Some(self.get(&id)?),
            }),
            Err(error) => {
                let _ = self.delete(&id);
                Err(error)
            }
        }
    }

    /// Backs up a Docker database before it is deleted, as `choices` allow, and refuses with
    /// [`ErrorKind::NoBackup`], saying why, when it can't be or is over the size limit, unless `without_backup` says someone confirmed going
    /// ahead without one. `delete` runs once the backup is listed.
    pub async fn delete_database(
        &self,
        choices: &Choices,
        docker: &Docker,
        name: &str,
        agent: Option<AgentMark>,
        without_backup: bool,
        delete: impl AsyncFnOnce() -> Result<bool, Error>,
    ) -> Result<DeleteDone, Error> {
        if without_backup {
            return Ok(DeleteDone {
                kept_volume: delete().await?,
                backup: None,
            });
        }
        if !choices.backups {
            return Err(Error::new(ErrorKind::NoBackup, "Backups are off in Settings."));
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let folder = self.dir.join(&id);
        let kept = match docker.back_up(name, &folder, choices.backup_limit_bytes).await {
            Ok(kept) => kept,
            Err(error) => {
                let _ = fs::remove_dir_all(&folder);
                return Err(match error.kind {
                    ErrorKind::NoBackup => error,
                    _ => Error {
                        kind: ErrorKind::NoBackup,
                        message: format!("The backup failed, so nothing was deleted: {}", error.message),
                        ..error
                    },
                });
            }
        };
        let backup = Backup {
            id: id.clone(),
            made_at_ms: now_ms(),
            holds: Holds::Database { container: kept },
            agent,
            call: None,
            bytes: folder_bytes(&folder),
            not_kept: vec![
                "role passwords".into(),
                "container settings beyond its image, port, user and database".into(),
            ],
            restored_at_ms: None,
        };
        let listed = backup.clone();
        self.change(|backups| backups.push(listed))?;
        match delete().await {
            Ok(kept_volume) => Ok(DeleteDone {
                kept_volume,
                backup: Some(backup),
            }),
            Err(error) => {
                let _ = self.delete(&id);
                Err(error)
            }
        }
    }

    /// Makes a deleted Docker database again from its backup, as a job the Docker page follows, and waits for it.
    pub async fn restore_database(&self, docker: &std::sync::Arc<Docker>, id: &str) -> Result<hn_core::ConnectionSpec, Error> {
        let Holds::Database { container } = self.get(id)?.holds else {
            return Err(Error::new(ErrorKind::Invalid, "This backup is of a table."));
        };
        let name = docker.free_name(&container.name).await?;
        let folder = self.dir.join(id);
        let job = std::sync::Arc::clone(docker);
        let target = name.clone();
        let mut done = docker.job(&name, "Putting it back…", async move { job.put_back(&container, &target, &folder).await });
        let outcome = done
            .wait_for(Option::is_some)
            .await
            .map_err(|_| Error::new(ErrorKind::Docker, "Halfnine lost track of putting it back."))?
            .clone()
            .unwrap_or_else(|| Err(Error::new(ErrorKind::Docker, "Halfnine lost track of putting it back.")));
        let spec = outcome?;
        self.restored(id)?;
        Ok(spec)
    }

    fn restored(&self, id: &str) -> Result<(), Error> {
        self.change(|backups| {
            if let Some(backup) = backups.iter_mut().find(|b| b.id == id) {
                backup.restored_at_ms = Some(now_ms());
            }
        })
    }

    fn start(&self, session: &Session, table: TableRef, into: RestoreInto, by: Option<AgentCall>) -> Result<TableFiles<'_>, Error> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let folder = self.dir.join(&id);
        private_dir(&folder)?;
        let rows = private_file(&folder.join(ROWS))?;
        Ok(TableFiles {
            backups: self,
            folder,
            table: table.clone(),
            rows: BufWriter::new(rows),
            written: 0.0,
            backup: Backup {
                id,
                made_at_ms: now_ms(),
                holds: Holds::Table {
                    connection_id: session.connection.id.clone(),
                    connection_name: session.connection.name.clone(),
                    database: session.database.clone(),
                    table,
                    into,
                    rows: 0.0,
                },
                agent: by.as_ref().map(|b| b.agent.clone()),
                call: by.map(|b| b.call),
                bytes: 0.0,
                not_kept: Vec::new(),
                restored_at_ms: None,
            },
        })
    }

    /// The statements putting a table backup back would run, to read first.
    pub async fn review_restore(&self, session: &Session, id: &str) -> Result<Vec<String>, Error> {
        let (definition, into) = self.table_backup(session, id)?;
        session.db.review_restore(&definition, &into).await
    }

    /// Puts a table backup back in one transaction, and gives how many rows went back.
    pub async fn restore(&self, session: &Session, id: &str) -> Result<f64, Error> {
        let (definition, into) = self.table_backup(session, id)?;
        let mut rows = FileSource::open(&self.dir.join(id).join(ROWS))?;
        let restored = session.db.restore(&definition, &into, &mut rows).await?;
        self.restored(id)?;
        Ok(restored)
    }

    fn table_backup(&self, session: &Session, id: &str) -> Result<(TableDefinition, RestoreInto), Error> {
        let Holds::Table {
            connection_id,
            connection_name,
            database,
            into,
            ..
        } = self.get(id)?.holds
        else {
            return Err(Error::new(ErrorKind::Invalid, "This backup is of a Docker database."));
        };
        if connection_id != session.connection.id || database != session.database {
            return Err(Error::new(
                ErrorKind::Invalid,
                format!("This backup is of {database} on {connection_name}. Connect to it there to put it back."),
            ));
        }
        let definition = files::read::<TableDefinition>(&self.dir.join(id).join(DEFINITION), VERSION, "the backup")?
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "This backup's files are gone."))?;
        Ok((definition, into))
    }

    pub fn delete(&self, id: &str) -> Result<(), Error> {
        self.change(|backups| backups.retain(|b| b.id != id))?;
        match fs::remove_dir_all(self.dir.join(id)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(storage("Couldn't delete the backup's files", e)),
            _ => Ok(()),
        }
    }

    /// The size of every backup together.
    pub fn bytes(&self) -> f64 {
        self.lock().iter().map(|b| b.bytes).sum()
    }

    fn change(&self, edit: impl FnOnce(&mut Vec<Backup>)) -> Result<(), Error> {
        let mut backups = self.lock();
        let mut next = backups.clone();
        edit(&mut next);
        private_dir(&self.dir)?;
        files::save_private(&self.dir.join(INDEX), VERSION, &Index { backups: next.clone() }, "the list of backups")?;
        *backups = next;
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Backup>> {
        self.index.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// A table backup being written, as the commit reads the table.
struct TableFiles<'a> {
    backups: &'a Backups,
    folder: PathBuf,
    table: TableRef,
    rows: BufWriter<File>,
    written: f64,
    backup: Backup,
}

impl RowSink for TableFiles<'_> {
    fn columns(&mut self, columns: &[PageColumn]) -> Result<(), Error> {
        self.rows
            .write_all(csv_line(columns.iter().map(|c| Some(c.name.as_str()))).as_bytes())
            .map_err(|e| storage("Couldn't write the backup", e))
    }

    fn row(&mut self, row: &[Cell]) -> Result<(), Error> {
        self.written += 1.0;
        self.rows
            .write_all(csv_line(row.iter().map(Option::as_deref)).as_bytes())
            .map_err(|e| storage("Couldn't write the backup", e))
    }
}

impl TableSink for TableFiles<'_> {
    fn definition(&mut self, definition: &TableDefinition) -> Result<(), Error> {
        self.backup.not_kept = definition.not_kept.clone();
        files::save_private(&self.folder.join(DEFINITION), VERSION, definition, "the backup")
    }

    fn finish(&mut self) -> Result<(), Error> {
        let failed = |e: std::io::Error| storage("Couldn't write the backup", e);
        self.rows.flush().map_err(failed)?;
        self.rows.get_ref().sync_all().map_err(failed)?;
        File::open(self.folder.join(DEFINITION)).and_then(|f| f.sync_all()).map_err(failed)?;
        self.backup.bytes = folder_bytes(&self.folder);
        if let Holds::Table { rows, .. } = &mut self.backup.holds {
            *rows = self.written;
        }
        let backup = self.backup.clone();
        self.backups.change(|backups| backups.push(backup))
    }
}

fn folder_bytes(folder: &Path) -> f64 {
    fs::read_dir(folder)
        .map(|entries| entries.flatten().filter_map(|e| e.metadata().ok()).map(|m| m.len() as f64).sum())
        .unwrap_or_default()
}

fn private_file(path: &Path) -> Result<File, Error> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path).map_err(|e| storage("Couldn't create the backup", e))
}

/// A size as Finder writes it, in powers of 1,000: "820 KB", "3.2 GB".
pub fn size_in_words(bytes: f64) -> String {
    let units = ["bytes", "KB", "MB", "GB", "TB"];
    let mut size = bytes;
    let mut unit = 0;
    while size >= 1000.0 && unit < units.len() - 1 {
        size /= 1000.0;
        unit += 1;
    }
    let number = if unit > 0 && size < 10.0 {
        format!("{size:.1}").trim_end_matches(".0").to_string()
    } else {
        format!("{size:.0}")
    };
    format!("{number} {}", units[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_what_a_change_would_lose() {
        let table = TableRef {
            schema: "public".into(),
            name: "notes".into(),
        };
        let structure = |changes| Changes::Structure {
            table: table.clone(),
            changes,
        };
        let drop_column = |name: &str| StructureChange::DropColumn { column: name.into() };
        assert_eq!(loses(&structure(vec![StructureChange::Truncate])).map(|l| l.1), Some(RestoreInto::Refill));
        assert_eq!(
            loses(&structure(vec![drop_column("a"), drop_column("b")])).map(|l| l.1),
            Some(RestoreInto::Columns {
                names: vec!["a".into(), "b".into()]
            })
        );
        assert_eq!(
            loses(&structure(vec![drop_column("a"), StructureChange::Drop])).map(|l| l.1),
            Some(RestoreInto::Recreate)
        );
        assert_eq!(loses(&structure(vec![])), None);
    }

    #[test]
    fn writes_sizes_as_finder_does() {
        assert_eq!(size_in_words(512.0), "512 bytes");
        assert_eq!(size_in_words(3_200_000_000.0), "3.2 GB");
        assert_eq!(size_in_words(1_000_000_000.0), "1 GB");
        assert_eq!(size_in_words(820_000.0), "820 KB");
    }
}

#[cfg(test)]
mod database_tests {
    use hn_core::{ConnectionSpec, Label, TlsMode};
    use hn_pg::{PgDatabase, Secrets};

    use super::*;

    /// A session on the Postgres CI provides, or `None` without `HN_TEST_DATABASE_URL`.
    async fn session() -> Option<Session> {
        let Ok(url) = std::env::var("HN_TEST_DATABASE_URL") else {
            eprintln!("skipped: HN_TEST_DATABASE_URL is not set");
            return None;
        };
        let parts = hn_core::parse_url(&url).unwrap();
        let connection = ConnectionSpec {
            id: "backups".into(),
            name: "Backups".into(),
            host: parts.host,
            port: parts.port,
            user: parts.user,
            database: parts.database.clone(),
            tls: parts.tls.unwrap_or(TlsMode::Disable),
            label: Label::None,
            read_only: false,
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
        let db = PgDatabase::connect(&connection, None, &secrets).await.unwrap();
        Some(Session {
            connection,
            database: parts.database,
            db,
            secrets,
        })
    }

    fn drop_table(table: &TableRef) -> Changes {
        Changes::Structure {
            table: table.clone(),
            changes: vec![StructureChange::Drop],
        }
    }

    #[tokio::test]
    async fn backs_a_table_up_privately_before_dropping_it_and_puts_it_back() {
        let Some(session) = session().await else { return };
        let dir = std::env::temp_dir().join(format!("hn-backups-{}", uuid::Uuid::new_v4().simple()));
        let backups = Backups::open(&dir).unwrap();
        let table = TableRef {
            schema: "public".into(),
            name: "hn_backups_notes".into(),
        };
        let made = session
            .db
            .run(&[
                "DROP TABLE IF EXISTS public.hn_backups_notes".to_string(),
                "CREATE TABLE public.hn_backups_notes (id int PRIMARY KEY, body text)".to_string(),
                "INSERT INTO public.hn_backups_notes VALUES (1, 'kept'), (2, ''), (3, NULL)".to_string(),
            ])
            .await
            .unwrap();
        assert!(made.failed.is_none());

        let off = Choices {
            backups: false,
            ..Choices::default()
        };
        let refused = backups.commit(&off, &session, &drop_table(&table), None, false).await;
        assert_eq!(refused.err().map(|e| e.kind), Some(ErrorKind::NoBackup));
        let small = Choices {
            backup_limit_bytes: Some(1.0),
            ..Choices::default()
        };
        let refused = backups.commit(&small, &session, &drop_table(&table), None, false).await;
        assert!(refused.unwrap_err().message.contains("Settings can raise that"));

        let done = backups
            .commit(&Choices::default(), &session, &drop_table(&table), None, false)
            .await
            .unwrap();
        let backup = done.backup.unwrap();
        assert!(matches!(backup.holds, Holds::Table { rows: 3.0, .. }));
        assert!(backup.bytes > 0.0);
        let folder = dir.join("backups").join(&backup.id);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dir.join("backups")), 0o700);
            assert_eq!(mode(&folder), 0o700);
            assert_eq!(mode(&folder.join(ROWS)), 0o600);
            assert_eq!(mode(&folder.join(DEFINITION)), 0o600);
            assert_eq!(mode(&dir.join("backups").join(INDEX)), 0o600);
        }
        assert_eq!(Backups::open(&dir).unwrap().list().len(), 1, "the index is on disk");

        assert!(backups.review_restore(&session, &backup.id).await.unwrap()[0].starts_with("CREATE TABLE"));
        assert_eq!(backups.restore(&session, &backup.id).await.unwrap(), 3.0);
        assert!(backups.get(&backup.id).unwrap().restored_at_ms.is_some());
        let back = session
            .db
            .run(&["SELECT body FROM public.hn_backups_notes ORDER BY id".to_string()])
            .await
            .unwrap();
        assert_eq!(back.results[0].rows, [[Some("kept".to_string())], [Some(String::new())], [None]]);

        backups.delete(&backup.id).unwrap();
        assert!(!folder.exists() && backups.list().is_empty());
        let _ = session.db.run(&["DROP TABLE public.hn_backups_notes".to_string()]).await;
        let _ = fs::remove_dir_all(dir);
    }
}
