import { useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { type Changes, type CommitDone, commands, type RelationKind, type SessionInfo, type StructureChange } from "../../ipc/bindings.ts";
import { brokeRule, describe } from "../../ipc/errors.ts";
import { inWords } from "./backup-note.ts";

const NOUN: Record<RelationKind, string> = {
  table: "table",
  partitionedTable: "table",
  foreignTable: "foreign table",
  view: "view",
  materializedView: "materialized view",
};

type Structure = Extract<Changes, { kind: "structure" }>;

/**
 * A drop, truncate or column drop that Halfnine couldn't back up first can't be undone, so it says why, shows the
 * statements, and waits for the table's name to be typed. Then it runs without a backup.
 */
export function ConfirmByName(props: {
  session: SessionInfo;
  changes: Structure;
  kind: RelationKind;
  /** Why there's no backup to undo it with. */
  why: string;
  onDone: (done: CommitDone) => void;
  onClose: () => void;
}) {
  const id = props.session.sessionId;
  const { name } = props.changes.table;
  const [typed, setTyped] = useState("");
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.focus(), []);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** The rule this breaks, once the person has seen it and may commit anyway. */
  const [broken, setBroken] = useState(false);
  const statements = useQuery({ queryKey: ["review", id, props.changes], queryFn: () => commands.reviewChanges(id, props.changes) });
  const { title, consequence, button, doing } = words(props.changes.changes, name, NOUN[props.kind]);

  const run = async (despiteRules = false) => {
    setBusy(true);
    setError(null);
    try {
      props.onDone(await commands.commitChanges(id, props.changes, true, despiteRules));
    } catch (e) {
      setError(describe(e));
      setBroken(brokeRule(e));
      setBusy(false);
    }
  };

  return (
    <div className="sheet-layer">
      <button type="button" className="sheet-scrim" aria-label="Close" tabIndex={-1} onClick={props.onClose} />
      <form
        className="sheet"
        role="dialog"
        aria-modal="true"
        aria-labelledby="confirm-title"
        onSubmit={(e) => {
          e.preventDefault();
          if (typed === name && !busy) void run();
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape") props.onClose();
        }}
      >
        <h2 id="confirm-title">{title}</h2>
        <p className="muted">
          {consequence} It can't be undone. {props.why}
        </p>
        <pre className="review-sql mono">{statements.data?.join("\n") ?? "…"}</pre>
        <label className="sheet-label" htmlFor="confirm-name">
          Type <strong>{name}</strong> to confirm
        </label>
        <input
          id="confirm-name"
          ref={input}
          className="input"
          spellCheck={false}
          autoComplete="off"
          value={typed}
          onChange={(e) => setTyped(e.target.value)}
        />
        {error && (
          <p className="status-error" role="alert">
            {error}
            {broken && (
              <>
                {" "}
                <button type="button" className="btn btn-quiet" disabled={busy} onClick={() => void run(true)}>
                  Commit anyway
                </button>
              </>
            )}
          </p>
        )}
        <div className="sheet-actions">
          <button type="button" className="btn" onClick={props.onClose}>
            Cancel
          </button>
          <button type="submit" className="btn btn-danger btn-danger-strong" disabled={typed !== name || busy}>
            {busy ? doing : button}
          </button>
        </div>
      </form>
    </div>
  );
}

function words(changes: StructureChange[], name: string, noun: string) {
  if (changes.some((c) => c.kind === "drop")) {
    return {
      title: `Drop ${name}?`,
      consequence: `This deletes the ${noun} ${name}${noun === "view" ? "" : " and every row in it"}.`,
      button: `Drop ${noun}`,
      doing: "Dropping…",
    };
  }
  if (changes.some((c) => c.kind === "truncate")) {
    return {
      title: `Truncate ${name}?`,
      consequence: `This deletes every row in ${name}. The table and its structure stay.`,
      button: `Truncate ${noun}`,
      doing: "Truncating…",
    };
  }
  const columns = changes.flatMap((c) => (c.kind === "dropColumn" ? [c.column] : []));
  return {
    title: `Drop ${inWords(columns)} from ${name}?`,
    consequence: `This deletes ${columns.length > 1 ? "these columns and every value in them" : "the column and every value in it"}.`,
    button: "Commit",
    doing: "Committing…",
  };
}
