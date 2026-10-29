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
- **Where queries live.** **Dan:** "commit and clean up i cant see where i ran commands or anything like that I feel like the UI should present this query option maybe im wrong instead of just the text idk where to follow our current design standard yk but lets get that fixed". Then: "I feel like it should be on the bottom and no collapse all of the options in the page of queries yk".
  - **Shown:** three layouts. **Dan, on the first** (a Queries page opened from the foot of the sidebar): "Yes, idk about history maybe another button inside the page to see history or something like that but idk what we are currently doing and idk what other pages i should add like queries".
  - **Dan:** "And dont overwhelm the user with options its the whole design", and "Remember my design philosophy we have been working with yk".
  - **Built:**
    - A fixed footer of pages in the sidebar: Queries and Sessions. M5 adds Agents and Docker there.
    - The Queries page shows the saved queries, with History behind a switch on the page.
    - The empty workspace offers buttons.
    - The structure is written into docs/brand.md.
- **M3.** **Dan:** "The m2 fixes work, lets do M3". Cancel with ⌘ and . together, and Analyze, both work in the app. The Queries page was committed as it stood before M3 began.
- **M3 check, the keychain.** **Dan:** "Ok im stuck in the key access i cant even access it". **Asked:** what he was stuck on. **Chose:** a Keychain password box.
  - Cause: the connections list read every saved password just to show "Saved in your Keychain", and every rebuilt dev app is a new app to macOS, so it asked again each time.
  - **Asked:** how to fix it at the root. **Chose:** ask only to connect. **Not chosen:** also signing dev builds with a self-signed certificate; leaving it.
  - The list no longer reads the keychain. The password field says an untouched field keeps what is saved.
- **Why the keychain at all.** **Dan:** "Why did you force me to store shit in the keychan in the first place". It came from the plan, following the secrets rule in `architecture-invariants.md`, and was never asked on its own.
  - **Asked:** where passwords should live. **Dan:** "Idk it should work like other apps ask to save password or input each time save where key is or something man save on keychan as a toggle option". Then: "Idk how windows works on this too".
  - **Built:** a Save password switch on each connection, on for new ones and for those saved before. When off, nothing is saved and a sheet asks for the password on each connect. Switching databases reuses the open session's password. On Windows the same switch uses Credential Manager, which never prompts.
- **Passwords, again.** **Dan:** "Ok it just feelsl ike too much friction yk it should be once for app unlock and i dont like theres two layers of keychain and once its flipped back off its always deleted idk help me out man". Then, on how to stop saving: "Lets follow FileZilla how they store shit or TablePlus or other stuff how do they do it idk man maybe youre right lets figure this shit out".
  - Looked up: TablePlus has a menu on the password (Store in keychain, Ask every time). FileZilla's Ask for password remembers it for the session. DataGrip offers Forever, Until restart and Never.
  - **Asked:** build it that way. **Chose:** build it.
  - **Built:** a menu beside Password, Save in Keychain or Ask each time, which also covers the SSH password. All saved passwords share one keychain entry, read once per launch. Typed passwords are remembered until the app quits. Nothing is removed until the form is saved, and the field says so first. The Save password switch is gone.
- **M3, after using the app.** **Dan:** "Ok its good though, lets move on commit and get live". M3 committed.
- **Ship v0.1 as TableUltra?** **Chose:** rename first. **Not chosen:** keep TableUltra.
  - **Offered:** Mesita, Veta and Tamiz (checked earlier the same day), or a new round. **Dan:** "Halfnine".
  - Checked: no app or database tool is called Halfnine. A two-person US IT-services firm, founded 2023, uses the name. halfnine.com and halfnine.app are registered; halfnine.dev, gethalfnine.app and halfnineapp.com were free. The "halfnine" GitHub organization exists, dormant since 2012. The crate and npm names were free. Trademarks weren't checked.
  - Renamed everywhere but this file. The app ID is now `com.notinferred.halfnine`.
