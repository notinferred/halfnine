# Roadmap

## Where things stand (2026-10-28)

M0 and M1 are done:

- Dan used the app and kept RevoGrid.
- All 21 Rust tests passed, the Postgres ones against Postgres 18.6 loaded with the test fixture, run once on Dan's Mac at his request.
- The repository isn't on GitHub yet, so CI hasn't run. M2 is next.

The scope grew on 2026-10-28. M1–M3 now include the everyday features of a database client, foreign-key navigation among them. M5 makes TableUltra the database tool AI agents use, and adds local Docker databases.

## M0: Foundation

- [x] Repository, workspace, pinned toolchains, Biome and rustfmt
- [x] `check.yml`
- [x] Design tokens for light, dark, increased contrast and reduced motion, plus the welcome window
- [x] An icon that doesn't depend on the name, drawn once and rendered by `pnpm icons`
- [x] Design pass: Dan approved the mockup of Connections, Table, Query and the palette (2026-10-28). The tokens match it
- [x] Rust installed through Homebrew, and `Cargo.lock` committed
- [x] First compile and run on macOS (2026-10-28): it compiles in 21 seconds and opens the window. The screen itself hasn't been looked at yet

## M1: Connect and browse

### Spikes

- [x] **RevoGrid.** Kept on 2026-10-28, after Dan used the app ("Looks great lets keep revogrid"). It stays only if, styled with our tokens, it does all of the following:
  - matches the mockup in light and dark;
  - scrolls 100k rows smoothly;
  - has resizable, sortable headers;
  - handles keyboard navigation and copying as TSV.

  Editing a cell with NULL kept apart from empty is judged in M3, when editing exists.
- [x] **sqlx.** Browsing needs no `raw_sql`, because the app casts every column to text itself, so this spike moved to M2, where the editor runs SQL as written.

### Built

- [x] **Connections:**
  - A form, or a pasted `postgres://` URL, with a connection test.
  - TLS modes.
  - The password kept in the OS keychain.
  - Colour labels and a read-only toggle.
  - Save, delete (with a second click to confirm), and double-click to connect.
- [x] **SSH tunnels:** an SSH agent, a key file or a password, trusting only hosts already in `~/.ssh/known_hosts`. Not yet tried against a real SSH server.
- [x] **Browsing a server:**
  - Switch between the databases on the server.
  - The sidebar has a filter and lists tables, views, materialized views, partitioned and foreign tables, functions, sequences and enum types.
- [x] **Table tabs:**
  - A Data view: columns that hide, reorder and resize; server-side sort; a typed WHERE filter (⌘F); pages of 500 that load as you scroll; an estimated row count, with an exact count on request.
  - A Structure view.
- [x] **Foreign keys you can follow:**
  - A key cell has an arrow to the row it points at.
  - The row panel lists every key out of the row and every table pointing into it.
  - Each one opens a filtered tab.
- [x] **Row detail panel:** one row shown vertically, with JSON pretty-printed, text in full up to 64 KB, `bytea` shown as size and hex, and NULL shown as a word.
- [x] **The ⌘K palette:** go to any table or view. Also ⌘R to refresh and ⌘W to close a tab.
- [x] **New crates:** `crates/core` holds pure types and the `Database` trait. `crates/pg` is the Postgres adapter.
- [x] **CI:** `check.yml` gains a Postgres 18 service loaded with `crates/pg/tests/fixture.sql`, the tests, and a check that the bindings are current.

### The M1 check (done 2026-10-28)

Dan used the app against the throwaway check database:

1. Connect by pasting its URL.
2. Browse `tu_fixture.invoices` (120,001 rows) smoothly, sort it, and filter it.
3. Follow `customer_id` to its customer, and back through "Referenced by".
4. Turn on read-only, then filter `customers` with `tu_fixture.touch()`. Postgres refuses with code 25006. The test `read_only_connections_are_refused_by_the_server` proves this step. It first found that the fixture's `touch()` named its table without the schema, which is now fixed.

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
