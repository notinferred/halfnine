# TableUltra

A small, calm PostgreSQL client for macOS and Windows, light enough to leave open all day. TableUltra is a working name.

It is early: [ROADMAP.md](ROADMAP.md) says where things stand.

## Install

There are no releases yet. Release builds will be unsigned, so each system asks once before opening them:

- **macOS** (Apple silicon, macOS 13 or later): open the `.dmg` and drag TableUltra to Applications. Open it once, then choose System Settings › Privacy & Security › Open Anyway. Or run `xattr -dr com.apple.quarantine /Applications/TableUltra.app`.
- **Windows** (x64): run the `-setup.exe`. At the SmartScreen warning choose More info › Run anyway. It installs for the current user, without administrator rights.

## Develop

You need Node 24 or later, pnpm (the version in `package.json` is fetched automatically), Rust through [rustup](https://rustup.rs) (`rust-toolchain.toml` picks the version), and on macOS the Xcode Command Line Tools. With Homebrew, `brew install rustup`, then put `/opt/homebrew/opt/rustup/bin` on your `PATH`; Homebrew leaves it off.

| Command | What it does |
|---|---|
| `pnpm install` | Installs the UI's dependencies |
| `pnpm dev` | Opens the app, reloading the UI as you edit. The first run compiles the Rust side and takes a few minutes |
| `pnpm check` / `pnpm format` | Lints and checks formatting with Biome / applies its fixes |
| `pnpm typecheck` | Checks the UI's types |
| `cargo fmt --all` | Formats the Rust crates |
| `cargo clippy --workspace --all-targets -- -D warnings` | Lints the Rust crates |
| `pnpm icons` | Renders every app icon size from `assets/brand/app-icon.svg` |
| `pnpm tauri build` | Builds the installer for the machine it runs on |
| `cargo test --workspace` | Runs the Rust tests. The Postgres tests skip themselves unless `TU_TEST_DATABASE_URL` names a database loaded with `crates/pg/tests/fixture.sql` |

To check the app or run the Postgres tests on a Mac without a database, start a throwaway one in Docker. It deletes itself when stopped:

```sh
docker run -d --rm --name tableultra-check -e POSTGRES_PASSWORD=check -p 127.0.0.1:55432:5432 postgres:18
docker exec -i tableultra-check psql -U postgres -q < crates/pg/tests/fixture.sql
# connect the app to postgres://postgres:check@localhost:55432/postgres
# or: TU_TEST_DATABASE_URL=postgres://postgres:check@localhost:55432/postgres cargo test --workspace
docker stop tableultra-check
```

## Layout

- `apps/desktop`: the UI, in React and Vite. Design tokens are in `src/ui/tokens.css`.
- `crates/desktop`: the Tauri app that hosts the UI, with its window, permissions and icons.
- `docs/`: [architecture](docs/architecture.md), [decisions](docs/decisions.md) and [brand](docs/brand.md).
