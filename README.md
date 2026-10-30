# Halfnine

A small, calm PostgreSQL client for macOS and Windows, light enough to leave open all day.

It is early: [ROADMAP.md](ROADMAP.md) says where things stand.

## Install

Download the installer from [Releases](https://github.com/notinferred/halfnine/releases). The builds aren't signed yet, so each system asks once before opening them:

- **macOS** (Apple silicon, macOS 13 or later): open the `.dmg` and drag Halfnine to Applications. Open it once, then choose System Settings › Privacy & Security › Open Anyway. Or run `xattr -dr com.apple.quarantine /Applications/Halfnine.app`.
- **Windows** (x64): run the `-setup.exe`. At the SmartScreen warning choose More info › Run anyway. It installs for the current user, without administrator rights.

When a connection saves its password, macOS asks once before Halfnine first reads it, and again after each update until the app is signed. Choose Always Allow.

## Use with AI agents

Halfnine is also an MCP server, so Claude Code, Codex, Cursor, VS Code and Claude Desktop can use your connections through it. Open Agents, at the foot of the sidebar or the connections screen, choose Connect next to your tool, and Halfnine shows what it will change before it adds itself. Or add it by hand; on macOS:

```sh
claude mcp add --scope user --transport stdio halfnine -- /Applications/Halfnine.app/Contents/MacOS/halfnine mcp
codex mcp add halfnine -- /Applications/Halfnine.app/Contents/MacOS/halfnine mcp
```

For Claude Desktop, install Halfnine first, then open the `.mcpb` file from the same release to add it as an extension. It is also listed in the official MCP Registry as `io.github.notinferred/halfnine`.

Each connection sets what agents may do there, under Agents in its settings: Off, Read, Read with approved writes, or Full. Red-labelled connections start at Off and every other at Read. At Read with approved writes, an agent's change opens in Halfnine as staged rows or SQL and waits for your answer. Everything agents do is listed on the Agents page, where one switch pauses them all. What an agent reads goes to its AI provider.

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
| `git tag v0.2.0 && git push origin v0.2.0` | Builds the macOS and Windows installers and the Claude Desktop extension in GitHub Actions into a draft release, to check and publish. Publishing it lists that version in the MCP Registry |
| `cargo test --workspace` | Runs the Rust tests. The Postgres tests skip themselves unless `HN_TEST_DATABASE_URL` names a database loaded with `crates/pg/tests/fixture.sql` |

To check the app or run the Postgres tests on a Mac without a database, start a throwaway one in Docker. It deletes itself when stopped:

```sh
docker run -d --rm --name halfnine-check -e POSTGRES_PASSWORD=check -p 127.0.0.1:55432:5432 postgres:18
docker exec -i halfnine-check psql -U postgres -q < crates/pg/tests/fixture.sql
# connect the app to postgres://postgres:check@localhost:55432/postgres
# or: HN_TEST_DATABASE_URL=postgres://postgres:check@localhost:55432/postgres cargo test --workspace
docker stop halfnine-check
```

## Layout

- `apps/desktop`: the UI, in React and Vite. Design tokens are in `src/ui/tokens.css`.
- `crates/desktop`: the Tauri app that hosts the UI, with its window, permissions and icons.
- `docs/`: [architecture](docs/architecture.md), [decisions](docs/decisions.md) and [brand](docs/brand.md).

## License

Halfnine is source-available under the [PolyForm Shield License 1.0.0](LICENSE.md). You may use it for any purpose, at work included, and change it, but not to sell it or to provide a product that competes with it. Copyright 2026 notInferred.
