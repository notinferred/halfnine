import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { type ChosenFile, commands } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";

const VERSIONS = [18, 17, 16, 15, 14];

/** A name not yet taken, to start from: db, then db-2, db-3 and so on. */
function suggestedName(taken: string[]) {
  if (!taken.includes("db")) return "db";
  for (let n = 2; ; n++) if (!taken.includes(`db-${n}`)) return `db-${n}`;
}

/** Making a Docker database: a name, a Postgres version and, if wanted, a dump or SQL file to load into it. */
export function NewDatabaseSheet(props: { taken: string[]; onClose: () => void }) {
  const queries = useQueryClient();
  const [name, setName] = useState(() => suggestedName(props.taken));
  const [version, setVersion] = useState(18);
  const [file, setFile] = useState<ChosenFile | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const first = useRef<HTMLInputElement>(null);
  useEffect(() => first.current?.select(), []);

  const choose = async () => {
    try {
      const picked = await commands.chooseDatabaseFile();
      if (picked) setFile(picked);
    } catch (e) {
      setError(describe(e));
    }
  };

  const create = async () => {
    setBusy(true);
    setError(null);
    try {
      await commands.createDockerDatabase(name.trim(), version, file?.token ?? null);
      void queries.invalidateQueries({ queryKey: ["docker"] });
      props.onClose();
    } catch (e) {
      setError(describe(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="sheet-layer">
      <button type="button" className="sheet-scrim" aria-label="Cancel" tabIndex={-1} onClick={props.onClose} />
      <form
        className="sheet"
        role="dialog"
        aria-modal="true"
        aria-labelledby="new-database-title"
        onSubmit={(e) => {
          e.preventDefault();
          void create();
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape") props.onClose();
        }}
      >
        <h2 id="new-database-title">New database</h2>
        <p className="muted">A Postgres container on this computer, with its own data volume, reachable from this computer only.</p>
        <label className="sheet-label" htmlFor="new-database-name">
          Name
        </label>
        <input id="new-database-name" ref={first} className="input" spellCheck={false} value={name} onChange={(e) => setName(e.target.value)} />
        <label className="sheet-label" htmlFor="new-database-version">
          Version
        </label>
        <select id="new-database-version" className="input" value={version} onChange={(e) => setVersion(Number(e.target.value))}>
          {VERSIONS.map((v) => (
            <option key={v} value={v}>
              PostgreSQL {v}
            </option>
          ))}
        </select>
        <span className="sheet-label">Load</span>
        <div className="new-database-file">
          <span className={file ? "mono" : "muted"}>{file ? file.name : "Nothing: start empty"}</span>
          {file ? (
            <button type="button" className="btn btn-quiet" onClick={() => setFile(null)}>
              Remove
            </button>
          ) : (
            <button type="button" className="btn" onClick={() => void choose()}>
              Choose a dump or SQL file…
            </button>
          )}
        </div>
        {error && (
          <p className="status-error" role="alert">
            {error}
          </p>
        )}
        <div className="sheet-actions">
          <button type="button" className="btn" onClick={props.onClose}>
            Cancel
          </button>
          <button type="submit" className="btn btn-primary" disabled={busy || name.trim() === ""}>
            Create
          </button>
        </div>
      </form>
    </div>
  );
}
