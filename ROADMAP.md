# Roadmap

## Where things stand (2026-10-29)

M0 to M3 are done. M4 is done on macOS: v0.1.0 is published, and the installed app opens in under half a second and idles at about 86 MB. Windows is still to be measured. The first step of M5, the agent core, is built and approved, and is on main:

- Agents such as Claude Code reach Halfnine over MCP through `halfnine mcp`, read through read-only transactions, and propose changes Dan approves in the table itself.
- CI passed: lint, types, the build, and all 86 Rust tests, the agent tests against Postgres 18 included.
- v0.1.0 is published, with a 3.3 MB macOS installer and a 3.0 MB Windows one. It predates the agent core, which comes in the next release. The code is licensed under PolyForm Shield 1.0.0 from the first commit.

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
2. Browse `hn_fixture.invoices` (120,001 rows) smoothly, sort it, and filter it.
3. Follow `customer_id` to its customer, and back through "Referenced by".
4. Turn on read-only, then filter `customers` with `hn_fixture.touch()`. Postgres refuses with code 25006. The test `read_only_connections_are_refused_by_the_server` proves this step. It first found that the fixture's `touch()` named its table without the schema, which is now fixed.

## M2: Query

- [x] **sqlx spike:** `prepare` describes each statement, and `raw_sql` returns its values as text, extension types included. Kept.
- [x] **The editor:**
  - CodeMirror with the Postgres dialect, and autocomplete for every schema, table and column.
  - Query tabs (⌘T) that keep their text when you switch away.
- [x] **Run:**
  - ⌘↵ runs the statement at the cursor or the selection, and ⇧⌘↵ runs everything.
  - Several result sets, each in its own tab, with a Messages tab. Running stops at the first error, which shows its SQLSTATE.
  - The first 10,000 rows are kept, and the rest counted.
  - A note when a transaction is left open.
  - Cancel (⌘.).
- [x] **Read-only stays read-only in the editor.** Each statement gets its own read-only transaction, after a snapshot, and Postgres verifies that each piece is a single statement. Browsing is read-only on every connection.
- [x] **History and saved queries.**
  - History keeps the last 500 runs, and the palette shows recent runs on this connection.
  - Save with ⌘S.
  - Format with ⇧⌘F, using sql-formatter.
- [x] **Copy and export:**
  - Results copy as TSV, CSV or JSON.
  - Tables copy loaded rows as CSV, JSON or INSERT statements.
  - Export streams a whole result or table to CSV or JSON, always read-only.
- [x] **EXPLAIN view:** the plan as a tree. Analyze runs the statement, shows the time for each step, then rolls back.
- [x] **Sessions view:** `pg_stat_activity`, with Cancel and End, each confirmed. Read-only connections can't stop other sessions. Opened from the palette.
- [x] **The M2 check (2026-10-28):** Dan ran queries against the check database, and all 35 tests passed against it. Dan hasn't yet reported a re-check of ⌘. or of Analyze after their fixes.
- Not yet: notices raised by `RAISE NOTICE` aren't shown, because sqlx doesn't pass them on. The split between editor and results doesn't resize.

## M3: Edit

