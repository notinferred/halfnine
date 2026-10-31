import type { UseQueryResult } from "@tanstack/react-query";
import { Eye, EyeOff, KeyRound, Plus } from "lucide-react";
import type { SessionInfo, Structure, TableRef } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { RulesSection } from "./rules.tsx";
import { COMMON_TYPES, type ColumnDraft, changed, draftFrom, newColumnDraft, type StructureDraft } from "./structure-draft.ts";

/** Which of a table's columns are hidden from agents, and how to change that. */
export type AgentColumns = { hidden: Set<string>; onToggle: (column: string, hide: boolean) => void };

/** Names that usually hold something an AI provider shouldn't receive, offered for hiding. */
const SENSITIVE = /email|phone|token|password|secret/i;

/** Hides a column from agents, or shows it to them again. A column whose name looks sensitive is suggested. */
function AgentEye(props: { column: string; agents: AgentColumns }) {
  const hidden = props.agents.hidden.has(props.column);
  const suggested = !hidden && SENSITIVE.test(props.column);
  const Icon = hidden ? EyeOff : Eye;
  return (
    <button
      type="button"
      className="btn btn-icon agent-eye"
      data-suggested={suggested || undefined}
      aria-pressed={hidden}
      aria-label={hidden ? `Show ${props.column} to agents` : `Hide ${props.column} from agents`}
      title={hidden ? "Hidden from agents" : suggested ? "Looks sensitive: hide it from agents?" : "Hide from agents"}
      onClick={() => props.agents.onToggle(props.column, !hidden)}
    >
      <Icon size={13} strokeWidth={1.5} aria-hidden="true" />
    </button>
  );
}

