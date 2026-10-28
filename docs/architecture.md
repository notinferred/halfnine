# Architecture

TableUltra is one desktop process. A Rust core opens the database connections, keeps the secrets and owns the files the app writes. The UI is a React app in the system's own webview (WebKit on macOS, WebView2 on Windows), so there is no bundled browser. The UI reaches the core only through typed Tauri commands, and the window may call only the commands its capability file grants.

## Shape

```
apps/desktop      UI: React + Vite. Calls the core through generated, typed bindings (from M1).
crates/desktop    tu-desktop: the Tauri app. Window, menu, commands, keychain, local store.
crates/pg         tu-pg (M1): the Postgres adapter. Implements core's Database trait; owns quoting and edit SQL.
crates/core       tu-core (M1): pure types and the Database trait. No I/O.
```

From M5, AI agents reach the app through `tableultra mcp`, the same binary started by the agent over stdio. That process holds no logic. It forwards each call over a local socket to the running app, which owns the access check, approvals, the activity log and the database connections.

Dependencies point inward: desktop, then the adapters, then core. Each crate's `Cargo.toml` is what enforces this, because core can't import a library it doesn't depend on. Adding MySQL later means adding a new adapter crate that implements `Database`. Nothing else changes.

Values cross the boundary as Postgres's own text. The app reads every value as text and writes edits back as `$n::text::<column type>`. Postgres's own input and output functions therefore do every type conversion, and every type works, extension types included, without per-type code in the app.

## Tools, and why

| Tool | Why |
|---|---|
| Tauri 2 | It uses the system webview, which gives a binary of about 10 MB and low idle memory, with a Rust core and permissions denied by default. Version 3 is still in alpha |
| React 19, Vite, TypeScript | They have the largest set of maintained components. notinferred uses the same React |
| shadcn (Base UI), Tailwind 4 (M1) | We get accessible behaviour without writing it, and the styling stays ours |
| cmdk, tinykeys, lucide-react (M1) | They give us the palette, shortcuts that are ⌘ on macOS and Ctrl on Windows, and icons |
| RevoGrid (M1, spike) | It is a maintained, virtualized grid with editing and selection built in. TanStack Table + Virtual is the fallback |
| CodeMirror 6 + lang-sql (M2) | It has a Postgres dialect and schema autocomplete, and is about a tenth the size of Monaco |
| sql-formatter (M2) | It formats SQL so we don't have to write a formatter |
| TanStack Query, Zustand (M1) | TanStack Query caches and invalidates calls to the core. Zustand holds tabs and selection |
| tauri-specta (M1) | It generates TypeScript bindings from the Rust commands, and CI fails if the bindings are stale |
| sqlx 0.9, Postgres driver (M1, spike) | It has rustls TLS and a pool, and `raw_sql` returns values as text. `tokio-postgres` is the fallback |
| keyring-core (M1) | It stores passwords in the macOS Keychain and the Windows Credential Manager |
| Tauri plugins: store, window-state, single-instance, dialog, log (M1) | These are small official plugins, so we don't write those features ourselves |
| rmcp (M5, candidate) | It is the official Rust SDK for MCP, the protocol AI agents use to call tools |
| bollard (M5, candidate) | It is a Rust client for the Docker Engine API. It finds, starts, stops and creates local Postgres containers through any Docker socket |

## The engineering invariants

This table checks the app against Dan's `architecture-invariants.md` and `ENGINEERING-CONCEPTS.md`. Each status is one of: **holds**, **partly**, **gap**, **planned** (with its milestone), or **dormant** (it only applies once the app adds the component it needs).

| Invariant | How it applies here | Status |
|---|---|---|
| Resilience | Connects time out after 10 s. Metadata queries have a `statement_timeout`, and TCP keepalive is on. A transient error reconnects on the next action; a terminal one (auth, TLS) is shown to you. Cancel uses `pg_cancel_backend` | planned, M1–M2 |
| Admission control | Each connection pools at most 3 sessions. Browse pages are 500 rows, and editor results stop at 10 000 with a visible notice. Large cells are cut in the grid | planned, M1–M2 |
| Parse, don't validate | Every command's input is a typed Rust struct. A URL is parsed once into `ConnectionSpec`, and each row becomes `Cell` values once | planned, M1 |
| Injection | Identifiers go through one quoting function, and values are always `$n` parameters. Editor SQL is the single deliberate exception, marked `AssertSqlSafe` in one place | planned, M1 |
| Secrets | Passwords live only in the OS keychain. They never go in the store, the logs or a command's reply | planned, M1 |
| Authorization enforced by the server | Read-only connections set `default_transaction_read_only = on`, so Postgres refuses the write. Hiding a button doesn't count | planned, M1 |
| Write identity, optimistic concurrency | An edit is keyed by primary key and also checks the original values. If no rows are affected, that's a conflict and the transaction rolls back. Tables without a key are read-only | planned, M3 |
| Caching | TanStack Query keys data per connection and per table. The schema cache is invalidated by ⌘R and after DDL | planned, M1 |
| Single-owner schema evolution | The local store has a `version`, and the app is its only writer and migrator. Writes are atomic | planned, M1 |
| Observability | `tracing` writes to a local log file, and each query has an id. There is no telemetry, and secrets and rows are never logged | planned, M1 |
| Test fidelity | `tu-pg` is tested against a real Postgres in CI, using hostile fixtures. `tu-core` has pure unit tests. Nothing mocks the database | planned, M1 |
| Least privilege in the window | The capability file denies by default, and the CSP loads nothing from outside the app | holds |
| Pinned dependencies | Versions are exact, published at least two weeks before they were added, with lockfiles. GitHub Actions are pinned to commit SHAs | partly: pnpm enforces the age for every JavaScript package, but Cargo can't for the crates beneath ours |
| One artifact, promoted | A tag builds the installers once, into a draft release that Dan publishes | planned, M4 |
| The eight fallacies | The database connection is the only network boundary. The UI never waits on it, and a dropped connection recovers | planned, M1 |
| A trust boundary for AI agents | Agents are untrusted clients. Each MCP call is parsed at the edge, and the core checks it against that connection's access level, not the MCP process. Results are marked as data, so text in a row can't pass as an instruction. Approvals happen only in the app, and an agent can't raise its own access | planned, M5 |
| Audit trail | Every agent call is appended to a log that is never rewritten. A write keeps the rows it changed, so it can be reverted | planned, M5 |
| Runtime off-switch | One switch pauses all agent access, with no release needed | planned, M5 |
| Idempotent writes | Each change an agent proposes has an id, so approving it twice or retrying it applies it once | planned, M5 |
| Admission control for agents | Each agent query has a time limit and a row cap, and a sensitive column is masked before any result leaves the app | planned, M5 |
| Single-owner schema evolution, in projects | In a project that has migrations, an agent's schema change becomes a migration file, so the project stays the only owner of its schema | planned, M5 |
| Brokers, locks, multiple regions, a public API | The app has none of these. MCP is local, between processes on one machine | dormant |
