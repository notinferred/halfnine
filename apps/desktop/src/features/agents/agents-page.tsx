import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { type ActivityEntry, commands, type Outcome } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { ago } from "../../ui/time.ts";
import "../queries/queries.css";
import "./agents.css";

const OUTCOME: Record<Outcome, { word: string; tone: "status-ok" | "status-error" | "muted" }> = {
  ok: { word: "Done", tone: "status-ok" },
  approved: { word: "Approved", tone: "status-ok" },
  waiting: { word: "Waiting", tone: "muted" },
  declined: { word: "Declined", tone: "muted" },
  withdrawn: { word: "Withdrawn", tone: "muted" },
  refused: { word: "Refused", tone: "muted" },
  paused: { word: "Paused", tone: "muted" },
  busy: { word: "Busy", tone: "muted" },
  conflict: { word: "Conflict", tone: "status-error" },
  error: { word: "Failed", tone: "status-error" },
};

/** The Agents page: which agents are connected, everything they did, and one switch that pauses them all. */
export function AgentsPage() {
  const queries = useQueryClient();
  const agents = useQuery({ queryKey: ["agents"], queryFn: commands.agentState });
  const activity = useQuery({ queryKey: ["agentActivity"], queryFn: () => commands.agentActivity(300) });
  const [error, setError] = useState<string | null>(null);
  const paused = agents.data?.paused ?? false;
  const clients = agents.data?.clients ?? [];

  const pause = async () => {
    setError(null);
    try {
      await commands.setAgentsPaused(!paused);
      void queries.invalidateQueries({ queryKey: ["agents"] });
    } catch (e) {
      setError(describe(e));
    }
  };

  return (
    <div className="queries-page">
      <header className="queries-header">
        <h1>Agents</h1>
        <div className="agents-pause">
          <span id="agents-pause-label">Pause agents</span>
          <button type="button" className="switch" aria-labelledby="agents-pause-label" aria-pressed={paused} onClick={() => void pause()} />
        </div>
      </header>
      <p className="queries-empty muted agents-status">
        {paused
          ? "Paused: agents can't read or change anything until you turn this off."
          : clients.length === 0
            ? "No agent is connected now."
            : clients.map((c) => `${c.agent} connected ${ago(c.sinceMs)}`).join(" · ")}
      </p>
      {(error ?? activity.error) && <p className="queries-empty status-error">{error ?? describe(activity.error)}</p>}
      <Activity entries={activity.data ?? []} />
    </div>
  );
}

function Activity({ entries }: { entries: ActivityEntry[] }) {
  if (entries.length === 0) return <p className="queries-empty muted">Nothing yet. Every call an agent makes is listed here.</p>;
  return (
    <ul className="queries-list">
      {entries.map((entry) => {
        const outcome = entry.outcome ? OUTCOME[entry.outcome] : null;
        const where = [entry.connection, entry.database].filter(Boolean).join(" · ");
        const counted = entry.rows != null && entry.rows > 0 ? ` · ${entry.rows.toLocaleString()} rows` : "";
        const took = entry.elapsedMs != null ? ` · ${entry.elapsedMs.toLocaleString()} ms` : "";
        return (
          <li key={entry.call} className="queries-item">
            <details className="agents-entry">
              <summary className="queries-open">
                <span className="queries-run">
                  {outcome && <span className={`queries-outcome ${outcome.tone}`}>{outcome.word}</span>}
                  <span className="muted">
                    {entry.agent} · {entry.tool}
                    {where && ` · ${where}`} · {ago(entry.atMs)}
                    {counted}
                    {took}
                  </span>
                </span>
                {entry.sql.length > 0 && <span className="queries-sql">{entry.sql.join("\n")}</span>}
                {entry.reason && <span className="agents-reason">“{entry.reason}”</span>}
              </summary>
              {entry.sql.length > 0 && <pre className="review-sql mono agents-sql">{entry.sql.join("\n\n")}</pre>}
            </details>
          </li>
        );
      })}
    </ul>
  );
}