/** A table's structure. Given `onDraft`, its columns and indexes edit in place and each change is staged. */
export function StructureView(props: {
  structure: UseQueryResult<Structure>;
  draft: StructureDraft | null;
  onDraft?: (draft: StructureDraft) => void;
  /** Shows the draft's staged changes but lets nothing change them, as for an agent's proposal. */
  staged?: boolean;
  /** Offers to hide each column from agents. */
  agents?: AgentColumns;
  /** Shows the table's rules, to add, accept from the data, and delete. */
  rules?: { session: SessionInfo; table: TableRef };
}) {
  const { structure } = props;
  const onDraft = props.onDraft ?? (props.staged ? () => undefined : undefined);
  if (structure.isPending) return <p className="structure-note muted">Loading…</p>;
  if (structure.isError) return <p className="structure-note status-error">{describe(structure.error)}</p>;
  const s = structure.data;
  const key = new Set(s.primaryKey);
  const draft = props.draft ?? draftFrom(s);

  return (
    <fieldset className="structure" disabled={props.staged}>
      <section>
        <h2>Columns</h2>
        {onDraft ? (
          <ColumnsEditor draft={draft} onDraft={onDraft} agents={props.agents} />
        ) : (
          <table className="structure-table">
            <thead>
              <tr>
                <th>Name</th>
                <th>Type</th>
                <th>Null</th>
                <th>Default</th>
                {props.agents && (
                  <th>
                    <span className="visually-hidden">Agents</span>
                  </th>
                )}
              </tr>
            </thead>
            <tbody>
              {s.columns.map((c) => (
                <tr key={c.name} title={c.comment ?? undefined}>
                  <td className="structure-name">
                    {key.has(c.name) && <KeyRound size={11} strokeWidth={2} aria-label="Primary key" />}
                    {c.name}
                  </td>
                  <td className="mono muted">{c.typeName}</td>
                  <td className="muted">{c.nullable ? "null" : "not null"}</td>
                  <td className="mono muted">{c.default ?? "—"}</td>
                  {props.agents && (
                    <td className="structure-row-action">
                      <AgentEye column={c.name} agents={props.agents} />
                    </td>
                  )}
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
      {onDraft ? (
        <IndexesEditor structure={s} draft={draft} onDraft={onDraft} />
      ) : (
        <Definitions title="Indexes" items={s.indexes.map((i) => ({ name: i.name, definition: i.definition }))} />
      )}
      <Definitions title="Constraints" items={s.constraints} />
      {props.rules && <RulesSection session={props.rules.session} table={props.rules.table} columns={s.columns} />}
      <Definitions
        title="Foreign keys"
        items={s.foreignKeys.map((f) => ({
          name: f.name,
          definition: `(${f.columns.join(", ")}) → ${f.target.schema}.${f.target.name} (${f.targetColumns.join(", ")})`,
        }))}
      />
      <Definitions
        title="Referenced by"
        items={s.referencedBy.map((f) => ({
          name: f.name,
          definition: `${f.source.schema}.${f.source.name} (${f.sourceColumns.join(", ")}) → (${f.columns.join(", ")})`,
        }))}
      />
    </fieldset>
  );
}

/** The columns as editable rows. A new table also chooses its primary key here. */
export function ColumnsEditor(props: { draft: StructureDraft; onDraft: (draft: StructureDraft) => void; creating?: boolean; agents?: AgentColumns }) {
  const { draft, onDraft } = props;
  const update = (id: string, change: Partial<ColumnDraft>) =>
    onDraft({ ...draft, columns: draft.columns.map((c) => (c.id === id ? { ...c, ...change } : c)) });
  const remove = (column: ColumnDraft) =>
    column.was === null ? onDraft({ ...draft, columns: draft.columns.filter((c) => c.id !== column.id) }) : update(column.id, { dropped: true });

  return (
    <>
      <table className="structure-table structure-edit">
        <thead>
          <tr>
            {props.creating && <th>Key</th>}
            <th>Name</th>
            <th>Type</th>
            <th>Null</th>
            <th>Default</th>
            {props.agents && (
              <th>
                <span className="visually-hidden">Agents</span>
              </th>
            )}
            <th>
              <span className="visually-hidden">Drop</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {draft.columns.map((c) => {
            const differs = changed(c);
            const fill = c.was?.generated ?? false;
            return (
              <tr key={c.id} data-dropped={c.dropped || undefined} data-new={(c.was === null && !props.creating) || undefined}>
                {props.creating && (
                  <td>
                    <input
                      type="checkbox"
                      aria-label={`${c.name || "Column"} is part of the primary key`}
                      checked={c.key}
                      onChange={(e) => update(c.id, { key: e.target.checked })}
                    />
                  </td>
                )}
                <td>
                  <input
                    className="cell-input"
                    data-staged={differs.name || undefined}
                    aria-label="Name"
                    placeholder="name"
                    spellCheck={false}
                    disabled={c.dropped}
                    value={c.name}
                    onChange={(e) => update(c.id, { name: e.target.value })}
                  />
                </td>
                <td>
                  <input
                    className="cell-input mono"
                    data-staged={differs.typeName || undefined}
                    aria-label={`Type of ${c.name}`}
                    list="common-types"
                    spellCheck={false}
                    disabled={c.dropped}
                    value={c.typeName}
                    onChange={(e) => update(c.id, { typeName: e.target.value })}
                  />
                </td>
                <td>
                  <input
                    type="checkbox"
                    data-staged={differs.nullable || undefined}
                    aria-label={`${c.name} may be NULL`}
                    disabled={c.dropped || c.key}
                    checked={c.nullable && !c.key}
                    onChange={(e) => update(c.id, { nullable: e.target.checked })}
                  />
                </td>
                <td>
                  <input
                    className="cell-input mono"
                    data-staged={differs.default || undefined}
                    aria-label={`Default of ${c.name}`}
                    placeholder="—"
                    spellCheck={false}
                    disabled={c.dropped}
                    readOnly={fill}
                    title={fill ? "Postgres fills this column itself" : undefined}
                    value={c.default}
                    onChange={(e) => update(c.id, { default: e.target.value })}
                  />
                </td>
                {props.agents && <td className="structure-row-action">{c.was && <AgentEye column={c.was.name} agents={props.agents} />}</td>}
                <td className="structure-row-action">
                  {c.dropped ? (
                    <button type="button" className="btn btn-quiet" onClick={() => update(c.id, { dropped: false })}>
                      Keep
                    </button>
                  ) : (
                    <button type="button" className="btn btn-quiet structure-drop" onClick={() => remove(c)}>
                      {c.was === null ? "Remove" : "Drop"}
                    </button>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      <datalist id="common-types">
        {COMMON_TYPES.map((t) => (
          <option key={t} value={t} />
        ))}
      </datalist>
      <button
        type="button"
        className="btn btn-quiet structure-add"
        onClick={() => onDraft({ ...draft, columns: [...draft.columns, newColumnDraft()] })}
      >
        <Plus size={13} strokeWidth={1.5} aria-hidden="true" />
        Add column
      </button>
    </>
  );
}

function IndexesEditor(props: { structure: Structure; draft: StructureDraft; onDraft: (draft: StructureDraft) => void }) {
  const { draft, onDraft } = props;
  const names = draft.columns.filter((c) => !c.dropped && c.name.trim()).map((c) => c.name.trim());
  const toggle = (id: string, column: string) =>
    onDraft({
      ...draft,
      indexes: draft.indexes.map((i) =>
        i.id !== id ? i : { ...i, columns: i.columns.includes(column) ? i.columns.filter((c) => c !== column) : [...i.columns, column] },
      ),
    });

  return (
    <section>
      <h2>Indexes</h2>
      {props.structure.indexes.length + draft.indexes.length > 0 && (
        <div className="structure-list">
          {props.structure.indexes.map((index) => {
            const dropped = draft.droppedIndexes.includes(index.name);
            return (
              <div key={index.name} className="structure-item" data-dropped={dropped || undefined}>
                <span className="structure-name">{index.name}</span>
                <span className="mono muted">{index.definition}</span>
                {!index.primary &&
                  (dropped ? (
                    <button
                      type="button"
                      className="btn btn-quiet"
                      onClick={() => onDraft({ ...draft, droppedIndexes: draft.droppedIndexes.filter((n) => n !== index.name) })}
                    >
                      Keep
                    </button>
                  ) : (
                    <button
                      type="button"
                      className="btn btn-quiet structure-drop"
                      onClick={() => onDraft({ ...draft, droppedIndexes: [...draft.droppedIndexes, index.name] })}
                    >
                      Drop
                    </button>
                  ))}
              </div>
            );
          })}
          {draft.indexes.map((index) => (
            <div key={index.id} className="structure-item" data-new>
              <span className="structure-name">New index</span>
              <span className="index-columns">
                {names.map((name) => (
                  <button
                    key={name}
                    type="button"
                    className="chip"
                    aria-pressed={index.columns.includes(name)}
                    onClick={() => toggle(index.id, name)}
                  >
                    {index.columns.includes(name) && <span className="chip-order">{index.columns.indexOf(name) + 1}</span>}
                    {name}
                  </button>
                ))}
                <label className="index-unique">
                  <input
                    type="checkbox"
                    checked={index.unique}
                    onChange={(e) =>
                      onDraft({ ...draft, indexes: draft.indexes.map((i) => (i.id === index.id ? { ...i, unique: e.target.checked } : i)) })
                    }
                  />
                  Unique
                </label>
              </span>
              <button
                type="button"
                className="btn btn-quiet"
                onClick={() => onDraft({ ...draft, indexes: draft.indexes.filter((i) => i.id !== index.id) })}
              >
                Remove
              </button>
            </div>
          ))}
        </div>
      )}
      <button
        type="button"
        className="btn btn-quiet structure-add"
        onClick={() => onDraft({ ...draft, indexes: [...draft.indexes, { id: crypto.randomUUID(), columns: [], unique: false }] })}
      >
        <Plus size={13} strokeWidth={1.5} aria-hidden="true" />
        Add index
      </button>
    </section>
  );
}

function Definitions(props: { title: string; items: { name: string; definition: string }[] }) {
  if (props.items.length === 0) return null;
  return (
    <section>
      <h2>{props.title}</h2>
      <div className="structure-list">
        {props.items.map((item) => (
          <div key={item.name} className="structure-item">
            <span className="structure-name">{item.name}</span>
            <span className="mono muted">{item.definition}</span>
          </div>
        ))}
      </div>
    </section>
  );
}
