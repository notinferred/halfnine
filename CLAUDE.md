# Halfnine

A small, calm PostgreSQL client for macOS and Windows, light enough to leave open all day.

## Where each fact lives

Each fact has one home, and everything else points to it. Docs don't repeat what the code shows, and a rule the code doesn't enforce yet is a to-do in ROADMAP.md, not a guarantee.

| What | Where | Changes when |
|---|---|---|
| Commands and install steps | README.md | A command changes |
| The license | LICENSE.md, PolyForm Shield 1.0.0, in every commit since the first | Only by Dan |
| Now and next | ROADMAP.md | A stage finishes; it is rewritten, not appended to |
| The design, its tools and why, the invariants | docs/architecture.md | The design changes |
| Dan's decisions, in his words | docs/decisions.md | Dan decides; entries are never reworded |
| Name, voice, colour, type, icon | docs/brand.md, with values in apps/desktop/src/ui/tokens.css | Only on purpose, said to Dan |
| What the code guarantees | This file, below | Only on purpose, said to Dan |

## What the code guarantees

- No crate uses `unsafe` code; the workspace forbids it.
- Every dependency we name is pinned exactly, at a version published at least two weeks before it was added. pnpm enforces that age for every JavaScript package, direct or not (`minimumReleaseAge` in `pnpm-workspace.yaml`). Cargo has no stable way to do the same, so for the Rust crates beneath ours, `Cargo.lock` records whatever versions were current when it was made.
- The window may call only what `crates/desktop/capabilities/default.json` grants, and its content security policy loads nothing from outside the app.
- The app sends nothing anywhere except to the databases you connect it to, the SSH servers that lead to them, and AI agents on this computer, through `halfnine mcp` over a socket only this account can open after it sends this launch's token (`crates/desktop/src/mcp/`). It changes another app's settings only when someone presses Connect on the Agents page, after showing the change, and copies the file first (`crates/desktop/src/setup.rs`).
- While Docker databases are on in Settings, it also talks to the Docker engine on this computer, through its socket or named pipe, never over TCP or SSH (`crates/docker/src/discover.rs`). The engine downloads a PostgreSQL image from its registry only when someone, or an agent, makes a database of a version it doesn't have.
- A password or SSH secret is saved only in Halfnine's one OS keychain entry (`crates/desktop/src/secrets.rs`), and only for a connection set to save it, or for a deleted Docker database, whose container's password is kept there with its backup until the backup is deleted. That entry is read at most once per launch. A password typed for a connection that asks each time stays in memory until the app quits. No command returns one, and `connections.json` holds none.
- A Docker database's credentials are read from its container each time it connects and never saved; `docker.json` holds only what someone set for it. An agent receives a URL, password included, only for a Docker database Halfnine made, at Full access, not read-only and hiding no columns, decided in `agents::verdict`.
- With backups on, dropping or emptying a table, or dropping a column, first writes the table to `backups/` in the same transaction, under a lock that makes writers wait, so a backup that fails stops the drop (`crates/pg/src/backup.rs`). A Docker database is dumped there before it is deleted. Backups hold row data, hidden columns included, and a Docker database's role password hashes, in folders only this account can read, and stay until someone deletes them (`crates/desktop/src/backups/`). A Docker database put back has the passwords it had. When one can't be made, nothing is dropped until the table's name is typed.
- Identifiers reach SQL only through `sql::ident`, and values only as bound parameters. Every statement the app builds runs through `PgDatabase::query` in `crates/pg`, except an import's `COPY`, which is fixed text. The exceptions are SQL someone types: a browse WHERE condition, a column's type or default in the structure editor, and the editor's statements; SQL an agent sends: a query, statements for `run_sql`, and the types and defaults of its structure changes; and the definitions Postgres printed for a backed-up table, which a restore shows before it runs them. Postgres checks that each is one statement.
- Staged changes are written in one transaction or not at all. An update or delete that finds its row changed or gone rolls back the whole commit (`crates/pg/src/changes.rs`).
- Postgres enforces read-only connections, and they can't be switched off from inside the app:
  - Browsing, exports and a read-only connection's explains always run in a `READ ONLY` transaction, and so do a read-only connection's commits and imports.
  - On a read-only connection the editor runs each statement in a `READ ONLY` transaction that has already taken its snapshot, after which Postgres refuses to make it read-write.
  - Every piece the editor runs is first described by Postgres, which refuses a piece holding more than one statement, so nothing slips in alongside it (`crates/pg/src/editor.rs`).
  - Extensions that open their own connections, such as dblink, are outside this.
- An agent reaches a database only at its connection's access level, decided in one function (`agents::verdict`), which only the window's commands change; nothing an agent sends changes access, allowances or the pause switch. At Read with approved writes, nothing an agent writes runs before someone approves it in the window, unless they chose "Yes, don't ask again here" for that agent on that connection, which lasts until the app quits and never covers dropping or emptying a table. An agent deletes a Docker database without asking only if it was made for an agent of the same name; any other waits for someone to answer Yes or No in the window. While backups are on, `run_sql` refuses DROP TABLE, SCHEMA, DATABASE, OWNED and MATERIALIZED VIEW, and TRUNCATE (`crates/desktop/src/agents/staging.rs`).
- An agent's queries always run in a `READ ONLY` transaction that has taken its snapshot, on every connection and at every level (`PgDatabase::read`). Its writes run in one transaction on a connection of their own, closed afterwards (`PgDatabase::execute`).
- A column hidden from agents shows as `[hidden]` wherever an agent's query returns it, by the origin Postgres reports for each result column. A query that filters on it or computes with it, and a write that reads it at all, is refused, found in Postgres's own plan (`crates/pg/src/hidden.rs`). This guards against accidental exposure; a view, or a subquery Postgres keeps apart, can still get past it.
- When an agent's `change_rows` commits, its reverse is kept in `agent-undo.json`, deleted rows' values included. Undoing stages that reverse in the table, to review and commit like any edit.
- Every agent call but `wait_for_approval`, which only waits, is appended to `agent-activity.jsonl` before it touches a database, and a call that can't be logged is refused (`crates/desktop/src/agents/activity.rs`).
- `hn-core` does no I/O. Its `Cargo.toml` has no runtime, driver or framework to do it with.
- An SSH tunnel opens only to a host whose key is already in `~/.ssh/known_hosts`.

## Working agreement

- When a decision is Dan's, offer him options and don't pick for him.
- The app working comes first. Judge work by what you can see in the app.
- Don't compile, run the app or run tests on Dan's Mac unless he asks in that session, and stop what you started. Tests run in CI. Biome and the type check take a second and are fine to run.
- Prefer a maintained library over our own code. When you add a dependency, a file or a check, say what it replaces.
- Commits are authored as `notInferred <contact@notinferred.com>`, set in this repository's git config. No commit message, trailer or pull request credits an AI.
- Code reads plainly: names and structure carry the meaning, and a comment only keeps a reason or constraint the code can't show. Formatting is whatever Biome and rustfmt print.
