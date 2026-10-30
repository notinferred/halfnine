import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { commands } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import "../queries/queries.css";
import "./settings.css";

/** The app's few choices, in the grouped rows the connection form uses. */
export function SettingsPage() {
  const queries = useQueryClient();
  const settings = useQuery({ queryKey: ["settings"], queryFn: commands.getSettings });
  const docker = useQuery({ queryKey: ["docker"], queryFn: commands.dockerState });
  const [error, setError] = useState<string | null>(null);
  const choices = settings.data;

  const change = async (action: () => Promise<unknown>) => {
    setError(null);
    try {
      await action();
    } catch (e) {
      setError(describe(e));
    } finally {
      void queries.invalidateQueries({ queryKey: ["settings"] });
      void queries.invalidateQueries({ queryKey: ["agents"] });
      void queries.invalidateQueries({ queryKey: ["docker"] });
    }
  };
  const engine = docker.data?.engine
    ? `Shows the Postgres containers on this computer as connections. Using Docker from ${docker.data.engine}.`
    : (docker.data?.problem ?? "Shows the Postgres containers on this computer as connections.");

  return (
    <div className="queries-page">
      <header className="queries-header">
        <h1>Settings</h1>
      </header>
      {(error ?? settings.error) && <p className="queries-empty status-error">{error ?? describe(settings.error)}</p>}
      {choices && (
        <div className="settings">
          <div className="group">
            <div className="row row-switch">
              <span id="settings-docker" className="row-label">
                Docker databases
              </span>
              <span className="field-note">{choices.docker ? engine : "Shows the Postgres containers on this computer as connections."}</span>
              <button
                type="button"
                className="switch"
                aria-labelledby="settings-docker"
                aria-pressed={choices.docker}
                onClick={() => void change(() => commands.setDockerEnabled(!choices.docker))}
              />
            </div>
          </div>
          <div className="group">
            <div className="row row-switch">
              <span id="settings-pause" className="row-label">
                Pause agents
              </span>
              <span className="field-note">While on, agents can't read or change anything.</span>
              <button
                type="button"
                className="switch"
                aria-labelledby="settings-pause"
                aria-pressed={choices.agentsPaused}
                onClick={() => void change(() => commands.setAgentsPaused(!choices.agentsPaused))}
              />
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
