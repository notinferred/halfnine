# Dan's decisions

Each entry gives the date, the question, and Dan's own words or the option he chose. Entries are never reworded. A later decision supersedes an earlier one by saying so.

## The brief (2026-10-28)

**Dan:** "Lets create a minimalist tableplus alternative the goal is also to make it very lightweight and always open while very refined in terms of functionality so we focus on design and ease of use. minimalist yet complete end to end should not be much Use a lot of libraries with the goal of following architecture and and reduce the maximum amount of code maybe even use open source projects and design toold. focus on design, apple design philosophy not their design directly. MACOS and windows app support even if unsigned now. use the correct languages for this and architecture for a monorepo with zero wasteful ci/cd distribution etc. simple for now very minimalist […] Desktop app macos and windows compatible help me with everything from roadmap a propper setup and naming and branding the small product"

The architecture follows his references `architecture-invariants.md` and `ENGINEERING-CONCEPTS.md`.

## 2026-10-28

- **Which databases must the first version support?** **Dan:** "PostgreSQL, lets deploy everything with cloudflare services if possible". The deployment half was superseded later the same day; see release builds below.
- **How should development builds work on this Mac?** **Chose:** on-demand local. `tauri dev` runs only when he asks during an active session and is stopped afterwards. No local tests unless he approves them.
- **What does "always open" mean?** **Chose:** a normal app with a tiny footprint, light enough to leave open all day. **Not chosen:** a menu bar or tray resident app; tray plus a global hotkey.
- **Where should release builds happen?** **Dan:** "Ignore cloudflare then GH Actions and tag, public repo ill make it public but this isnt your priority yk"
- **What should exist on Cloudflare for the first release?** **Dan:** "Only github, all of that other stuff is not a priority the app that works is first"
- **Name.** **Dan:** "I can change the name use TableUltra for now". **Not chosen:** Mesita, Veta, Tabla and Tamiz, each checked for conflicts and free domains.
- **Fresh build or fork?** **Chose:** a fresh build from libraries. **Not chosen:** forking dbcooper (MIT) or Tabularis (Apache-2.0).
- **Data grid.** **Chose:** RevoGrid, after a time-boxed spike, with TanStack Table + Virtual as the fallback. **Not chosen:** Glide Data Grid, which has had no stable release since February 2024.
- **Who owns the repository and authors commits?** **Chose:** the notinferred org, with commits as `notInferred <contact@notinferred.com>`.
- **The plan** (stack, layout, invariants, CI, roadmap M0–M4). **Accepted.**
- **Design pass**: the mockup of Connections, Table, Query and the ⌘K palette, in light and dark. **Dan:** "Looking perfect from what i can see"
- **Install Rust on this Mac?** **Chose:** via Homebrew (`brew install rustup`). `rust-toolchain.toml` picks the version. **Not chosen:** the rustup.rs script; not yet.
- **First commit?** **Chose:** commit M0 now, locally, as notInferred. Nothing is pushed.
- **Which Postgres checks M1?** **Chose:** a throwaway `postgres:18` Docker container with sample data, started only when Dan asks for a check, then stopped and deleted. **Not chosen:** a hosted development database; deciding at M1.
- **Scope extension, M5.** **Dan:** "Ok now we can extend scope on the roadmap, I want it to be a table manager for AI so when you put it in the AI for example we need a table right now it would call tableplus instead of asking and we can also see and manage local docker databases yk. What am i not thinking about either lets add that i think M4 it was after it so make M5"
- **What may an agent do without approval?** **Chose:** per connection, with safe defaults: Off / Read / Read with approved writes / Full. Docker databases TableUltra created are Full, red-labelled connections are Off, and every other connection is Read. **Not chosen:** reads free and writes approved everywhere; everything approved.
- **Which Docker abilities?** **Chose:** see and connect; start, stop and delete; create new databases; agents create their own. **Dan:** "Also lets be able to connect to tables through keys and other common database features not on this but before yk". These went into M1–M3: foreign-key navigation and key-aware editing, SSH tunnels, switching databases, the row detail panel, saved queries, the EXPLAIN view, the sessions view, editors for big values, CSV import, structure editing, and truncate and drop.
- **What else should go into M5?** **Chose:** activity log and undo; migrations, not direct DDL; work on a copy; hide sensitive columns.
- **M1, after using the app.** **Dan:** "Looks great lets keep revogrid, run them against the db and if good commit then clean up". RevoGrid stays. The Postgres tests ran once against the local check database instead of waiting for CI.
- **M2.** **Dan:** "Lets do M2". **How to check it?** **Chose:** the app and the tests, as for M1: the throwaway database, all the Rust tests against it, then the app for Dan to try. Commit and clean up if it passes.
- **M2, after using the app.** **Dan:** "Just pressing command didnt cancel it but the button did and analyze idk if its working Everything else looks good". **Asked:** which keys he pressed (**Chose:** "Only ⌘") and what Analyze showed (**Chose:** "An error").
  - The Postgres log showed Analyze had been sent a lone `)`. A selection was clipped to the statements around it.
  - Selections are now parsed on their own.
  - The statement at the cursor is tinted, as the approved mockup showed.
  - Explain errors quote what they were sent.
  - Shortcuts are drawn as keycaps (⌘ and . apart).
