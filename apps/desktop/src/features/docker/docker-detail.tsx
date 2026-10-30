import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { type AgentAccess, commands, type DockerDatabase, type Label } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { AGENT_ACCESS, LABELS } from "../connections/connection-form.tsx";

/** Who made a Docker database, in words, or `null` when Halfnine didn't. */
export function madeBy(database: DockerDatabase): string | null {
  const source = database.connection.source;
  if (source?.kind !== "docker" || !source.made || source.made.how === "restored") return null;
  return source.made.agent ? `made by ${source.made.agent.title}` : "made by Halfnine";
}

/** What agents get on a Docker database until a level is picked, as `ConnectionSpec::agent_access` decides it. */
const defaultAccess = (database: DockerDatabase): AgentAccess => (database.connection.label === "red" ? "off" : madeBy(database) ? "full" : "read");

/**
 * A Docker database: what it runs and where, and what someone sets for it. Its credentials come from its container,
 * so there is nothing to type, and each change applies at once.
 */
export function DockerDetail(props: { database: DockerDatabase; opening: boolean; openError: string | null; onOpen: () => void }) {
  const { database } = props;
  const spec = database.connection;
  const queries = useQueryClient();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const made = madeBy(database);

  const act = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (e) {
      setError(describe(e));
    } finally {
      setBusy(false);
      void queries.invalidateQueries({ queryKey: ["docker"] });
      void queries.invalidateQueries({ queryKey: ["connections"] });
    }
  };
  const set = (change: Partial<{ label: Label; readOnly: boolean; agentAccess: AgentAccess | null }>) =>
    act(() =>
      commands.setDockerSettings(spec.name, {
        label: change.label ?? spec.label,
        readOnly: change.readOnly ?? spec.readOnly,
        agentAccess: change.agentAccess !== undefined ? change.agentAccess : (spec.agentAccess ?? null),
      }),
    );

  const where = database.port === null ? "publishes no port, so it can't be connected to" : `127.0.0.1:${database.port}`;
  const status = error ?? props.openError;

  return (
    <div className="connection-form">
      <header className="connection-form-header">
        <div className="connection-form-title">
          <span className="dot" data-label={spec.label} aria-hidden="true" />
          <h1>{spec.name}</h1>
        </div>
        <p className="muted">
          {database.running ? "Running" : "Stopped"} · PostgreSQL {database.major ?? "?"} · {database.image} · {where}
          {made && ` · ${made}`}
        </p>
      </header>

      <div className="group">
        <div className="row">
          <span className="row-label">Sign in</span>
          <span className="field-note">
            {spec.user} on {spec.database}, with the password its container was made with.
          </span>
        </div>
      </div>

      <div className="group">
        <div className="row">
          <span id="docker-label-heading" className="row-label">
            Label
          </span>
          <fieldset aria-labelledby="docker-label-heading" className="swatches">
            {LABELS.map((l) => (
              <button
                key={l.value}
                type="button"
                className="swatch dot"
                data-label={l.value}
                aria-label={l.name}
                aria-pressed={spec.label === l.value}
                onClick={() => void set({ label: l.value })}
              />
            ))}
          </fieldset>
        </div>
        <div className="row row-switch">
          <span id="docker-read-only" className="row-label">
            Read-only
          </span>
          <span className="field-note">Postgres refuses every write made through this connection.</span>
          <button
            type="button"
            className="switch"
            aria-labelledby="docker-read-only"
            aria-pressed={spec.readOnly}
            onClick={() => void set({ readOnly: !spec.readOnly })}
          />
        </div>
        <div className="row row-switch">
          <label htmlFor="docker-agent-access" className="row-label">
            Agents
          </label>
          <span className="field-note">
            {spec.readOnly ? "Read-only, so agents only ever read here." : "What AI agents may do here through Halfnine."}
          </span>
          <select
            id="docker-agent-access"
            value={spec.agentAccess ?? defaultAccess(database)}
            onChange={(e) => void set({ agentAccess: e.target.value as AgentAccess })}
          >
            {AGENT_ACCESS.map((a) => (
              <option key={a.value} value={a.value}>
                {a.name}
              </option>
            ))}
          </select>
        </div>
      </div>

      <footer className="connection-form-actions">
        <p role="status" className={`connection-status ${status ? "status-error" : "muted"}`}>
          {status}
        </p>
        {database.running ? (
          <button type="button" className="btn" disabled={busy} onClick={() => void act(() => commands.stopDockerDatabase(spec.name))}>
            Stop
          </button>
        ) : (
          <button type="button" className="btn" disabled={busy} onClick={() => void act(() => commands.startDockerDatabase(spec.name))}>
            Start
          </button>
        )}
        <button type="button" className="btn btn-primary" disabled={busy || props.opening || database.port === null} onClick={props.onOpen}>
          {props.opening ? "Connecting…" : "Connect"}
        </button>
      </footer>
    </div>
  );
}
