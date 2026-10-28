# Roadmap

## Where things stand (2026-10-28)

M0 is nearly done. The repository, pinned toolchains, lockfiles, CI check, design tokens and icon exist, and Dan approved the design. The window is written to open on a welcome screen, but it hasn't been compiled yet. Rust is installed on Dan's Mac, and the first `pnpm dev` waits for his go-ahead. The repository isn't on GitHub yet, so CI hasn't run.

## M0: Foundation

- [x] Repository, workspace, pinned toolchains, Biome and rustfmt
- [x] `check.yml`
- [x] Design tokens for light, dark, increased contrast and reduced motion, plus the welcome window
- [x] An icon that doesn't depend on the name, drawn once and rendered by `pnpm icons`
- [x] Design pass: Dan approved the mockup of Connections, Table, Query and the palette (2026-10-28). The tokens match it
- [x] Rust installed through Homebrew, and `Cargo.lock` committed
- [ ] First compile and run on macOS, when Dan says to

## M1: Connect and browse

These two spikes come first, about a day each:

1. **RevoGrid.** It stays only if, styled with our tokens, it:
   - matches the mockup in light and dark;
   - scrolls 100k rows smoothly;
   - edits a cell while keeping NULL apart from empty;
   - handles keyboard navigation and copying as TSV;
   - has resizable, sortable headers.

   Otherwise TanStack Table and TanStack Virtual replace it.
2. **sqlx `raw_sql`.** It stays only if, for a multi-statement script that includes extension types, it returns values as text and type names for each statement. Otherwise `tokio-postgres` `simple_query` replaces it.

Then build:

- Connections:
  - a form, or a pasted `postgres://` URL, with a connection test;
  - TLS modes;
  - the password kept in the OS keychain;
  - colour labels;
  - a read-only toggle.
- A schema sidebar with a filter.
- Table tabs, each with Data and Structure views.
- The ⌘K palette.

Also in M1:

- **New crates.** `crates/core` holds pure types and the `Database` trait, and does no I/O. `crates/pg` is the Postgres adapter.
- **CI.** `check.yml` gains a Postgres service, the tests and the tauri-specta bindings check.

These guarantees become true in M1 and move to CLAUDE.md once the code enforces them:

- Passwords live only in the OS keychain.
- Values are always bound parameters, and identifiers always go through one quoting function.
- The server enforces read-only connections.
- `tu-core` does no I/O.

## M2: Query

- An editor with schema autocomplete.
- Run and cancel, and multiple result sets.
- History, formatting, copying and export.

## M3: Edit

- Staged edits, reviewed as SQL before they run.
- One transaction per commit, with conflicts detected.
- Insert and delete, and filters.

## M4: v0.1

- `release.yml` builds an unsigned `.dmg` (Apple silicon) and a per-user `-setup.exe` (x64) into a draft GitHub Release whenever a version tag is pushed.
- The final icon and the README's install steps.
- Measured budgets: an installer under 15 MB, a window ready in under 1 second, and idle memory recorded on both systems.

## Next, in the order Dan chooses

- SSH tunnels.
- An auto-updater.
- Multiple windows.
- MySQL and SQLite through `Database`.
- Signing.
- Homebrew and winget.
- A website.
- The final name.

## Not planned

- A tray icon or a global hotkey: Dan chose a normal app.
- Cloudflare: GitHub only for now.
- Forking an existing client: Dan chose a fresh build.
