# Brand

**TableUltra** is a working name. The icon and the visual language don't depend on it, so a rename touches only words.

## Character

TableUltra is calm, quick and exact. It borrows Apple's design philosophy, not Apple's look:

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

## Icon

The icon is a 3 × 3 table. The top row is its header, and the centre cell is selected in the accent colour. It sits on a dark tile drawn to Apple's icon grid (an 824 px tile on a 1024 px canvas). It is drawn once in `assets/brand/app-icon.svg`, and `pnpm icons` renders every size from it. The welcome screen draws the same mark in `apps/desktop/src/ui/glyph.tsx`.

## Voice

- Short, plain sentences: say what happened and what to do next.
- Errors name the cause in words and keep Postgres's error code beside them, for example "Password rejected for user app (28P01)".
- No exclamation marks, no jokes in errors, and no "Oops".
- Buttons are verbs: Connect, Run, Commit, Discard.
