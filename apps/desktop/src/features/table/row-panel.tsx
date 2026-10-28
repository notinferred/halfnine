import { ArrowRight, X } from "lucide-react";
import type { ForeignKey, IncomingKey, PageColumn, Structure } from "../../ipc/bindings.ts";

function pretty(value: string, typeName: string): string {
  if (typeName === "json" || typeName === "jsonb") {
    try {
      return JSON.stringify(JSON.parse(value), null, 2);
    } catch {
      return value;
    }
  }
  return value;
}

function bytes(value: string): string {
  // Postgres prints bytea as \x followed by two hex digits per byte.
  const size = Math.max(0, (value.length - 2) / 2);
  const preview = value.slice(2, 2 + 64).replace(/(..)/g, "$1 ");
  return `${size.toLocaleString()} bytes\n${preview}${value.length > 66 ? "…" : ""}`;
}

/** One row shown down the page, with the foreign keys that lead out of it and into it. */
export function RowPanel(props: {
  columns: PageColumn[];
  row: (string | null)[];
  cut: Set<number>;
  structure: Structure | undefined;
  onFollow: (key: ForeignKey) => void;
  onReferences: (key: IncomingKey) => void;
  onClose: () => void;
}) {
  const outgoing = props.structure?.foreignKeys ?? [];
  const incoming = props.structure?.referencedBy ?? [];

  return (
    <aside className="row-panel" aria-label="Row">
      <header className="row-panel-header">
        <h2>Row</h2>
        <button type="button" className="btn btn-quiet btn-icon" aria-label="Close the row panel" onClick={props.onClose}>
          <X size={14} strokeWidth={1.5} aria-hidden="true" />
        </button>
      </header>
      <div className="row-panel-body">
        <dl className="row-fields">
          {props.columns.map((column, index) => {
            const value = props.row[index] ?? null;
            return (
              <div key={column.name} className="row-field">
                <dt>
                  <span>{column.name}</span>
                  <span className="mono muted">{column.typeName}</span>
                </dt>
                <dd className={value === null ? "grid-null" : column.typeName === "bytea" ? "mono" : undefined}>
                  {value === null ? "NULL" : column.typeName === "bytea" ? bytes(value) : pretty(value, column.typeName)}
                  {props.cut.has(index) && <span className="row-cut muted"> Cut at 64 KB.</span>}
                </dd>
              </div>
            );
          })}
        </dl>
        {outgoing.length > 0 && (
          <section className="row-links">
            <h3>Points to</h3>
            {outgoing.map((key) => (
              <button key={key.name} type="button" className="row-link" onClick={() => props.onFollow(key)}>
                <span>
                  {key.target.name}
                  <span className="muted"> by {key.columns.join(", ")}</span>
                </span>
                <ArrowRight size={13} strokeWidth={1.5} aria-hidden="true" />
              </button>
            ))}
          </section>
        )}
        {incoming.length > 0 && (
          <section className="row-links">
            <h3>Referenced by</h3>
            {incoming.map((key) => (
              <button key={key.name} type="button" className="row-link" onClick={() => props.onReferences(key)}>
                <span>
                  {key.source.schema === "public" ? key.source.name : `${key.source.schema}.${key.source.name}`}
                  <span className="muted"> by {key.sourceColumns.join(", ")}</span>
                </span>
                <ArrowRight size={13} strokeWidth={1.5} aria-hidden="true" />
              </button>
            ))}
          </section>
        )}
      </div>
    </aside>
  );
}
