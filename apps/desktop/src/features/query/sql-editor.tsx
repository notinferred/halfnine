import { autocompletion, closeBrackets, closeBracketsKeymap, completionKeymap } from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { PostgreSQL, type SQLNamespace, sql } from "@codemirror/lang-sql";
import { bracketMatching, HighlightStyle, indentOnInput, syntaxHighlighting } from "@codemirror/language";
import { Compartment, EditorState, RangeSetBuilder } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  drawSelection,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
  placeholder,
  ViewPlugin,
  type ViewUpdate,
} from "@codemirror/view";
import { tags } from "@lezer/highlight";
import { type RefObject, useEffect, useRef } from "react";
import { statementAt } from "./statements.ts";

/** Colours come from the app's tokens, so the editor follows light, dark and increased contrast with everything else. */
const theme = EditorView.theme({
  "&": { height: "100%", fontSize: "12.5px", color: "var(--text)", backgroundColor: "var(--bg)" },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { fontFamily: "var(--font-mono)", lineHeight: "20px" },
  ".cm-content": { caretColor: "var(--accent)", padding: "10px 0" },
  ".cm-gutters": { backgroundColor: "var(--bg)", color: "var(--text-2)", border: "none" },
  ".cm-lineNumbers .cm-gutterElement": { padding: "0 12px 0 16px" },
  ".cm-activeLine": { backgroundColor: "var(--hover)" },
  ".cm-activeLineGutter": { backgroundColor: "transparent", color: "var(--text)" },
  ".cm-cursor": { borderLeftColor: "var(--accent)", borderLeftWidth: "2px" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection": { backgroundColor: "var(--selection)" },
  ".cm-matchingBracket": { backgroundColor: "var(--selection)", outline: "none" },
  ".cm-placeholder": { color: "var(--text-2)" },
  ".cm-target": { backgroundColor: "var(--staged)" },
  ".cm-tooltip": {
    border: "none",
    borderRadius: "var(--radius-md)",
    backgroundColor: "var(--field)",
    boxShadow: "var(--shadow-overlay)",
    overflow: "hidden",
  },
  ".cm-tooltip-autocomplete > ul": { fontFamily: "var(--font-mono)", fontSize: "12px", maxHeight: "240px" },
  ".cm-tooltip-autocomplete > ul > li": { padding: "3px 10px !important" },
  ".cm-tooltip-autocomplete > ul > li[aria-selected]": { backgroundColor: "var(--selection)", color: "var(--text)" },
  ".cm-completionDetail": { color: "var(--text-2)", fontStyle: "normal" },
});

const targetLine = Decoration.line({ class: "cm-target" });

/** Tints the lines of the statement at the cursor, so it is always clear what Run, Explain and Analyze will use. A selection shows itself. */
const statementHighlight = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = this.build(view);
    }
    update(update: ViewUpdate) {
      if (update.docChanged || update.selectionSet || update.viewportChanged) this.decorations = this.build(update.view);
    }
    build(view: EditorView): DecorationSet {
      const builder = new RangeSetBuilder<Decoration>();
      const { state } = view;
      if (!state.selection.main.empty) return builder.finish();
      const piece = statementAt(state, state.selection.main.head);
      if (!piece) return builder.finish();
      for (let line = state.doc.lineAt(piece.from); ; line = state.doc.line(line.number + 1)) {
        builder.add(line.from, line.from, targetLine);
        if (line.to >= piece.to || line.number === state.doc.lines) break;
      }
      return builder.finish();
    }
  },
  { decorations: (plugin) => plugin.decorations },
);

const highlight = HighlightStyle.define([
  { tag: tags.keyword, color: "var(--accent)", fontWeight: "500" },
  { tag: [tags.comment, tags.lineComment, tags.blockComment], color: "var(--text-2)", fontStyle: "italic" },
  { tag: [tags.string, tags.special(tags.string)], color: "var(--text-2)" },
]);

export type EditorKeys = {
  run: () => void;
  runAll: () => void;
  format: () => void;
  save: () => void;
  cancel: () => void;
};

export function SqlEditor(props: {
  value: string;
  onChange: (sql: string) => void;
  schema: SQLNamespace;
  keys: EditorKeys;
  view: RefObject<EditorView | null>;
}) {
  const host = useRef<HTMLDivElement>(null);
  const language = useRef(new Compartment());
  const latest = useRef(props);
  latest.current = props;

  // Created once per tab; the tab keeps the text, so this mounts with it.
  useEffect(() => {
    if (!host.current) return;
    const bind = (key: string, action: keyof EditorKeys) => ({
      key,
      run: () => {
        latest.current.keys[action]();
        return true;
      },
    });
    const view = new EditorView({
      parent: host.current,
      state: EditorState.create({
        doc: latest.current.value,
        extensions: [
          lineNumbers(),
          highlightActiveLineGutter(),
          highlightActiveLine(),
          history(),
          drawSelection(),
          indentOnInput(),
          bracketMatching(),
          closeBrackets(),
          autocompletion(),
          syntaxHighlighting(highlight),
          theme,
          placeholder("SELECT …"),
          statementHighlight,
          keymap.of([
            bind("Mod-Enter", "run"),
            bind("Shift-Mod-Enter", "runAll"),
            bind("Shift-Mod-f", "format"),
            bind("Mod-s", "save"),
            bind("Mod-.", "cancel"),
            ...closeBracketsKeymap,
            ...defaultKeymap,
            ...historyKeymap,
            ...completionKeymap,
            indentWithTab,
          ]),
          language.current.of(sql({ dialect: PostgreSQL, schema: latest.current.schema, defaultSchema: "public", upperCaseKeywords: true })),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) latest.current.onChange(update.state.doc.toString());
          }),
        ],
      }),
    });
    props.view.current = view;
    view.focus();
    return () => {
      props.view.current = null;
      view.destroy();
    };
  }, [props.view]);

  useEffect(() => {
    props.view.current?.dispatch({
      effects: language.current.reconfigure(sql({ dialect: PostgreSQL, schema: props.schema, defaultSchema: "public", upperCaseKeywords: true })),
    });
  }, [props.schema, props.view]);

  return <div ref={host} className="sql-editor" />;
}
