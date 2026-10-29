# Architecture

Halfnine is one desktop process. A Rust core opens the database connections, keeps the secrets and owns the files the app writes. The UI is a React app in the system's own webview (WebKit on macOS, WebView2 on Windows), so there is no bundled browser. The UI reaches the core only through typed Tauri commands, and the window may call only the commands its capability file grants.

## Shape

```
apps/desktop      UI: React + Vite. Calls the core through bindings generated from the Rust commands (src/ipc/bindings.ts).
crates/desktop    hn-desktop: the Tauri app. Window, commands, keychain, saved connections, open sessions.
crates/pg         hn-pg: the Postgres adapter. Implements core's Database trait; owns quoting, the SQL it builds, and SSH tunnels.
crates/core       hn-core: pure types and the Database trait. No I/O.
```

From M5, AI agents reach the app through `halfnine mcp`, the same binary started by the agent over stdio. That process holds no logic. It forwards each call over a local socket to the running app, which owns the access check, approvals, the activity log and the database connections.

Dependencies point inward: desktop, then the adapters, then core. Each crate's `Cargo.toml` enforces this, because core can't import a library it doesn't depend on. Adding MySQL later means adding a new adapter crate that implements `Database`. Nothing else changes.

Values cross the boundary as Postgres's own text. A page selects every column as `left(column::text, 65537)`, and following a key compares `column = $n::text::<column type>`. The editor gets text another way. Postgres first describes each statement, which gives its columns even when it returns no rows. The statement then runs in the simple protocol, where Postgres sends every value as text. The editor keeps its own connection, apart from the pool, so `SET`, `BEGIN … COMMIT` and temporary tables last from one run to the next. Postgres's own input and output functions therefore do every type conversion, and every type works, extension types included, without per-type code in the app. Edits use the same cast: a staged value is written as `$n::text::<column type>`. An import streams the file through Postgres's own CSV reader into a temporary table of text columns, then casts each column on its way into the table.

## Tools, and why

| Tool | Why |
|---|---|
| Tauri 2 | It uses the system webview, which gives a binary of about 10 MB and low idle memory, with a Rust core and permissions denied by default. Version 3 is still in alpha |
| React 19, Vite, TypeScript | They have the largest set of maintained components. notinferred uses the same React |
| Native controls and plain CSS on our tokens | Native inputs and selects look and behave like the system's, and the approved mockup needed nothing more. This replaced the planned shadcn and Tailwind: two fewer dependencies and no copied components. Base UI comes in when a control native HTML lacks is needed, such as a menu |
| cmdk, tinykeys, lucide-react | They give us the palette, shortcuts that are ⌘ on macOS and Ctrl on Windows, and icons |
| RevoGrid | It is a maintained, virtualized grid with editing, range selection and copying built in, themed through its CSS variables. Dan kept it after using M1 |
| CodeMirror 6 + lang-sql | It has a Postgres dialect, including dollar-quoted bodies, and schema autocomplete, and is about a tenth the size of Monaco. Its parser splits the editor into statements, and Postgres checks each piece again |
| sql-formatter | It formats SQL so we don't have to write a formatter |
| tauri-plugin-dialog, csv | The save dialog for exports, opened from Rust so a file path never comes from the UI. The `csv` crate quotes CSV correctly |
| TanStack Query, Zustand | TanStack Query caches and invalidates calls to the core. Zustand holds the session, tabs and palette |
| tauri-specta 2.0.0-rc.25 | It generates TypeScript bindings from the Rust commands. A test regenerates them, and CI fails if the committed file differs |
| sqlx 0.9, Postgres driver | It has a pool and rustls TLS verified against the system's certificates. Browsing casts every value to text in the SQL it builds. The editor uses `prepare` to describe a statement, then `raw_sql` to run it as text. That passed the M2 spike, so `tokio-postgres` isn't needed |
| russh | It runs SSH tunnels in-process, trusting only host keys already in `~/.ssh/known_hosts`. It uses `ring` rather than its default `aws-lc-rs`, which needs a C toolchain |
| keyring-core, with the Apple and Windows stores | It stores passwords in the macOS Keychain and the Windows Credential Manager |
| Tauri plugins: single-instance, window-state, log | A second launch focuses the open window, the window reopens where it was, and logs go to the app's log folder |
| rmcp (M5, candidate) | It is the official Rust SDK for MCP, the protocol AI agents use to call tools |
| bollard (M5, candidate) | It is a Rust client for the Docker Engine API. It finds, starts, stops and creates local Postgres containers through any Docker socket |

## The engineering invariants

This table checks the app against Dan's `architecture-invariants.md` and `ENGINEERING-CONCEPTS.md`. Each status is one of: **holds**, **partly**, **gap**, **planned** (with its milestone), or **dormant** (it only applies once the app adds the component it needs).

