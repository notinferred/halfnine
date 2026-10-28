import type { SQLNamespace } from "@codemirror/lang-sql";
import type { EditorView } from "@codemirror/view";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { AlignLeft, ListTree, Play, Save, Square } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { format as formatSql } from "sql-formatter";
import { commands, type RunOutcome, type SchemaNode, type SessionInfo } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { type QueryTab as Tab, useWorkspace } from "../../state.ts";
import { Keys } from "../../ui/keys.tsx";
import { modCap, modKey, useShortcuts } from "../../ui/shortcuts.ts";
import { PlanView } from "./plan-view.tsx";
import { Results } from "./results.tsx";
import { SqlEditor } from "./sql-editor.tsx";
import { type Piece, statementsIn, target } from "./statements.ts";
import "./query.css";

export type Run =
  | { phase: "idle" }
  | { phase: "running"; startedAt: number }
  | { phase: "done"; statements: string[]; outcome: RunOutcome }
  | { phase: "plan"; plan: string; analyzed: boolean }
  | { phase: "error"; message: string };

/** Statements that only read; after anything else, the schema and open tables are refreshed. */
const READS = /^\s*(select|explain|show|values|table)\b/i;

function namespace(nodes: SchemaNode[] | undefined): SQLNamespace {
  return Object.fromEntries((nodes ?? []).map((n) => [n.name, Object.fromEntries(n.relations.map((r) => [r.name, r.columns]))]));
}

