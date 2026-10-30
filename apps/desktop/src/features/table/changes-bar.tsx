import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { type Changes, type CommitDone, type Committed, commands, type SessionInfo } from "../../ipc/bindings.ts";
import { describe, notBackedUp } from "../../ipc/errors.ts";
import { Keys } from "../../ui/keys.tsx";
import { modCap, useShortcuts } from "../../ui/shortcuts.ts";
import { noteUndo } from "../workspace/backup-note.ts";
import { ConfirmByName } from "../workspace/confirm-by-name.tsx";

/**
 * The bar under staged changes: what is staged, the SQL it will run, and Commit (⌘S) or Discard. Nothing is written
 * until Commit, and then everything is written in one transaction or nothing is.
 */
export function ChangesBar(props: {
  session: SessionInfo;
  changes: Changes;
  summary: string;
  /** Says what one transaction promises for these changes, above the SQL. */
  promise: string;
  commitLabel?: string;
  /** Someone tried to close the tab, or leave the connection, with these changes staged. */
  held?: boolean;
  onDiscard: () => void;
  onCommitted: (done: Committed) => void;
}) {
  const id = props.session.sessionId;
  const queries = useQueryClient();
  const [reviewing, setReviewing] = useState(false);
  /** Why dropping a column can't be backed up, while its name is asked for instead. */
  const [unbacked, setUnbacked] = useState<string | null>(null);
  const [committing, setCommitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const review = useQuery({
    queryKey: ["review", id, props.changes],
    queryFn: () => commands.reviewChanges(id, props.changes),
    enabled: reviewing,
  });

  const commit = async () => {
    if (committing) return;
    setCommitting(true);
    setError(null);
    try {
      committed(await commands.commitChanges(id, props.changes, false));
    } catch (e) {
      if (notBackedUp(e)) setUnbacked(describe(e));
      else setError(describe(e));
    } finally {
      setCommitting(false);
    }
  };

  const committed = (done: CommitDone) => {
    setReviewing(false);
    setUnbacked(null);
    if (done.backup) {
      noteUndo(queries, id, done.backup);
      void queries.invalidateQueries({ queryKey: ["backups"] });
    }
    props.onCommitted(done.committed);
  };

  useShortcuts({ "$mod+s": () => void commit() });

  return (
    <div className="changes">
      {reviewing && (
        <div className="review">
          <p className="muted">{props.promise}</p>
          <pre className="review-sql mono">
            {review.isError ? describe(review.error) : review.data ? ["BEGIN;", ...review.data, "COMMIT;"].join("\n") : "…"}
          </pre>
        </div>
      )}
      {(error ?? props.held) && (
        <p className="changes-note status-error" role="alert">
          {error ?? "Commit or discard these changes first, so they aren't lost."}
        </p>
      )}
      <div className="changes-bar">
        <span className="changes-dot" aria-hidden="true" />
        <span className="changes-summary">{props.summary}</span>
        <button type="button" className="btn btn-quiet changes-review" aria-expanded={reviewing} onClick={() => setReviewing(!reviewing)}>
          Review SQL
        </button>
        {committing ? (
          <button type="button" className="btn" onClick={() => void commands.cancelRun(id)}>
            Stop
          </button>
        ) : (
          <button type="button" className="btn" onClick={props.onDiscard}>
            Discard
          </button>
        )}
        <button type="button" className="btn btn-primary" disabled={committing} onClick={() => void commit()}>
          {committing ? "Committing…" : (props.commitLabel ?? "Commit")}
          {!committing && <Keys keys={[modCap, "S"]} />}
        </button>
      </div>
      {unbacked !== null && props.changes.kind === "structure" && (
        <ConfirmByName
          session={props.session}
          changes={props.changes}
          kind="table"
          why={unbacked}
          onClose={() => setUnbacked(null)}
          onDone={committed}
        />
      )}
    </div>
  );
}

/** The dot beside something that holds staged changes, with words for screen readers. */
export function StagedMark() {
  return (
    <>
      <span className="changes-dot" aria-hidden="true" />
      <span className="visually-hidden">, with staged changes</span>
    </>
  );
}

export const stagedSummary = (count: number) => `${count.toLocaleString()} staged ${count === 1 ? "change" : "changes"}`;
