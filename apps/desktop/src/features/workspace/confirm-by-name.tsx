import { useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { type Changes, commands, type RelationKind, type SessionInfo, type TableRef } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";

const NOUN: Record<RelationKind, string> = {
  table: "table",
  partitionedTable: "table",
  foreignTable: "foreign table",
  view: "view",
  materializedView: "materialized view",
};

/** Truncating or dropping can't be undone, so it asks for the table's name to be typed first, and shows the statement. */
export function ConfirmByName(props: {
  session: SessionInfo;
  table: TableRef;
  kind: RelationKind;
  action: "truncate" | "drop";
  onDone: () => void;
  onClose: () => void;
}) {
  const id = props.session.sessionId;
  const { name } = props.table;
  const [typed, setTyped] = useState("");
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.focus(), []);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const changes: Changes = { kind: "structure", table: props.table, changes: [{ kind: props.action }] };
  const statement = useQuery({ queryKey: ["review", id, changes], queryFn: () => commands.reviewChanges(id, changes) });
  const noun = NOUN[props.kind];
  const verb = props.action === "drop" ? "Drop" : "Truncate";

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      await commands.commitChanges(id, changes, true);
      props.onDone();
    } catch (e) {
      setError(describe(e));
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
        <h2 id="confirm-title">
          {verb} {name}?
        </h2>
        <p className="muted">
          {props.action === "drop"
            ? `This deletes the ${noun} ${name}${noun === "view" ? "" : " and every row in it"}. It can't be undone.`
            : `This deletes every row in ${name}. The table and its structure stay. It can't be undone.`}
        </p>
        <pre className="review-sql mono">{statement.data?.join("\n") ?? "…"}</pre>
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
          </p>
        )}
        <div className="sheet-actions">
          <button type="button" className="btn" onClick={props.onClose}>
            Cancel
          </button>
          <button type="submit" className="btn btn-danger btn-danger-strong" disabled={typed !== name || busy}>
            {busy ? `${verb === "Drop" ? "Dropping" : "Truncating"}…` : `${verb} ${noun}`}
          </button>
        </div>
      </form>
    </div>
  );
}
