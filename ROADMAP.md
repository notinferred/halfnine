# Roadmap

## Where things stand (2026-10-28)

M0 is nearly done:

- The repository, pinned toolchains, lockfiles, CI check, design tokens and icon exist, and Dan approved the design.
- The window is written to open on a welcome screen, but it hasn't been compiled yet. Rust is installed on Dan's Mac, and the first `pnpm dev` waits for his go-ahead.
- The repository isn't on GitHub yet, so CI hasn't run.

The scope grew on 2026-10-28. M1–M3 now include the everyday features of a database client, foreign-key navigation among them. M5 makes TableUltra the database tool AI agents use, and adds local Docker databases.

## M0: Foundation

- [x] Repository, workspace, pinned toolchains, Biome and rustfmt
- [x] `check.yml`
- [x] Design tokens for light, dark, increased contrast and reduced motion, plus the welcome window
- [x] An icon that doesn't depend on the name, drawn once and rendered by `pnpm icons`
- [x] Design pass: Dan approved the mockup of Connections, Table, Query and the palette (2026-10-28). The tokens match it
- [x] Rust installed through Homebrew, and `Cargo.lock` committed
- [ ] First compile and run on macOS, when Dan says to

## M1: Connect and browse

Two spikes come first, about a day each:

1. **RevoGrid.** It stays only if, styled with our tokens, it does all of the following:
   - matches the mockup in light and dark;
   - scrolls 100k rows smoothly;
   - edits a cell with NULL kept apart from empty;
   - handles keyboard navigation and copying as TSV;
   - has resizable, sortable headers.
   
   Otherwise TanStack Table and TanStack Virtual replace it.
2. **sqlx `raw_sql`.** It stays only if, for a multi-statement script that includes extension types, it returns text values and type names for each statement. Otherwise `tokio-postgres` `simple_query` replaces it.

Then:

- **Connections:**
  - A form, or a pasted `postgres://` URL, with a connection test.
  - TLS modes.
  - **SSH tunnels**: host, user, and a key or agent.
  - The password kept in the OS keychain.
  - Colour labels and a read-only toggle.
- **Browsing a server:** switch between the databases on the server. The sidebar has a filter and lists tables, views, materialized views, functions, sequences and enum types.
- **Table tabs:**
  - A Data view: column hide, reorder and resize; sort; filter.
  - A Structure view.
- **Foreign keys you can follow:**
  - A key cell links to the row it points at.
  - A row lists the rows in other tables that point at it.
- **Row detail panel:** one row shown vertically, with JSON pretty-printed, long text in full and `bytea` shown as size and hex.
- **The ⌘K palette.**
- **New crates:** `crates/core` holds pure types and the `Database` trait and does no I/O. `crates/pg` is the Postgres adapter.
- **CI:** `check.yml` gains a Postgres service, the tests and the tauri-specta bindings check.

These guarantees move to CLAUDE.md once the code enforces them:

- Passwords live only in the OS keychain.
- Values are always bound parameters, and identifiers always go through one quoting function.
- The server enforces read-only connections.
- `tu-core` does no I/O.

## M2: Query

- An editor with schema autocomplete.
- Run and cancel, and multiple result sets.
- History, **saved queries**, formatting, copying and export.
- **EXPLAIN view:** a query plan as a readable tree, with ANALYZE when asked.
- **Sessions view:** what is running on the server (`pg_stat_activity`), with cancel and terminate.

## M3: Edit

- Staged edits, reviewed as SQL before they run.
- One transaction per commit, with conflicts detected.
- Insert, delete and filters.
- **Key-aware editing:** setting a foreign-key cell picks a row from the referenced table, with search.
- **Editors for big values:** JSON and JSONB, and multi-line text.
- **CSV import** into a table: map the columns, preview, then import in one transaction.
- **Structure editing:** create a table, add, change or drop columns, and add indexes. All of it goes through Review SQL.
- **Truncate and drop** from the sidebar, after typing the table's name to confirm.

## M4: v0.1

