import { PostgreSQL, sql } from "@codemirror/lang-sql";
import { ensureSyntaxTree, syntaxTree } from "@codemirror/language";
import { EditorState } from "@codemirror/state";

export type Piece = { from: number; to: number; text: string };

const onlyComments = (text: string) =>
  text
    .replace(/--[^\n]*/g, "")
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .trim() === "";

/**
 * Every statement in the document, as lang-sql's parser splits it (strings, comments and dollar-quoted bodies
 * included). Postgres checks each piece again before it runs: it refuses one that holds more than one statement.
 */
export function statementsIn(state: EditorState): Piece[] {
  const tree = ensureSyntaxTree(state, state.doc.length, 500) ?? syntaxTree(state);
  const pieces: Piece[] = [];
  tree.iterate({
    enter: (node) => {
      if (node.name !== "Statement") return;
      const text = state.sliceDoc(node.from, node.to).trim().replace(/;\s*$/, "").trim();
      if (!onlyComments(text)) pieces.push({ from: node.from, to: node.to, text });
      return false;
    },
  });
  return pieces;
}

/** The statements inside a selection, read from the selected text alone, so a selection never yields part of a statement around it. */
export function statementsSelected(state: EditorState, from: number, to: number): Piece[] {
  const selected = EditorState.create({ doc: state.sliceDoc(from, to), extensions: [sql({ dialect: PostgreSQL })] });
  return statementsIn(selected).map((p) => ({ ...p, from: p.from + from, to: p.to + from }));
}

/** The statement the cursor is in, or the one just before it when the cursor sits after its semicolon. */
export function statementAt(state: EditorState, position: number): Piece | null {
  const all = statementsIn(state);
  return all.find((p) => p.from <= position && position <= p.to) ?? all.filter((p) => p.to <= position).at(-1) ?? all[0] ?? null;
}

/** What Run, Explain and Analyze act on: the selection's statements, or the statement at the cursor. */
export function target(state: EditorState): Piece[] {
  const selection = state.selection.main;
  if (!selection.empty) return statementsSelected(state, selection.from, selection.to);
  const one = statementAt(state, selection.head);
  return one ? [one] : [];
}
