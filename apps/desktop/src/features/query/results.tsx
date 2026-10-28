import { Copy, Download } from "lucide-react";
import { useState } from "react";
import { commands, type ExportFormat, type RunOutcome, type SessionInfo, type StatementResult } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { copy, toCsv, toJson } from "../../ui/copy.ts";
import { Menu } from "../../ui/menu.tsx";
import { DataGrid } from "../table/data-grid.tsx";

const plural = (n: number, one: string, many: string) => `${n.toLocaleString()} ${n === 1 ? one : many}`;

function label(result: StatementResult, index: number) {
  const counted = result.columns.length > 0 ? plural(result.totalRows, "row", "rows") : plural(result.rowsAffected, "row changed", "rows changed");
  return `Result ${index + 1} · ${counted}`;
}

const preview = (sql: string) => (sql.length > 90 ? `${sql.slice(0, 90)}…` : sql).replace(/\s+/g, " ");

export function Results(props: { session: SessionInfo; statements: string[]; outcome: RunOutcome }) {
  const { results, failed } = props.outcome;
  const [active, setActive] = useState(failed && results.length === 0 ? results.length : 0);
  const [note, setNote] = useState<string | null>(null);
  const current = results[active];

  const exportResult = async (format: ExportFormat) => {
    const sql = props.statements[active];
    if (!sql) return;
    setNote("Exporting…");
    try {
      const done = await commands.export(props.session.sessionId, { kind: "statement", sql }, format, "result");
      setNote(done ? `Saved ${plural(done.rows ?? 0, "row", "rows")} to ${done.file}.` : null);
    } catch (error) {
      setNote(describe(error));
    }
  };

  const copyAs = async (format: "csv" | "json" | "tsv") => {
    if (!current) return;
    const names = current.columns.map((c) => c.name);
    const text =
      format === "csv"
        ? toCsv(names, current.rows)
        : format === "json"
          ? toJson(names, current.rows)
          : [names, ...current.rows].map((r) => r.map((v) => v ?? "").join("\t")).join("\n");
    await copy(text);
    setNote(`Copied ${plural(current.rows.length, "row", "rows")}.`);
  };

  return (
    <div className="results">
      <div className="results-bar">
        <div className="results-tabs">
          {results.map((r, i) => (
            <button key={i} type="button" aria-pressed={active === i} onClick={() => setActive(i)}>
              {label(r, i)}
            </button>
          ))}
          <button
            type="button"
            aria-pressed={active === results.length}
            onClick={() => setActive(results.length)}
            className={failed ? "status-error" : undefined}
          >
            Messages
          </button>
        </div>
        <span className="results-note muted">{note}</span>
        {current && current.columns.length > 0 && (
          <>
            <Menu
              label="Copy"
              icon={<Copy size={13} strokeWidth={1.5} aria-hidden="true" />}
              items={[
                { label: "Copy as TSV", onSelect: () => void copyAs("tsv") },
                { label: "Copy as CSV", onSelect: () => void copyAs("csv") },
                { label: "Copy as JSON", onSelect: () => void copyAs("json") },
              ]}
            />
            <Menu
              label="Export"
              icon={<Download size={13} strokeWidth={1.5} aria-hidden="true" />}
              items={[
                { label: "Export as CSV…", onSelect: () => void exportResult("csv") },
                { label: "Export as JSON…", onSelect: () => void exportResult("json") },
              ]}
            />
          </>
        )}
      </div>
      {current ? (
        current.columns.length > 0 ? (
          <>
            {current.totalRows > current.rows.length && (
              <p className="results-cap">
                Showing the first {current.rows.length.toLocaleString()} of {current.totalRows.toLocaleString()} rows. Export to get them all.
              </p>
            )}
            <div className="results-grid">
              <DataGrid key={active} columns={current.columns} rows={current.rows} />
            </div>
          </>
        ) : (
          <p className="query-message">
            {plural(current.rowsAffected, "row changed", "rows changed")} · {Math.round(current.elapsedMs)} ms
          </p>
        )
      ) : (
        <ol className="messages">
          {results.map((r, i) => (
            <li key={i}>
              <span className="mono">{preview(props.statements[i] ?? "")}</span>
              <span className="muted">
                {r.columns.length > 0 ? plural(r.totalRows, "row", "rows") : plural(r.rowsAffected, "row changed", "rows changed")} ·{" "}
                {Math.round(r.elapsedMs)} ms
              </span>
            </li>
          ))}
          {failed && (
            <li className="status-error">
              <span className="mono">{preview(props.statements[failed.statement] ?? "")}</span>
              <span>
                {failed.error.message}
                {failed.error.code && ` (${failed.error.code})`}
              </span>
              {failed.statement + 1 < props.statements.length && <span className="muted">The statements after it didn’t run.</span>}
            </li>
          )}
        </ol>
      )}
    </div>
  );
}
