# Brand

**Halfnine** is the name, chosen by Dan for v0.1. Its app ID, `com.notinferred.halfnine`, names the Keychain entry and the settings folder, so it stays fixed from v0.1 even if the name ever changes. The icon and the visual language don't depend on the name.

## Character

Halfnine is calm, quick and exact. It borrows Apple's design philosophy, not Apple's look:

- **Deference.** Your data is the subject, and the interface stays quiet around it.
- **Clarity.** Every element says what it does, in plain words.
- **Forgiveness.** Changes are staged, shown as SQL and committed together. Production is a colour you can't miss.
- **Restraint.** One accent, few settings, and nothing decorative.

## Colour

Colours are defined in `apps/desktop/src/ui/tokens.css`. Nothing else defines them.

- **Neutrals carry the interface.** There is one blue accent, used for selection and the one primary action in a view.
- **Connection labels:** green, amber and red mark environments. They are the only other colours, and they appear only on connection labels.
- **Text contrast.** Secondary text (`--text-2`) passes 4.5:1 on every surface. `--text-3` fails that, so it is only for disabled and decorative marks, never for text someone has to read.
- **Light and dark follow the system.** Increased contrast strengthens lines and secondary text, and reduced motion turns transitions off.
- **Depth comes from lightness.** Shadows appear only on overlays such as the palette, menus and dialogs.

## Type and shape

- **Type.** The interface and the grid use the system font: SF on macOS and Segoe UI Variable on Windows. SQL and type names use the system monospace. No font files ship with the app.
- **Sizes.** Text is 13 px, at the density of a Mac app, with 12 px in the grid. The grid uses tabular figures so numbers line up.
- **Values.** The grid shows values exactly as Postgres prints them, and NULL is shown as a word in italics, distinct from an empty string.
- **Spacing and radii.** Spacing comes in steps of 4 px. Radii are 4, 6 and 10 px.
- **Icons.** Icons are Lucide's at a 1.5 px stroke, and there are no emoji.

## Structure

The whole window follows one layout, so a new feature finds its place rather than adding a panel.

- **The sidebar is the map.** It shows the connection at the top and the tables in the middle. The app's pages sit at the foot: Queries, Sessions and Agents, with Docker to come. They are always in view, and nothing collapses them. Agents is also at the foot of the connections screen, so pausing and setup work before any connection is open.
- **A new kind of place becomes a page in that footer,** not another panel, menu or setting.
- **A page shows one thing at a time.** Queries lists the saved queries, and a switch on the page shows History instead.
- **What you open becomes a tab:** a table, a query or a page. ⌘K reaches any of them by name.
- **An empty area offers its next step as a button,** never as instructions to read.
- **Shortcuts are shown as keycaps,** one per key, so ⌘ and . can't read as ⌘ followed by a full stop.
- **A change is staged before it is written.** Staged cells and columns take the accent colour on a faint accent ground. A bar at the foot of the view counts them and offers Review SQL, Discard and Commit.
- **An agent's change is staged like yours.** It opens in its own tab, marked with the agent's name, as staged cells, rows or columns that only answering can change. The bar asks the way Claude Code asks: 1. Yes, 2. Yes, don't ask again here, 3. No, and tell it why. Nothing is chosen by default, and the choices work only after the window has had focus for a second.
- **What can't be undone asks for the name.** Truncate and drop show the statement and wait for the table's name to be typed.

## Icon

The icon is a 3 × 3 table. The top row is its header, and the centre cell is selected in the accent colour. It sits on a dark tile drawn to Apple's icon grid (an 824 px tile on a 1024 px canvas). It is drawn once in `assets/brand/app-icon.svg`, and `pnpm icons` renders every size from it. The welcome screen draws the same mark in `apps/desktop/src/ui/glyph.tsx`.

## Voice

- Short, plain sentences: say what happened and what to do next.
- Errors name the cause in words and keep Postgres's error code beside them, for example "Password rejected for user app (28P01)".
- No exclamation marks, no jokes in errors, and no "Oops".
- Buttons are verbs: Connect, Run, Commit, Discard. The one exception is an agent's question, which keeps Claude Code's own words, and its bare 1, 2 and 3 keys, the only shortcuts without a modifier.
- What agents read starts with one status word (OK, WAITING, APPROVED, DECLINED, CONFLICT, REFUSED, PAUSED, BUSY or ERROR), then says what happened and what to do next.