export function QueryTab({ session, tab }: { session: SessionInfo; tab: Tab }) {
  const id = session.sessionId;
  const queries = useQueryClient();
  const updateQuery = useWorkspace((s) => s.updateQuery);
  const view = useRef<EditorView | null>(null);
  const [run, setRun] = useState<Run>({ phase: "idle" });
  const [note, setNote] = useState<string | null>(null);
  const [naming, setNaming] = useState<string | null>(null);
  const [now, setNow] = useState(0);
  const schema = useQuery({ queryKey: ["schema", id], queryFn: () => commands.schema(id) });
  const names = useMemo(() => namespace(schema.data), [schema.data]);

  useEffect(() => {
    if (run.phase !== "running") return;
    const timer = setInterval(() => setNow(performance.now()), 100);
    return () => clearInterval(timer);
  }, [run.phase]);

  const pieces = (all: boolean): Piece[] => {
    const state = view.current?.state;
    if (!state) return [];
    return all ? statementsIn(state) : target(state);
  };

  const execute = async (all: boolean) => {
    if (run.phase === "running") return;
    const statements = pieces(all).map((p) => p.text);
    if (statements.length === 0) {
      setNote("There's no statement to run.");
      return;
    }
    setNote(null);
    setRun({ phase: "running", startedAt: performance.now() });
    try {
      const outcome = await commands.runStatements(id, statements);
      setRun({ phase: "done", statements, outcome });
      if (!statements.slice(0, outcome.results.length + 1).every((s) => READS.test(s))) {
        void queries.invalidateQueries({ predicate: (q) => q.queryKey[1] === id });
      }
    } catch (error) {
      setRun({ phase: "error", message: describe(error) });
    } finally {
      void queries.invalidateQueries({ queryKey: ["history"] });
    }
  };

  const explain = async (analyze: boolean) => {
    if (run.phase === "running") return;
    const [one] = pieces(false);
    if (!one) {
      setNote("Put the cursor in the statement to explain.");
      return;
    }
    setNote(null);
    setRun({ phase: "running", startedAt: performance.now() });
    try {
      setRun({ phase: "plan", plan: await commands.explain(id, one.text, analyze), analyzed: analyze });
    } catch (error) {
      const shown = one.text.length > 80 ? `${one.text.slice(0, 80)}…` : one.text;
      setRun({ phase: "error", message: `Couldn’t ${analyze ? "analyze" : "explain"} “${shown}”: ${describe(error)}` });
    }
  };

  const format = () => {
    const editor = view.current;
    if (!editor) return;
    const selection = editor.state.selection.main;
    const range = selection.empty ? { from: 0, to: editor.state.doc.length } : selection;
    try {
      const formatted = formatSql(editor.state.sliceDoc(range.from, range.to), { language: "postgresql", keywordCase: "upper", tabWidth: 2 });
      editor.dispatch({ changes: { from: range.from, to: range.to, insert: formatted } });
      setNote(null);
    } catch {
      setNote("This SQL couldn't be formatted. Check it for a syntax error.");
    }
  };

  const persist = async (title: string) => {
    try {
      const saved = await commands.saveQuery(tab.savedId, title, tab.sql);
      updateQuery(tab.id, { title: saved.name, savedId: saved.id });
      setNaming(null);
      setNote(`Saved as “${saved.name}”.`);
      void queries.invalidateQueries({ queryKey: ["saved"] });
    } catch (error) {
      setNote(describe(error));
    }
  };

  const save = () => (tab.savedId ? void persist(tab.title) : setNaming(tab.title.startsWith("Query ") ? "" : tab.title));
  const cancel = () => void commands.cancelRun(id);
  const keys = { run: () => void execute(false), runAll: () => void execute(true), format, save, cancel };

  useShortcuts({
    "$mod+Enter": keys.run,
    "$mod+Shift+Enter": keys.runAll,
    "$mod+Period": keys.cancel,
    "$mod+KeyS": keys.save,
    "$mod+Shift+KeyF": keys.format,
  });

  const running = run.phase === "running";
  const status = running
    ? `Running… ${((now - run.startedAt) / 1000).toFixed(1)} s`
    : run.phase === "done"
      ? `${run.outcome.results.length} of ${run.statements.length} ${run.statements.length === 1 ? "statement" : "statements"} · ${Math.round(run.outcome.elapsedMs)} ms`
      : "";

  return (
    <div className="query-tab">
      <SqlEditor value={tab.sql} onChange={(sql) => updateQuery(tab.id, { sql })} schema={names} keys={keys} view={view} />
      <div className="run-bar">
        <button
          type="button"
          className="btn btn-primary"
          disabled={running}
          onClick={keys.run}
          title={`Run the statement at the cursor (${modKey}↵)`}
        >
          <Play size={11} fill="currentColor" strokeWidth={0} aria-hidden="true" />
          Run
        </button>
        <button type="button" className="btn btn-quiet" disabled={running} onClick={keys.runAll} title={`Run every statement (⇧${modKey}↵)`}>
          Run all
        </button>
        <button type="button" className="btn btn-quiet" disabled={running} onClick={() => void explain(false)} title="Show the plan">
          <ListTree size={13} strokeWidth={1.5} aria-hidden="true" />
          Explain
        </button>
        <button
          type="button"
          className="btn btn-quiet"
          disabled={running}
          onClick={() => void explain(true)}
          title="Run the statement to measure its plan, then roll back what it changed"
        >
          Analyze
        </button>
        <button type="button" className="btn btn-quiet" onClick={format} title={`Format (⇧${modKey}F)`}>
          <AlignLeft size={13} strokeWidth={1.5} aria-hidden="true" />
          Format
        </button>
        {naming === null ? (
          <button type="button" className="btn btn-quiet" onClick={save} title={`Save (${modKey}S)`}>
            <Save size={13} strokeWidth={1.5} aria-hidden="true" />
            {tab.savedId ? "Save" : "Save…"}
          </button>
        ) : (
          <form
            className="save-form"
            onSubmit={(e) => {
              e.preventDefault();
              void persist(naming);
            }}
          >
            <label htmlFor={`name-${tab.id}`} className="visually-hidden">
              Name
            </label>
            <input
              id={`name-${tab.id}`}
              className="input"
              placeholder="Name this query"
              value={naming}
              onChange={(e) => setNaming(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") setNaming(null);
              }}
              // biome-ignore lint/a11y/noAutofocus: the field appears because Save was just chosen
              autoFocus
            />
            <button type="submit" className="btn">
              Save
            </button>
          </form>
        )}
        <span className="run-bar-status" role="status">
          {note ?? status}
        </span>
        {running && (
          <button type="button" className="btn" onClick={cancel}>
            <Square size={10} fill="currentColor" strokeWidth={0} aria-hidden="true" />
            Cancel <Keys keys={[modCap, "."]} />
          </button>
        )}
      </div>
      {run.phase === "done" && run.outcome.inTransaction && (
        <p className="transaction-note">A transaction is open on this connection. Run COMMIT or ROLLBACK to end it.</p>
      )}
      <div className="query-results">
        {run.phase === "done" && <Results session={session} statements={run.statements} outcome={run.outcome} />}
        {run.phase === "plan" && <PlanView plan={run.plan} analyzed={run.analyzed} />}
        {run.phase === "error" && <p className="query-message status-error">{run.message}</p>}
        {run.phase === "idle" && (
          <p className="query-message muted">
            <Keys keys={[modCap, "↵"]} /> runs the highlighted statement, or the selection. <Keys keys={["⇧", modCap, "↵"]} /> runs them all.{" "}
            <Keys keys={[modCap, "."]} /> cancels.
          </p>
        )}
      </div>
    </div>
  );
}
