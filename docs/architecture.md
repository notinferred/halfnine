# Architecture

TableUltra is one desktop process. A Rust core opens the database connections, keeps the secrets and owns the files the app writes. The UI is a React app in the system's own webview (WebKit on macOS, WebView2 on Windows), so there is no bundled browser. The UI reaches the core only through typed Tauri commands, and the window may call only the commands its capability file grants.

## Shape

```
apps/desktop      UI: React + Vite. Calls the core through bindings generated from the Rust commands (src/ipc/bindings.ts).
crates/desktop    tu-desktop: the Tauri app. Window, commands, keychain, saved connections, open sessions.
crates/pg         tu-pg: the Postgres adapter. Implements core's Database trait; owns quoting, the SQL it builds, and SSH tunnels.
crates/core       tu-core: pure types and the Database trait. No I/O.
```

From M5, AI agents reach the app through `tableultra mcp`, the same binary started by the agent over stdio. That process holds no logic. It forwards each call over a local socket to the running app, which owns the access check, approvals, the activity log and the database connections.

Dependencies point inward: desktop, then the adapters, then core. Each crate's `Cargo.toml` enforces this, because core can't import a library it doesn't depend on. Adding MySQL later means adding a new adapter crate that implements `Database`. Nothing else changes.

Values cross the boundary as Postgres's own text. A page selects every column as `left(column::text, 65537)`, and following a key compares `column = $n::text::<column type>`. Postgres's own input and output functions therefore do every type conversion, and every type works, extension types included, without per-type code in the app. M3's edits will use the same cast.

## Tools, and why

| Tool | Why |
|---|---|
| Tauri 2 | It uses the system webview, which gives a binary of about 10 MB and low idle memory, with a Rust core and permissions denied by default. Version 3 is still in alpha |
| React 19, Vite, TypeScript | They have the largest set of maintained components. notinferred uses the same React |
| Native controls and plain CSS on our tokens | Native inputs and selects look and behave like the system's, and the approved mockup needed nothing more. This replaced the planned shadcn and Tailwind: two fewer dependencies and no copied components. Base UI comes in when a control native HTML lacks is needed, such as a menu |
| cmdk, tinykeys, lucide-react | They give us the palette, shortcuts that are ⌘ on macOS and Ctrl on Windows, and icons |
| RevoGrid | It is a maintained, virtualized grid with editing, range selection and copying built in, themed through its CSS variables. Dan kept it after using M1 |
| CodeMirror 6 + lang-sql (M2) | It has a Postgres dialect and schema autocomplete, and is about a tenth the size of Monaco |
| sql-formatter (M2) | It formats SQL so we don't have to write a formatter |
| TanStack Query, Zustand | TanStack Query caches and invalidates calls to the core. Zustand holds the session, tabs and palette |
| tauri-specta 2.0.0-rc.25 | It generates TypeScript bindings from the Rust commands. A test regenerates them, and CI fails if the committed file differs |
| sqlx 0.9, Postgres driver | It has a pool and rustls TLS verified against the system's certificates. Browsing needs no `raw_sql`, because every value is cast to text in the SQL the app builds, so the `raw_sql` spike moved to M2, where the editor needs it. `tokio-postgres` is still the fallback |
| russh | It runs SSH tunnels in-process, trusting only host keys already in `~/.ssh/known_hosts`. It uses `ring` rather than its default `aws-lc-rs`, which needs a C toolchain |
| keyring-core, with the Apple and Windows stores | It stores passwords in the macOS Keychain and the Windows Credential Manager |
| Tauri plugins: single-instance, window-state, log | A second launch focuses the open window, the window reopens where it was, and logs go to the app's log folder |
| rmcp (M5, candidate) | It is the official Rust SDK for MCP, the protocol AI agents use to call tools |
| bollard (M5, candidate) | It is a Rust client for the Docker Engine API. It finds, starts, stops and creates local Postgres containers through any Docker socket |

## The engineering invariants

This table checks the app against Dan's `architecture-invariants.md` and `ENGINEERING-CONCEPTS.md`. Each status is one of: **holds**, **partly**, **gap**, **planned** (with its milestone), or **dormant** (it only applies once the app adds the component it needs).

