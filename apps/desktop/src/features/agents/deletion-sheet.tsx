import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { commands, type DeletionRequestView } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { useWorkspace } from "../../state.ts";
import { showNote } from "../../ui/note.tsx";
import { noteUndo } from "../workspace/backup-note.ts";
import { isTyping, useArmed } from "./approval-bar.tsx";

/**
 * An agent asking to delete a Docker database it didn't make: 1. Yes or 3. No, as the approval bar asks, with no
 * "don't ask again", which never covers a delete. The choices work only once the window has had focus for a second.
 */
export function DeletionSheet(props: { request: DeletionRequestView }) {
  const { request } = props;
  const { name } = request.connection;
  const queries = useQueryClient();
  const armed = useArmed();
  const [error, setError] = useState<string | null>(null);
  const deleting = request.state.kind === "deleting";

  const answer = async (yes: boolean) => {
    if (!armed || deleting) return;
    setError(null);
    try {
      const done = await commands.answerDeletion(request.id, yes);
      if (!done) return;
      // The window was on that database, which is gone now.
      const { session, leave } = useWorkspace.getState();
      if (session?.connection.id === request.connection.id) leave();
      const kept = done.keptVolume ? " Its data volume wasn't made by Halfnine, so it was kept." : "";
      if (done.backup) noteUndo(queries, done.backup, null, kept);
      else showNote({ text: `Deleted ${name}.${kept}` });
    } catch (e) {
      setError(describe(e));
    } finally {
      void queries.invalidateQueries({ queryKey: ["docker"] });
      void queries.invalidateQueries({ queryKey: ["connections"] });
    }
  };

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (isTyping(event.target) || event.metaKey || event.ctrlKey || event.altKey) return;
      if (event.key === "1" || event.key === "3") {
        event.preventDefault();
        void answer(event.key === "1");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  return (
    <div className="sheet-layer">
      <div className="sheet-scrim" aria-hidden="true" />
      <div className="sheet" role="alertdialog" aria-modal="true" aria-labelledby="deletion-title" aria-describedby="deletion-detail">
        <h2 id="deletion-title">
          {request.agent} wants to delete {name}
        </h2>
        <p id="deletion-detail" className="muted">
          {request.agent} didn't make this Docker database.{" "}
          {request.backedUp
            ? "Halfnine backs it up first, so you can put it back from the Docker page's backups."
            : "Backups are off in Settings, so it can't be put back."}
        </p>
        {error && (
          <p className="status-error" role="alert">
            {error}
          </p>
        )}
        <fieldset className="approval-choices sheet-actions" disabled={!armed || deleting} aria-label={`Answer ${request.agent}`}>
          {deleting ? (
            <span className="muted">{request.backedUp ? "Backing up, then deleting…" : "Deleting…"}</span>
          ) : (
            <>
              <button type="button" className="btn" onClick={() => void answer(false)}>
                <kbd>3</kbd> No
              </button>
              <button type="button" className="btn btn-danger btn-danger-strong" onClick={() => void answer(true)}>
                <kbd>1</kbd> Yes, delete it
              </button>
            </>
          )}
        </fieldset>
      </div>
    </div>
  );
}
