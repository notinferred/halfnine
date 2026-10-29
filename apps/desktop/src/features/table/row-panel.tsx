import { ArrowRight, RotateCcw, Trash2, X } from "lucide-react";
import { useEffect, useRef } from "react";
import type { ForeignKey, IncomingKey, PageColumn, Structure } from "../../ipc/bindings.ts";
import type { Cell } from "./edits.ts";

const isJson = (typeName: string) => typeName === "json" || typeName === "jsonb";

function pretty(value: string, typeName: string): string {
  if (isJson(typeName)) {
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

function validJson(text: string): boolean {
  try {
    JSON.parse(text);
    return true;
  } catch {
    return false;
  }
}

/** What the panel needs to edit the row. Without it the panel only shows the row. */
export type PanelEditing = {
  locked: Set<string>;
  /** The row is new, so an unset cell takes its default. */
  isNew: boolean;
  deleted: boolean;
  /** A column to put the cursor in, asked for each time `nonce` changes. */
  focus: { column: number; nonce: number } | null;
  onChange: (column: number, value: Cell | undefined) => void;
  onPick: (key: ForeignKey) => void;
  onDelete: () => void;
  onRestore: () => void;
};

/** One row down the page, with the foreign keys that lead out of it and into it. Each field edits its value whole. */
export function RowPanel(props: {
  columns: PageColumn[];
  /** The row as shown, with staged values in place. `undefined` is a new row's cell left to its default. */
  row: (Cell | undefined)[];
  staged: Set<number>;
  cut: Set<number>;
  structure: Structure | undefined;
  editing: PanelEditing | null;
  onFollow: (key: ForeignKey) => void;
  onReferences: (key: IncomingKey) => void;
  onClose: () => void;
}) {
  const { editing } = props;
  const outgoing = editing?.isNew ? [] : (props.structure?.foreignKeys ?? []);
  const incoming = editing?.isNew ? [] : (props.structure?.referencedBy ?? []);
  const fields = useRef<(HTMLTextAreaElement | null)[]>([]);

  useEffect(() => {
    if (editing?.focus) fields.current[editing.focus.column]?.focus();
  }, [editing?.focus]);

  return (
    <aside className="row-panel" aria-label="Row">
      <header className="row-panel-header">
        <h2>{editing?.isNew ? "New row" : editing?.deleted ? "Row, to be deleted" : "Row"}</h2>
        <span className="row-panel-actions">
          {editing &&
            (editing.deleted ? (
              <button type="button" className="btn btn-quiet" onClick={editing.onRestore}>
                <RotateCcw size={13} strokeWidth={1.5} aria-hidden="true" />
                Keep row
              </button>
            ) : (
              <button type="button" className="btn btn-quiet btn-icon" aria-label="Delete the row" title="Delete the row" onClick={editing.onDelete}>
                <Trash2 size={14} strokeWidth={1.5} aria-hidden="true" />
              </button>
            ))}
          <button type="button" className="btn btn-quiet btn-icon" aria-label="Close the row panel" onClick={props.onClose}>
            <X size={14} strokeWidth={1.5} aria-hidden="true" />
          </button>
        </span>
      </header>
      <div className="row-panel-body">
        <dl className="row-fields">
          {props.columns.map((column, index) => {
            const value = props.row[index];
            const info = props.structure?.columns.find((c) => c.name === column.name);
            const key = props.structure?.foreignKeys.find((k) => k.columns.length === 1 && k.columns[0] === column.name);
            const writable = editing !== null && !editing.deleted && !editing.locked.has(column.name) && !props.cut.has(index);
            const staged = props.staged.has(index);
            return (
              <div key={column.name} className="row-field" data-staged={staged || undefined}>
                <dt>
                  <label htmlFor={`field-${index}`}>{column.name}</label>
                  <span className="mono muted">{column.typeName}</span>
                  {writable && (
                    <span className="field-actions">
                      {key && (
                        <button type="button" className="field-action" onClick={() => editing.onPick(key)}>
                          Choose…
                        </button>
                      )}
                      {editing.isNew && value !== undefined && (
                        <button type="button" className="field-action" onClick={() => editing.onChange(index, undefined)}>
                          Default
                        </button>
                      )}
                      {info?.nullable !== false && value !== null && (
                        <button type="button" className="field-action" onClick={() => editing.onChange(index, null)}>
                          Set NULL
                        </button>
                      )}
                    </span>
                  )}
                </dt>
                <dd>
                  {writable ? (
                    <Field
                      id={`field-${index}`}
                      ref={(element) => {
                        fields.current[index] = element;
                      }}
                      value={value}
                      text={value == null ? "" : staged ? value : pretty(value, column.typeName)}
                      json={isJson(column.typeName)}
                      onChange={(text) => editing.onChange(index, text)}
                    />
                  ) : (
                    <span className={value == null ? "grid-null" : column.typeName === "bytea" ? "mono" : undefined}>
                      {value === undefined
                        ? "DEFAULT"
                        : value === null
                          ? "NULL"
                          : column.typeName === "bytea"
                            ? bytes(value)
                            : pretty(value, column.typeName)}
                      {props.cut.has(index) && <span className="muted"> Cut at 64 KB, so it can't be edited here.</span>}
                    </span>
                  )}
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

/** A value's whole text, as many lines tall as it needs up to a limit. An empty field shows whether it is NULL. */
function Field(props: {
  id: string;
  ref: (element: HTMLTextAreaElement | null) => void;
  value: Cell | undefined;
  text: string;
  json: boolean;
  onChange: (text: string) => void;
}) {
  const lines = Math.min(16, Math.max(1, props.text.split("\n").length));
  const invalid = props.json && props.text.trim() !== "" && !validJson(props.text);
  return (
    <>
      <textarea
        id={props.id}
        ref={props.ref}
        className={`field ${props.json ? "mono" : ""}`}
        rows={lines}
        spellCheck={false}
        value={props.text}
        placeholder={props.value === undefined ? "DEFAULT" : props.value === null ? "NULL" : ""}
        onChange={(e) => props.onChange(e.target.value)}
      />
      {invalid && <span className="field-note status-error">This isn't valid JSON yet. Postgres will refuse it.</span>}
    </>
  );
}
