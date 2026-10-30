import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { type Backup, commands } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { useWorkspace } from "../../state.ts";
import { showNote } from "../../ui/note.tsx";
import { ago } from "../../ui/time.ts";
import { sizeInWords } from "../settings/settings-page.tsx";
import { doneInWords, inWords, refreshTable } from "../workspace/backup-note.ts";

/** Every backup, newest first: what it holds and why it was made, with Restore and Delete. */
export function BackupsView() {
  const backups = useQuery({ queryKey: ["backups"], queryFn: commands.listBackups });
  const list = backups.data?.backups ?? [];
  if (backups.error) return <p className="queries-empty status-error">{describe(backups.error)}</p>;
  if (backups.data && list.length === 0) {
    return (
      <p className="queries-empty muted">
        No backups yet. With backups on in Settings, a table is backed up before it is dropped or emptied, or a column before it is dropped, so it can
        be put back.
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

function BackupRow(props: { backup: Backup }) {
  const { backup } = props;
  const { holds } = backup;
  const queries = useQueryClient();
  const session = useWorkspace((s) => s.session);
  const here = session?.connection.id === holds.connectionId && session.server.database === holds.database ? session : null;
  const [reviewing, setReviewing] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const review = useQuery({
    queryKey: ["review-restore", here?.sessionId, backup.id],
    queryFn: () => commands.reviewRestore(here?.sessionId ?? "", backup.id),
    enabled: reviewing && here !== null,
  });
  const detail = [
    `${ago(backup.madeAtMs)}`,
    `${(holds.rows ?? 0).toLocaleString()} ${holds.rows === 1 ? "row" : "rows"}`,
    sizeInWords(backup.bytes ?? 0),
    `${holds.database} on ${holds.connectionName}`,
    backup.agent ? `by ${backup.agent.title}` : null,
    backup.restoredAtMs === null ? null : `put back ${ago(backup.restoredAtMs)}`,
  ]
    .filter(Boolean)
    .join(" · ");

  const restore = async () => {
    if (!here) return;
    setBusy(true);
    setError(null);
    try {
      await commands.restoreBackup(here.sessionId, backup.id);
      setReviewing(false);
      showNote({ text: `Put ${holds.table.name} back.` });
      refreshTable(queries, here.sessionId, holds.table);
    } catch (e) {
      setError(describe(e));
    } finally {
      setBusy(false);
      void queries.invalidateQueries({ queryKey: ["backups"] });
    }
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

  return (
    <li className="queries-item backup-row">
      <div className="backup-head">
        <span className="docker-row-text">
          <span className="queries-name">{doneInWords(backup)}</span>
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
              disabled={!here}
              title={here ? undefined : `Connect to ${holds.database} on ${holds.connectionName} to put it back`}
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
      {reviewing && here && (
        <div className="backup-review">
          <pre className="review-sql mono">
            {review.isError ? describe(review.error) : review.data ? ["BEGIN;", ...review.data, "COMMIT;"].join("\n") : "…"}
          </pre>
          <span className="docker-actions">
            <button type="button" className="btn btn-quiet" onClick={() => setReviewing(false)}>
              Cancel
            </button>
            <button type="button" className="btn btn-primary" disabled={busy || !review.data} onClick={() => void restore()}>
              {busy ? "Putting it back…" : "Put it back"}
            </button>
          </span>
        </div>
      )}
      {error && <p className="status-error docker-detail">{error}</p>}
    </li>
  );
}