| Invariant | How it applies here | Status |
|---|---|---|
| Resilience | Connects and SSH time out after 10 s, and every session sets `statement_timeout = 30s`. The pool checks a connection before lending it, so a dropped one is replaced on its next use. A network error or timeout is retried once; a terminal one (auth, TLS) is shown with its code. Cancel arrives with the editor | partly: cancel is M2, and there is no TCP keepalive |
| Admission control | Each connection pools at most 3 sessions. Pages are 500 rows and the server caps a page at 1,000. Cells over 64 KB are cut and marked. The editor's 10,000-row cap is M2 | partly: the editor is M2 |
| Parse, don't validate | Every command's input is a typed Rust struct. A pasted URL is parsed once (`tu_core::parse_url`), a draft connection is checked once (`spec_from`), and each row becomes `Cell` values once | holds |
| Injection | Identifiers go through `sql::ident` and values are always `$n` parameters. Everything the app builds runs through `PgDatabase::query`, the one place marked `AssertSqlSafe`. The only text taken as SQL is a WHERE condition someone typed, which Postgres runs as one prepared statement, refusing a second statement | holds |
| Secrets | Passwords and SSH secrets live only in the OS keychain. The store, the logs and every command's reply carry at most whether a secret exists | holds |
| Authorization enforced by the server | A read-only connection starts every session with `default_transaction_read_only = on`, so Postgres refuses writes, including ones hidden in a function | holds for M1. In M2, SQL typed into the editor could switch the setting off, and M2 must decide how to stop that |
| Write identity, optimistic concurrency | An edit is keyed by primary key and also checks the original values. If no rows are affected, that's a conflict and the transaction rolls back. Tables without a key are read-only | planned, M3 |
| Caching | Every query key holds its session, so ⌘R refreshes everything that session loaded. Nothing refetches on focus or a timer. DDL from the editor will invalidate the schema in M2 | partly: DDL is M2 |
| Single-owner schema evolution | `connections.json` has a `version`. The app refuses one from a newer version, writes it to a temporary file and renames it over the old one, and the single-instance plugin keeps a second copy from writing at the same time | holds |
| Observability | The log plugin writes to the app's log folder, and nothing is logged that holds a secret or a row. There is no telemetry. Query ids arrive with history in M2 | partly |
| Test fidelity | `tu-core` and the SQL builder have unit tests. `tu-pg` is tested against Postgres 18 loaded with `crates/pg/tests/fixture.sql`, which has hostile names, every common type and a 70 KB value. Nothing mocks the database | holds: all 21 passed against Postgres 18.6 on 2026-10-28; CI runs them on every push once the repository is on GitHub |
| Least privilege in the window | The capability file denies by default, and the CSP loads nothing from outside the app | holds |
| Pinned dependencies | Versions are exact, published at least two weeks before they were added, with lockfiles. GitHub Actions are pinned to commit SHAs | partly: pnpm enforces the age for every JavaScript package, but Cargo can't for the crates beneath ours |
| One artifact, promoted | A tag builds the installers once, into a draft release that Dan publishes | planned, M4 |
| The eight fallacies | The database connection, or its SSH tunnel, is the only network boundary. Every command is async, so the window never waits on the network, and a dropped connection is replaced | partly: no keepalive, no cancel yet |
| A trust boundary for AI agents | Agents are untrusted clients. Each MCP call is parsed at the edge, and the core checks it against that connection's access level, not the MCP process. Results are marked as data, so text in a row can't pass as an instruction. Approvals happen only in the app, and an agent can't raise its own access | planned, M5 |
| Audit trail | Every agent call is appended to a log that is never rewritten. A write keeps the rows it changed, so it can be reverted | planned, M5 |
| Runtime off-switch | One switch pauses all agent access, with no release needed | planned, M5 |
| Idempotent writes | Each change an agent proposes has an id, so approving it twice or retrying it applies it once | planned, M5 |
| Admission control for agents | Each agent query has a time limit and a row cap, and a sensitive column is masked before any result leaves the app | planned, M5 |
| Single-owner schema evolution, in projects | In a project that has migrations, an agent's schema change becomes a migration file, so the project stays the only owner of its schema | planned, M5 |
| Brokers, locks, multiple regions, a public API | The app has none of these. MCP is local, between processes on one machine | dormant |