- **License, description and topics.** **Dan:** "Lets add the correct repo tags descriptions and license to the project make sure it was since the first commit".
  - **Which license?** **Dan:** "Just not used for resale or others to profit idk how i could even do it myself but yeah thats it". **Offered:** PolyForm Shield (recommended), PolyForm Noncommercial, the Functional Source License. **Dan:** "Go", to the recommendation.
  - **Since the first commit?** **Chose:** rewrite history, so LICENSE.md is in every commit from the first. **Not chosen:** add it in a new commit.
  - **Copyright?** **Chose:** notInferred.
  - **Topics.** **Dan:** "I dont like those topics try some people actually look at", then: "Think of it its a tableplus alternatives FOR database management for AI features too", "Thats the selling point AI But the tags for AI suck idk man Help me out". **Offered:** a description that leads with AI agents, marked as coming until M5 ships, and twelve topics people browse now, with mcp, mcp-server, ai-agents and claude-code added when M5 ships. **Dan:** "Go".

## 2026-10-29: M5, the agent core

- **What to build.** **Dan:** "Lets do M5, lets make sure to have a claude code-like and compatible design frictionless and compatible with table manager"
- **What "Claude Code-like" means.** **Chose:** "Works inside Claude Code": Halfnine is a tool that Claude Code, Cursor and Codex call over MCP, and asks before writes in Claude Code's style: 1. Yes, 2. Yes, don't ask again here, 3. No, and tell it why. Halfnine comes forward to ask. **Not chosen:** an agent inside Halfnine; both, MCP first.
- **How an agent's work shows in the table manager.** **Chose:** "In the table, like my edits": staged cells and rows in that table's tab, marked with the agent's name, with the same Review SQL. **Not chosen:** on an Agents page; both.
- **How much of M5 now.** **Chose:** "Agent core first": MCP, one-step setup, access per connection, approvals, the activity log and the pause switch. Undo, copies, migrations, hidden columns and Docker follow. **Not chosen:** all agent features, then Docker; all of M5 at once.
- **What "Yes, don't ask again here" covers.** **Chose:** "This agent, this connection, until quit". Nothing is saved. **Not chosen:** switch the connection to Full; this agent, this table, until quit.
- **Setup.** **Chose:** "One click per client": Halfnine shows what it will change, then adds itself; for Claude Code it also allows Halfnine's tools, so only Halfnine asks. **Not chosen:** copy and paste.
- **An agent needs a password typed each time, not typed yet.** **Chose:** "Halfnine asks you": the window comes forward with the usual password box and the agent waits. **Not chosen:** tell the agent to ask.
- **A change on a connection other than the open one.** **Chose:** "Its own tab, beside yours"; on the connections screen, Halfnine opens that connection. **Not chosen:** switch to that connection.
- **A change to a table already open on the same connection.** **Chose:** "Its own tab, always". **Not chosen:** in your tab if nothing is staged.
- **What still asks after "don't ask again".** **Chose:** "Dropping or emptying a table", including inside the agent's SQL. **Not chosen:** nothing.
- **Values in the activity log.** **Chose:** "SQL as written". **Not chosen:** SQL without values.
- **Connections set to Off.** **Chose:** "Listed as Off", so the agent can tell Dan. **Not chosen:** hidden.
- **The plan** for the agent core, including the Agents entry on the connections screen and the changes to CLAUDE.md's guarantees. **Accepted.**
- **Checking the code on this Mac.** **Chose:** "Compile checks as I go": cargo check, clippy and the bindings test while building; the Postgres tests run in CI, and the app waits for the final check. **Not chosen:** CI only; compile and test as I go.
- **Design of the Agents entry on the connections screen.** **Dan:** "agents button should come before new connection one or at least a divider idk the design feels fit in tightly just to fit it somewhere not designed to be cohesive". **Chose:** "Footer, like the sidebar": New Connection stays under the list, and Agents sits below it in the same divided footer of pages as the workspace sidebar. **Not chosen:** Agents first, then New Connection; a + beside Saved with Agents in the footer.
- **The Agents page with nothing connected.** **Dan:** "Also the agent tabs isnt when no agent connected the default to add a new agent idk man the design is getting bad on these angles too". The page now opens on Connect until an agent is set up or connected, and an empty activity list offers Connect an agent as a button.
- **The check of the agent core.** **Dan:** "Lets finish the steps its all good and approved lets finish whats missing". Approved, with Claude Code not yet connected in the check. The next steps were merging the agent core, publishing v0.1.0 and measuring the installed app.
