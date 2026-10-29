import { useState } from "react";
import { commands, type ImportPreview, type PageColumn, type SessionInfo, type TableRef } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";

/** Matches the file's columns to the table's by name, ignoring case, so a file exported from this table maps itself. */
const matchByName = (preview: ImportPreview, columns: PageColumn[]) =>
  preview.columns.map((name) => columns.find((c) => c.name.toLowerCase() === name.trim().toLowerCase())?.name ?? null);

/** Imports a CSV file: each of its columns goes into a column of the table, or is skipped. All rows go in, or none do. */
export function ImportSheet(props: {
  session: SessionInfo;
  table: TableRef;
  /** The columns that can be written to. */
  columns: PageColumn[];
  preview: ImportPreview;
  onImported: (rows: number) => void;
  onClose: () => void;
}) {
  const [targets, setTargets] = useState(() => matchByName(props.preview, props.columns));
  const [importing, setImporting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const id = props.session.sessionId;

  const run = async () => {
    setImporting(true);
    setError(null);
    try {
      const rows = await commands.importCsv(id, props.table, props.preview.token, targets);
      props.onImported(rows ?? 0);
    } catch (e) {
      setError(describe(e));
    } finally {
      setImporting(false);
    }
  };

  return (
    <div className="sheet-layer">
      <button type="button" className="sheet-scrim" aria-label="Close" tabIndex={-1} onClick={importing ? undefined : props.onClose} />
      <div className="sheet sheet-wide" role="dialog" aria-modal="true" aria-labelledby="import-title">
        <h2 id="import-title">
          Import {props.preview.file} into {props.table.name}
        </h2>
        <p className="muted">Choose where each column of the file goes. Empty fields become NULL. Every row is added, or none is.</p>
        <div className="import-map">
          <table className="structure-table">
            <thead>
              <tr>
                <th>In the file</th>
                <th>Goes into</th>
                <th>First values</th>
              </tr>
            </thead>
            <tbody>
              {props.preview.columns.map((name, i) => (
                <tr key={`${i}-${name}`}>
                  <td>{name || <span className="muted">Column {i + 1}</span>}</td>
                  <td>
                    <select
                      className="input"
                      aria-label={`Where ${name || `column ${i + 1}`} goes`}
                      value={targets[i] ?? ""}
                      onChange={(e) => setTargets(targets.map((t, j) => (j === i ? e.target.value || null : t)))}
                    >
                      <option value="">Skip</option>
                      {props.columns.map((c) => (
                        <option key={c.name} value={c.name}>
                          {c.name}
                        </option>
                      ))}
                    </select>
                  </td>
                  <td className="mono muted import-sample">{props.preview.rows.map((r) => r[i] ?? "").join(" · ")}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {error && (
          <p className="status-error" role="alert">
            {error}
          </p>
        )}
        <div className="sheet-actions">
          {importing ? (
            <button type="button" className="btn" onClick={() => void commands.cancelRun(id)}>
              Stop
            </button>
          ) : (
            <button type="button" className="btn" onClick={props.onClose}>
              Cancel
            </button>
          )}
          <button type="button" className="btn btn-primary" disabled={importing || targets.every((t) => t === null)} onClick={() => void run()}>
            {importing ? "Importing…" : "Import"}
          </button>
        </div>
      </div>
    </div>
  );
}
