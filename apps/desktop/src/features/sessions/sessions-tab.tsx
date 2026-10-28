import { useQuery } from "@tanstack/react-query";
import { RotateCw } from "lucide-react";
import { useState } from "react";
import { commands, type SessionInfo } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import "./sessions.css";

function duration(ms: number | null) {
  if (ms === null) return "—";
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  const minutes = Math.floor(ms / 60_000);
  return minutes < 60 ? `${minutes} min` : `${Math.floor(minutes / 60)} h ${minutes % 60} min`;
}

/** Every client session on the server, from pg_stat_activity, with a way to cancel a statement or end a session. */
export function SessionsTab({ session }: { session: SessionInfo }) {
  const id = session.sessionId;
  const activity = useQuery({ queryKey: ["activity", id], queryFn: () => commands.activity(id) });
  const [confirming, setConfirming] = useState<{ pid: number; terminate: boolean } | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const readOnly = session.connection.readOnly;

  const stop = async (pid: number, terminate: boolean) => {
    setConfirming(null);
    try {
      const done = await commands.stopSession(id, pid, terminate);
      setNote(done ? (terminate ? `Ended session ${pid}.` : `Cancelled session ${pid}’s statement.`) : `Session ${pid} had already finished.`);
      void activity.refetch();
    } catch (error) {
      setNote(describe(error));
    }
  };

  return (
    <div className="sessions-tab">
      <div className="toolbar">
        <h2 className="sessions-title">Sessions on {session.server.database}</h2>
        <span className="sessions-note muted">
          {note ?? (readOnly ? "Read-only: this connection doesn’t stop other sessions." : "Cancel stops a statement. End closes the whole session.")}
        </span>
        <button type="button" className="btn btn-icon" aria-label="Refresh" onClick={() => void activity.refetch()}>
          <RotateCw size={14} strokeWidth={1.5} aria-hidden="true" />
        </button>
      </div>
      {activity.isError && <p className="structure-note status-error">{describe(activity.error)}</p>}
      <div className="sessions-body">
        <table className="sessions-table">
          <thead>
            <tr>
              <th>Process</th>
              <th>User</th>
              <th>Database</th>
              <th>Application</th>
              <th>State</th>
              <th>Running for</th>
              <th>Query</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {(activity.data ?? []).map((a) => (
              <tr key={a.pid}>
                <td className="mono">{a.pid}</td>
                <td>{a.user ?? "—"}</td>
                <td>{a.database ?? "—"}</td>
                <td className="muted">{a.application || "—"}</td>
                <td>
                  {a.state ?? "—"}
                  {a.waitingOn && <span className="muted"> · {a.waitingOn}</span>}
                </td>
                <td className="sessions-number">{a.state === "active" ? duration(a.durationMs) : "—"}</td>
                <td className="sessions-query mono" title={a.query}>
                  {a.query}
                </td>
                <td className="sessions-actions">
                  {confirming?.pid === a.pid ? (
                    <>
                      <button type="button" className="btn btn-quiet" onClick={() => setConfirming(null)}>
                        Keep
                      </button>
                      <button type="button" className="btn btn-danger" onClick={() => void stop(a.pid, confirming.terminate)}>
                        {confirming.terminate ? "End it" : "Cancel it"}
                      </button>
                    </>
                  ) : (
                    <>
                      <button
                        type="button"
                        className="btn btn-quiet"
                        disabled={readOnly}
                        onClick={() => setConfirming({ pid: a.pid, terminate: false })}
                      >
                        Cancel
                      </button>
                      <button
                        type="button"
                        className="btn btn-quiet"
                        disabled={readOnly}
                        onClick={() => setConfirming({ pid: a.pid, terminate: true })}
                      >
                        End
                      </button>
                    </>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
