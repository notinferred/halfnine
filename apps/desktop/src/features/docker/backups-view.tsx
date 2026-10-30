import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { type Backup, commands } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { openOn, useWorkspace } from "../../state.ts";
import { ago } from "../../ui/time.ts";
import { sizeInWords } from "../settings/settings-page.tsx";
import { doneInWords, inWords, putBack } from "../workspace/backup-note.ts";

/** Every backup, newest first: what it holds and why it was made, with Restore and Delete. */
export function BackupsView() {
  const backups = useQuery({ queryKey: ["backups"], queryFn: commands.listBackups });
  const list = backups.data?.backups ?? [];
  if (backups.error) return <p className="queries-empty status-error">{describe(backups.error)}</p>;
  if (backups.data && list.length === 0) {
    return (
      <p className="queries-empty muted">
        No backups yet. With backups on in Settings, a table is backed up before it is dropped or emptied, a column before it is dropped, and a Docker
        database before it is deleted, so each can be put back.
      </p>
    );
  }
  return (
    <ul className="queries-list">
      {list.map((backup) => (
        <BackupRow key={backup.id} backup={backup} />
      ))}
    </ul>
  );
}

/** One backup: what it holds, with Restore, which shows what it runs first, and Delete. */
export function BackupRow(props: { backup: Backup }) {
  const { backup } = props;
  const { holds } = backup;
  const queries = useQueryClient();
  // A table backup goes back through a session on its database, which any connection open in the window may hold.
  const session = useWorkspace((s) => (holds.kind === "table" ? openOn(s, holds.connectionId, holds.database) : null));
  const here = session !== null;
  const [reviewing, setReviewing] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const review = useQuery({
    queryKey: ["review-restore", session?.sessionId, backup.id],
    queryFn: () => commands.reviewRestore(session?.sessionId ?? "", backup.id),
    enabled: reviewing && here,
  });
  const what =
    holds.kind === "table"
      ? [`${(holds.rows ?? 0).toLocaleString()} ${holds.rows === 1 ? "row" : "rows"}`, `${holds.database} on ${holds.connectionName}`]
      : [`${holds.container.databases.length} ${holds.container.databases.length === 1 ? "database" : "databases"}`, holds.container.image];
  const detail = [
    ago(backup.madeAtMs),
    ...what,
    sizeInWords(backup.bytes ?? 0),
    backup.agent ? `by ${backup.agent.title}` : null,
    backup.restoredAtMs === null ? null : `put back ${ago(backup.restoredAtMs)}`,
  ]
    .filter(Boolean)
    .join(" · ");

  const restore = async () => {
    setBusy(true);
    await putBack(queries, backup, here ? (session?.sessionId ?? null) : null);
    setBusy(false);
    setReviewing(false);
  };

  const remove = async () => {
    try {
      await commands.deleteBackup(backup.id);
    } catch (e) {
      setError(describe(e));
    } finally {
      setConfirming(false);
      void queries.invalidateQueries({ queryKey: ["backups"] });
    }
  };

  const restorable = holds.kind === "database" || here;
  return (
    <li className="queries-item backup-row">
      <div className="backup-head">
        <span className="docker-row-text">
          <span className="queries-name">{doneInWords(holds)}</span>
          <span className="muted docker-detail">{detail}</span>
          {backup.notKept.length > 0 && <span className="muted docker-detail">It doesn't keep {inWords(backup.notKept)}.</span>}
        </span>
        {confirming ? (
          <span className="docker-actions">
            <button type="button" className="btn btn-quiet" onClick={() => setConfirming(false)}>
              Keep
            </button>
            <button type="button" className="btn btn-danger" onClick={() => void remove()}>
              Delete
            </button>
          </span>
        ) : (
          <span className="docker-actions">
            <button
              type="button"
              className="btn btn-quiet"
              disabled={!restorable}
              title={holds.kind === "table" && !here ? `Connect to ${holds.database} on ${holds.connectionName} to put it back` : undefined}
              aria-expanded={reviewing}
              onClick={() => setReviewing(!reviewing)}
            >
              Restore…
            </button>
            <button type="button" className="btn btn-quiet" onClick={() => setConfirming(true)}>
              Delete
            </button>
          </span>
        )}
      </div>
      {reviewing && restorable && (
        <div className="backup-review">
          {holds.kind === "table" ? (
            <pre className="review-sql mono">
              {review.isError ? describe(review.error) : review.data ? ["BEGIN;", ...review.data, "COMMIT;"].join("\n") : "…"}
            </pre>
          ) : (
            <p className="muted docker-detail">
              Makes a container named {holds.container.name}, or {holds.container.name}-restored if that's taken, from {holds.container.image}
              {holds.container.port === null ? "" : `, on port ${holds.container.port} if it's free`}, and restores{" "}
              {inWords(holds.container.databases)} into it.
            </p>
          )}
          <span className="docker-actions">
            <button type="button" className="btn btn-quiet" onClick={() => setReviewing(false)}>
              Cancel
            </button>
            <button
              type="button"
              className="btn btn-primary"
              disabled={busy || (holds.kind === "table" && !review.data)}
              onClick={() => void restore()}
            >
              {busy ? "Putting it back…" : "Put it back"}
            </button>
          </span>
        </div>
      )}
      {error && <p className="status-error docker-detail">{error}</p>}
    </li>
  );
}