- [x] **Staged edits.** Cells edit in place (Enter, a double-click, or typing). Staged cells take the accent, as in the mockup, and a bar at the foot counts them, with Review SQL, Discard and Commit (⌘S). Tabs and the Data and Structure switch show a dot while something is staged. A tab with staged changes, or a connection left with some, is held open until they are committed or discarded.
- [x] **NULL kept apart from empty** (RevoGrid's last spike criterion). Delete or Backspace sets the selected cells to NULL. An edit that leaves an empty cell empty changes nothing, so NULL never becomes empty text by accident. The row panel shows NULL in its empty fields and sets it with Set NULL.
- [x] **One transaction per commit, with conflicts detected.** Each update is keyed by primary key and checks that the edited cells still print as they did when loaded. A row that changed or vanished rolls back everything and is named in the error.
- [x] **Insert and delete.** + adds a new row above the others, whose empty cells take their defaults. ⌘⌫ deletes the selected rows, shown struck through until the commit. The row panel deletes and keeps rows too.
- [x] **Key-aware editing:** editing a single-column foreign key opens a picker of the referenced rows, searched by key or by their first text column.
- [x] **Editors for big values:** JSON, JSONB, XML, multi-line text and values over 200 characters open in the row panel, which edits each value whole. JSON is checked as you type.
- [x] **CSV import**, from the Copy and export menu: each column of the file goes into a chosen column (matched by name to start with), with its first values shown. Postgres's own CSV reader tells NULL (an empty field) from empty text (`""`). Every row goes in, or none does.
- [x] **Structure editing:** the Structure view edits columns (name, type, null, default), adds and drops columns and indexes, and commits through Review SQL in one transaction. **New table** comes from the + beside a schema in the sidebar, or from the palette, and starts with an identity key.
- [x] **Truncate and drop** from a table's ⋯ menu in the sidebar, after typing its name. The statement is shown first.
- [x] **Commits go into History**, as the statements that ran.
- [x] **Passwords, after Dan's M3 check:** a menu beside Password, Save in Keychain or Ask each time, as in TablePlus. Saved passwords share one Keychain entry, read at most once per launch. Typed ones are remembered until the app quits. Nothing is removed until the form is saved.
- [x] **The M3 check (2026-10-28):** all 50 tests passed against the check database, and Dan used it: "Ok its good though".

## M4: v0.1

- [x] **The name:** Halfnine, chosen by Dan before v0.1, since the app ID it carries stays fixed from then on.
- [x] **`release.yml`:** a version tag builds an unsigned `.dmg` (Apple silicon) and a per-user `-setup.exe` (x64) into a draft GitHub Release. tauri-action is pinned to v1.0.0's commit.
- [x] **The README's install steps**, which the release notes repeat.
- [x] **License:** PolyForm Shield 1.0.0, copyright notInferred, in every commit since the first. The repository's description leads with AI agents, marked as coming, and its topics are ones people browse.
- [x] **The first draft release** from the tag `v0.1.0` (2026-10-28): `Halfnine_0.1.0_aarch64.dmg` and `Halfnine_0.1.0_x64-setup.exe`. It waits for Dan to publish it.
- [x] **Published** v0.1.0 (2026-10-29).
- [x] **Measured budgets on macOS** (2026-10-29), v0.1.0 installed from the release on an Apple silicon Mac:
  - The installer is under 15 MB: 3.3 MB for macOS and 3.0 MB for Windows. Installed, the app is 6.8 MB.
  - The window is on screen 390 ms after the first launch, and about 200 ms after later ones. This is measured to the window appearing; its page draws just after.
  - Idle memory is about 86 MB in all, by macOS's physical footprint, 20 seconds after launch on the connections screen: the app 26 MB, and its WebKit processes 43, 13 and 4 MB.
- [ ] **Measured budgets on Windows:** the same two numbers, from a Windows machine.
- **Keychain prompts after an update.** An unsigned release is a new app to macOS, so after each update macOS asks once before Halfnine reads a saved password. The README says to choose Always Allow. Signing (under Next) ends it.
- The icon stays as drawn in M0; it never depended on the name.

## M5: The database tool for AI agents, and local Docker databases

An agent that needs a table, a query answered, or a fresh database asks Halfnine over MCP instead of asking Dan. Docker databases on the machine show up and are managed like any other connection.

**When M5 ships:** drop "(coming soon)" from the repository's description, add the topics mcp, mcp-server, ai-agents and claude-code, and list Halfnine where people look for MCP servers: the official MCP Registry, Smithery, mcp.so and awesome-mcp-servers.

### The agent core (done 2026-10-29)

- [x] **`halfnine mcp`:** the same binary, started by any MCP client, relaying to the running app over a socket only this account can open, and opening the app in the background if it is closed.
- [x] **Seven tools:** `list_connections` (names, labels and levels only, Off ones included), `describe`, `query` (always read-only, with time and row limits), `change_rows`, `change_structure`, `run_sql` and `wait_for_approval`. Every result starts with a status word.
- [x] **Access per connection:** Off, Read, Read with approved writes, or Full, set in the connection's form. Red-labelled connections start at Off and every other at Read. A read-only connection stays read-only.
- [x] **Approvals in the table:** an agent's change opens in its own tab as staged rows, cells or columns, or as its SQL, and the bar asks 1. Yes, 2. Yes, don't ask again here, 3. No, and tell it why. The window comes forward to ask. A row that changed meanwhile rolls the change back.
- [x] **Passwords:** a connection that asks each time asks Dan when an agent needs it, and the agent waits.
- [x] **The Agents page:** connected agents, every call with its SQL and outcome, the pause switch, and Connect for Claude Code, Codex, Cursor, VS Code and Claude Desktop.
- [x] **Dan's check** (2026-10-29): "its all good and approved". Claude Code itself wasn't connected during it, so the first real session is still to come.
- [ ] **Windows check,** from a release build: stdio through the app binary, the app opening apart from the agent, and coming forward.

### Safety (built 2026-10-29)

Dan chose to release sooner: this step, then v0.2.0 with agents, then the rest of M5.

- [x] **Hidden columns:** an eye on each column in the Structure view hides it from agents, and suggests it for names like email, phone, token, password and secret. Its values show as `[hidden]` wherever an agent's query returns them; a query that filters on it or computes with it, and a write that reads it, is refused, using Postgres's own plan. It stops accidental exposure, not an agent deliberately working around it through a view or a kept-apart subquery.
- [x] **Undo for `change_rows`:** the reverse of each committed row change is kept, deleted rows' values included, and Undo on the Agents page stages it in the table like your own edits. `run_sql` and structure changes can't be undone there.
- [ ] **Dan's check.**

### Release

- [ ] **v0.2.0** with the agent core and the safety step, built by `release.yml` from a tag.
- [ ] **Listed:** the repository's description without "(coming soon)", the topics mcp, mcp-server, ai-agents and claude-code, and Halfnine in the official MCP Registry, Smithery, mcp.so and awesome-mcp-servers.
- [x] **A Claude Desktop extension** (`.mcpb`), built with each release; the MCP Registry lists it.

### Next in M5
- **Migrations, not direct DDL:** when the agent's project has migrations (Drizzle, Prisma, sqlx and others, found through the project folder the agent shares by MCP roots), a schema change becomes a migration file in that project rather than a direct change to the database. Direct DDL is only for databases nothing owns, such as a scratch Docker database.
- **Work on a copy:** clone a database before risky work. On a local server this is `CREATE DATABASE … TEMPLATE`; for Docker, a cloned container. The agent works on the copy, and Dan keeps it or discards it.
- **Credentials:** an agent may receive the URL of a Docker database Halfnine created, because those credentials are local and throwaway. Every other connection's credentials never leave the app.
- **Agents create, copy and delete local databases,** with Docker below. Databases Halfnine created in Docker start at Full.

### Docker

- **See and connect:** it lists the Postgres containers on this machine and connects in a click, reading the user, password and database from the container's environment. It works with any Docker engine socket: Docker Desktop, OrbStack or Colima.
- **Start, stop and delete:** deleting a container offers to delete its data volume too, after a confirmation.
- **Create:**
  - Choose a Postgres version and a name. The port is picked automatically.
  - Optionally load a dump or an SQL file.
  - Each database gets its own volume, and Halfnine labels its containers so it knows which are its own.
- **Agents create their own:** an agent can ask for a fresh database and get one without asking Dan. It shows in the sidebar marked with the agent's name.

### Candidates

The Docker Engine API client `bollard`, which passes the same adoption test as every other dependency, with the same two-week age rule. The MCP SDK, `rmcp`, is in use.

## Next, in the order Dan chooses

- An auto-updater.
- Multiple windows.
- Dump and restore for remote servers.
- MySQL and SQLite through `Database`.
- Signing.
- Homebrew and winget.
- A website.

## Not planned

- A tray icon or a global hotkey: Dan chose a normal app. An agent opens the app when it is closed, and the window comes forward when it needs an answer.
- Cloudflare: GitHub only for now.
- Forking an existing client: Dan chose a fresh build.
