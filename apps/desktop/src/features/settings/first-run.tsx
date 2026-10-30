import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { commands } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { Glyph } from "../../ui/glyph.tsx";
import "./settings.css";

/** Shown once, at the first launch: Docker databases and backups, the two choices worth making before anything else. */
export function FirstRun() {
  const queries = useQueryClient();
  const found = useQuery({ queryKey: ["findDocker"], queryFn: commands.findDocker });
  // Docker databases start on when Docker is on this computer, until the person flips the switch.
  const [chosenDocker, setDocker] = useState<boolean | null>(null);
  const docker = chosenDocker ?? (found.data !== undefined && found.data !== null);
  const [backups, setBackups] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const dockerNote = found.isPending
    ? "Looking for Docker on this computer…"
    : found.data
      ? `Shows the Postgres containers on this computer as connections, and makes new ones. Docker is running in ${found.data}.`
      : "Shows the Postgres containers on this computer as connections. Halfnine can't find Docker running now.";

  const finish = async () => {
    setBusy(true);
    setError(null);
    try {
      await commands.finishSetup(docker, backups);
      void queries.invalidateQueries({ queryKey: ["settings"] });
      void queries.invalidateQueries({ queryKey: ["docker"] });
      void queries.invalidateQueries({ queryKey: ["connections"] });
    } catch (e) {
      setError(describe(e));
      setBusy(false);
    }
  };

  return (
    <div className="first-run">
      <div className="traffic-lights-space" data-tauri-drag-region />
      <form
        className="first-run-card"
        onSubmit={(e) => {
          e.preventDefault();
          void finish();
        }}
      >
        <Glyph size={56} />
        <h1>Welcome to Halfnine</h1>
        <p className="muted">Two choices to start with. Settings has both, to change later.</p>
        <div className="group">
          <div className="row row-switch">
            <span id="first-docker" className="row-label">
              Docker databases
            </span>
            <span className="field-note">{dockerNote}</span>
            <button type="button" className="switch" aria-labelledby="first-docker" aria-pressed={docker} onClick={() => setDocker(!docker)} />
          </div>
          <div className="row row-switch">
            <span id="first-backups" className="row-label">
              Back up first
            </span>
            <span className="field-note">
              Before a table is dropped or emptied, a column dropped, or a Docker database deleted, so it can be undone.
            </span>
            <button type="button" className="switch" aria-labelledby="first-backups" aria-pressed={backups} onClick={() => setBackups(!backups)} />
          </div>
        </div>
        {error && (
          <p className="status-error" role="alert">
            {error}
          </p>
        )}
        <button type="submit" className="btn btn-primary first-run-continue" disabled={busy}>
          Continue
        </button>
      </form>
    </div>
  );
}
