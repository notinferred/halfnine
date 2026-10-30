import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { commands } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import "../queries/queries.css";
import "./settings.css";

const LIMITS = [100_000_000, 1_000_000_000, 10_000_000_000];

/** A size as Finder writes it, in powers of 1,000: "820 KB", "3.2 GB". */
export function sizeInWords(bytes: number): string {
  const units = ["bytes", "KB", "MB", "GB", "TB"];
  let size = bytes;
  let unit = 0;
  while (size >= 1000 && unit < units.length - 1) {
    size /= 1000;
    unit += 1;
  }
  const number = unit > 0 && size < 10 ? size.toFixed(1).replace(/\.0$/, "") : size.toFixed(0);
  return `${number} ${units[unit]}`;
}

/** The app's few choices, in the grouped rows the connection form uses. */
export function SettingsPage() {
  const queries = useQueryClient();
  const settings = useQuery({ queryKey: ["settings"], queryFn: commands.getSettings });
  const docker = useQuery({ queryKey: ["docker"], queryFn: commands.dockerState });
  const backups = useQuery({ queryKey: ["backups"], queryFn: commands.listBackups });
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
      void queries.invalidateQueries({ queryKey: ["backups"] });
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
              <span id="settings-backups" className="row-label">
                Back up first
              </span>
              <span className="field-note">
                Before a table is dropped or emptied, or a column dropped, so it can be undone.
                {backups.data && backups.data.backups.length > 0 && ` Backups take ${sizeInWords(backups.data.bytes ?? 0)}.`}
              </span>
              <button
                type="button"
                className="switch"
                aria-labelledby="settings-backups"
                aria-pressed={choices.backups}
                onClick={() => void change(() => commands.setBackupsEnabled(!choices.backups))}
              />
            </div>
            {choices.backups && (
              <div className="row">
                <label htmlFor="settings-limit" className="row-label">
                  Tables up to
                </label>
                <span className="field-note">A bigger table asks for its name instead, so a drop never holds others up for long.</span>
                <select
                  id="settings-limit"
                  value={String(choices.backupLimitBytes ?? "none")}
                  onChange={(e) => {
                    const bytes = e.target.value === "none" ? null : Number(e.target.value);
                    void change(() => commands.setBackupLimit(bytes));
                  }}
                >
                  {LIMITS.map((bytes) => (
                    <option key={bytes} value={bytes}>
                      {sizeInWords(bytes)}
                    </option>
                  ))}
                  <option value="none">No limit</option>
                </select>
              </div>
            )}
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