- `release.yml` builds an unsigned `.dmg` (Apple silicon) and a per-user `-setup.exe` (x64) into a draft GitHub Release whenever a version tag is pushed.
- The final icon and the README's install steps.
- **Measured budgets:** an installer under 15 MB, a window ready in under 1 second, and idle memory recorded on both systems.

## M5: The database tool for AI agents, and local Docker databases

An agent that needs a table, a query answered, or a fresh database asks TableUltra over MCP instead of asking Dan. Docker databases on the machine show up and are managed like any other connection.

### Agents

- **The connection: MCP.** `tableultra mcp` is the same app binary, started by the agent and talking to it over stdio. It works with any MCP client: Claude Code, Claude Desktop, Cursor, Codex, VS Code.
  - It talks to the running app over a local socket, and opens the app if it is closed, so approvals and the activity log always live in the app.
  - A "Connect an agent" screen sets up each client.
  - The keypaste MCP bridge is the precedent.
- **What an agent can do:**
  - List connections, by name and label only.
  - Describe a schema: tables, columns, keys, the foreign-key graph and row estimates.
  - Read sample rows, and run queries.
  - Propose changes.
  - Create, copy and delete local databases.
- **Access per connection:** each connection is set to Off, Read, Read with approved writes, or Full.
  - Defaults: databases TableUltra created in Docker are Full. Red-labelled connections are Off. Every other connection is Read.
  - A read-only connection stays read-only whatever its level.
- **Approvals:**
  - A write that needs approval appears in the app as staged SQL, in the same Review SQL view, named with the agent that sent it. Dan approves or rejects it there, and a notification appears if the app is in the background.
  - Approval happens only in the app, never through the MCP channel. An agent can't change its own access.
- **Activity log and undo:**
  - Every agent call is logged: the agent, the connection, the SQL, the rows touched and the time it took.
  - Before an agent's write runs, the rows it changes are saved, so the write can be reverted from the log.
  - Undo covers row changes. Schema changes go through migrations or copies.
- **Migrations, not direct DDL:** when the agent's project has migrations (Drizzle, Prisma, sqlx and others, found through the project folder the agent shares by MCP roots), a schema change becomes a migration file in that project rather than a direct change to the database. Direct DDL is only for databases nothing owns, such as a scratch Docker database.
- **Work on a copy:** clone a database before risky work. On a local server this is `CREATE DATABASE … TEMPLATE`; for Docker, a cloned container. The agent works on the copy, and Dan keeps it or discards it.
- **Sensitive columns hidden:**
  - Columns marked sensitive are masked in everything an agent sees.
  - TableUltra suggests columns to mark from their names: email, phone, token, password, secret.
  - Whatever an agent reads is sent to its AI provider, which is why this matters.
- **Credentials:**
  - An agent may receive the URL of a Docker database TableUltra created, because those credentials are local and throwaway, and the project it's building may need them.
  - Every other connection's credentials never leave the app.
- **One switch pauses all agent access.**

### Docker

- **See and connect:** it lists the Postgres containers on this machine and connects in a click, reading the user, password and database from the container's environment. It works with any Docker engine socket: Docker Desktop, OrbStack or Colima.
- **Start, stop and delete:** deleting a container offers to delete its data volume too, after a confirmation.
- **Create:**
  - Choose a Postgres version and a name. The port is picked automatically.
  - Optionally load a dump or an SQL file.
  - Each database gets its own volume, and TableUltra labels its containers so it knows which are its own.
- **Agents create their own:** an agent can ask for a fresh database and get one without asking Dan. It shows in the sidebar marked with the agent's name.

### Candidates, checked when M5 starts

The official MCP Rust SDK (`rmcp`) and the Docker Engine API client `bollard`. Each passes the same adoption test as every other dependency, with the same two-week age rule.

## Next, in the order Dan chooses

- An auto-updater.
- Multiple windows.
- Dump and restore for remote servers.
- MySQL and SQLite through `Database`.
- Signing.
- Homebrew and winget.
- A website.
- The final name.

## Not planned

- A tray icon or a global hotkey: Dan chose a normal app. An agent opens the app when it needs an approval.
- Cloudflare: GitHub only for now.
- Forking an existing client: Dan chose a fresh build.
