import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Plus } from "lucide-react";
import { useState } from "react";
import { commands, type HistoryEntry, type SavedQuery, type SessionInfo } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { useWorkspace } from "../../state.ts";
import { Keys } from "../../ui/keys.tsx";
import { modCap } from "../../ui/shortcuts.ts";
import { ago } from "../../ui/time.ts";
import "./queries.css";

/** The Queries page: every saved query, and with one switch, everything that has run on this connection. */
export function QueriesPage({ session }: { session: SessionInfo }) {
  const newQuery = useWorkspace((s) => s.newQuery);
  const [showing, setShowing] = useState<"saved" | "history">("saved");
  const saved = useQuery({ queryKey: ["saved"], queryFn: commands.listSavedQueries });
  const history = useQuery({ queryKey: ["history"], queryFn: commands.listHistory });
  const runs = (history.data ?? []).filter((h) => h.connectionId === session.connection.id);
  const failure = saved.error ?? history.error;

  return (
    <div className="queries-page">
      <header className="queries-header">
        <h1>Queries</h1>
        <fieldset className="segmented" aria-label="Show">
          <button type="button" aria-pressed={showing === "saved"} onClick={() => setShowing("saved")}>
            Saved
          </button>
          <button type="button" aria-pressed={showing === "history"} onClick={() => setShowing("history")}>
            History
          </button>
        </fieldset>
        <button type="button" className="btn btn-primary queries-new" onClick={() => newQuery()}>
          <Plus size={13} strokeWidth={2} aria-hidden="true" />
          New query
        </button>
      </header>
      {failure && <p className="queries-empty status-error">{describe(failure)}</p>}
      {showing === "saved" ? (
        <Saved queries={saved.data ?? []} onOpen={(q) => newQuery(q.sql, q.name, q.id)} />
      ) : (
        <History runs={runs} onOpen={(h) => newQuery(h.sql)} />
      )}
    </div>
  );
}

function Saved(props: { queries: SavedQuery[]; onOpen: (query: SavedQuery) => void }) {
  const queries = useQueryClient();
  const [confirming, setConfirming] = useState<string | null>(null);

  if (props.queries.length === 0) {
    return (
      <p className="queries-empty muted">
        No saved queries yet. Save one from a query tab with <Keys keys={[modCap, "S"]} />.
      </p>
    );
  }

  const forget = async (id: string) => {
    await commands.deleteSavedQuery(id);
    setConfirming(null);
    void queries.invalidateQueries({ queryKey: ["saved"] });
  };

  return (
    <ul className="queries-list">
      {props.queries.map((q) => (
        <li key={q.id} className="queries-item">
          <button type="button" className="queries-open" onClick={() => props.onOpen(q)}>
            <span className="queries-name">{q.name}</span>
            <span className="queries-sql">{q.sql}</span>
          </button>
          {confirming === q.id ? (
            <span className="queries-confirm">
              <button type="button" className="btn btn-quiet" onClick={() => setConfirming(null)}>
                Keep
              </button>
              <button type="button" className="btn btn-danger" onClick={() => void forget(q.id)}>
                Delete
              </button>
            </span>
          ) : (
            <button type="button" className="btn btn-quiet queries-delete" onClick={() => setConfirming(q.id)}>
              Delete
            </button>
          )}
        </li>
      ))}
    </ul>
  );
}

function History(props: { runs: HistoryEntry[]; onOpen: (run: HistoryEntry) => void }) {
  if (props.runs.length === 0) return <p className="queries-empty muted">Nothing has run on this connection yet.</p>;
  return (
    <ul className="queries-list">
      {props.runs.map((run) => (
        <li key={run.id} className="queries-item">
          <button type="button" className="queries-open" onClick={() => props.onOpen(run)}>
            <span className="queries-run">
              <span className={`queries-outcome ${run.ok ? "status-ok" : "status-error"}`}>{run.ok ? "Ran" : "Failed"}</span>
              <span className="muted">
                {ago(run.ranAtMs)} · {run.elapsedMs.toLocaleString()} ms · {run.database}
              </span>
            </span>
            <span className="queries-sql">{run.sql}</span>
          </button>
        </li>
      ))}
    </ul>
  );
}
