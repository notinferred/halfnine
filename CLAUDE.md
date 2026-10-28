# TableUltra

A small, calm PostgreSQL client for macOS and Windows, light enough to leave open all day. TableUltra is a working name.

## Where each fact lives

Each fact has one home, and everything else points to it. Docs don't repeat what the code shows, and a rule the code doesn't enforce yet is a to-do in ROADMAP.md, not a guarantee.

| What | Where | Changes when |
|---|---|---|
| Commands and install steps | README.md | A command changes |
| Now and next | ROADMAP.md | A stage finishes; it is rewritten, not appended to |
| The design, its tools and why, the invariants | docs/architecture.md | The design changes |
| Dan's decisions, in his words | docs/decisions.md | Dan decides; entries are never reworded |
| Name, voice, colour, type, icon | docs/brand.md, with values in apps/desktop/src/ui/tokens.css | Only on purpose, said to Dan |
| What the code guarantees | This file, below | Only on purpose, said to Dan |

## What the code guarantees

- No crate uses `unsafe` code; the workspace forbids it.
- Every dependency we name is pinned exactly, at a version published at least two weeks before it was added. pnpm enforces that age for every JavaScript package, direct or not (`minimumReleaseAge` in `pnpm-workspace.yaml`). Cargo has no stable way to do the same, so for the Rust crates beneath ours, `Cargo.lock` records whatever versions were current when it was made.
- The window may call only what `crates/desktop/capabilities/default.json` grants, and its content security policy loads nothing from outside the app.
- The app sends nothing anywhere except to the databases you connect it to.

## Working agreement

- When a decision is Dan's, offer him options and don't pick for him.
- The app working comes first. Judge work by what you can see in the app.
- Don't compile, run the app or run tests on Dan's Mac unless he asks in that session, and stop what you started. Tests run in CI. Biome and the type check take a second and are fine to run.
- Prefer a maintained library over our own code. When you add a dependency, a file or a check, say what it replaces.
- Commits are authored as `notInferred <contact@notinferred.com>`, set in this repository's git config. No commit message, trailer or pull request credits an AI.
- Code reads plainly: names and structure carry the meaning, and a comment only keeps a reason or constraint the code can't show. Formatting is whatever Biome and rustfmt print.