| Invariant | How it applies here | Status |
|---|---|---|
| Resilience | Connects and SSH time out after 10 s, and browsing sets `statement_timeout = 30s`. The editor, explains and exports have no time limit and stop on Cancel, which signals their server process with `pg_cancel_backend`. The pool checks a connection before lending it, and the editor's own connection is replaced after a network error, so a dropped one is replaced on its next use. A browse that fails on the network or a timeout is retried once; a terminal error (auth, TLS) is shown with its code | partly: there is no TCP keepalive |
| Admission control | Each connection pools at most 3 sessions, plus the editor's own. Pages are 500 rows and the server caps a page at 1,000. The editor keeps the first 10,000 rows of a result and counts the rest, while an export streams every row in constant memory. Cells over 64 KB are cut and marked. A connection runs one editor run at a time | holds |
| Parse, don't validate | Every command's input is a typed Rust struct. A pasted URL is parsed once (`hn_core::parse_url`), a draft connection is checked once (`spec_from`), and each row becomes `Cell` values once | holds |
| Injection | Identifiers go through `sql::ident` and values are always `$n` parameters. Everything the app builds runs through `PgDatabase::query`, the one place marked `AssertSqlSafe`, except an import's `COPY`, which has its own protocol and is fixed text. The only text taken as SQL is what someone typed for their own table: a WHERE condition, or a column's type or default. Postgres runs each as part of one prepared statement, refusing a second statement. Review SQL is written in the same pass as the statement that runs, with the values written in | holds |
| Secrets | Saved passwords and SSH secrets live together in one OS keychain entry, read at most once per launch (the first time one is needed) and then kept in memory until the app quits, so macOS asks at most once. A connection set to ask each time asks when it opens and remembers what was typed until the app quits. Choosing Ask each time removes the saved password only when the form is saved. As TablePlus, FileZilla and DataGrip do it. The store, the logs and every command's reply carry nothing about them. macOS asks before a build that didn't save a secret reads it, and an unsigned build is a new app to macOS each time it is built | holds |
| Authorization enforced by the server | Browsing, exports and read-only explains run in `READ ONLY` transactions. Commits and imports run in a transaction that Postgres makes read-only on a read-only connection, which the window also doesn't offer to edit. On a read-only connection, every editor statement runs in its own `READ ONLY` transaction, which has taken its snapshot first, so Postgres refuses to make it read-write. Postgres describes each piece first and refuses one holding two statements. Tests try `SET`, `COMMIT`, `set_config` and `SESSION CHARACTERISTICS`, and each is refused. Stopping other sessions is refused on read-only connections, by the app | holds, except through extensions that open their own connections, such as dblink |
| Write identity, optimistic concurrency | An edit is keyed by primary key and also checks that each edited cell still prints as it did when loaded (`column::text = $n`, which works for every type). An update or delete that changes no row is a conflict: the whole commit rolls back and names the row. Tables without a key are read-only, and the status bar says why | holds: tested against Postgres 18 on 2026-10-28 |
| Caching | Every query key holds its session, so ⌘R refreshes everything that session loaded. Nothing refetches on focus or a timer. After an editor run that did more than read, everything for the session is refreshed | holds |
| Single-owner schema evolution | `connections.json` has a `version`. The app refuses one from a newer version, writes it to a temporary file and renames it over the old one, and the single-instance plugin keeps a second copy from writing at the same time | holds |
| Observability | The log plugin writes to the app's log folder. Each editor run has an id, which is logged with its statement count, time and outcome, and kept in history. Nothing logged holds a secret, a row or SQL text. There is no telemetry | holds |
| Test fidelity | `hn-core` and the SQL builder have unit tests. `hn-pg` is tested against Postgres 18 loaded with `crates/pg/tests/fixture.sql`, which has hostile names, every common type and a 70 KB value. Nothing mocks the database | holds: all 50 Rust tests passed on 2026-10-28, 23 of them against Postgres 18; CI runs them on every push once the repository is on GitHub |
| Least privilege in the window | The capability file denies by default, and the CSP loads nothing from outside the app | holds |
| Pinned dependencies | Versions are exact, published at least two weeks before they were added, with lockfiles. GitHub Actions are pinned to commit SHAs | partly: pnpm enforces the age for every JavaScript package, but Cargo can't for the crates beneath ours |
| One artifact, promoted | A tag builds the installers once, into a draft release that Dan publishes | planned, M4 |
| The eight fallacies | The database connection, or its SSH tunnel, is the only network boundary. Every command is async, so the window never waits on the network. A dropped connection is replaced, and a long statement can be cancelled | partly: no keepalive |
| A trust boundary for AI agents | Agents are untrusted clients. Each MCP call is parsed at the edge, and the core checks it against that connection's access level, not the MCP process. Results are marked as data, so text in a row can't pass as an instruction. Approvals happen only in the app, and an agent can't raise its own access | planned, M5 |
| Audit trail | Every agent call is appended to a log that is never rewritten. A write keeps the rows it changed, so it can be reverted | planned, M5 |
| Runtime off-switch | One switch pauses all agent access, with no release needed | planned, M5 |
| Idempotent writes | Each change an agent proposes has an id, so approving it twice or retrying it applies it once | planned, M5 |
| Admission control for agents | Each agent query has a time limit and a row cap, and a sensitive column is masked before any result leaves the app | planned, M5 |
| Single-owner schema evolution, in projects | In a project that has migrations, an agent's schema change becomes a migration file, so the project stays the only owner of its schema | planned, M5 |
| Brokers, locks, multiple regions, a public API | The app has none of these. MCP is local, between processes on one machine | dormant |
