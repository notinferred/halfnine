import type { UseQueryResult } from "@tanstack/react-query";
import { KeyRound } from "lucide-react";
import type { Structure } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";

export function StructureView({ structure }: { structure: UseQueryResult<Structure> }) {
  if (structure.isPending) return <p className="structure-note muted">Loading…</p>;
  if (structure.isError) return <p className="structure-note status-error">{describe(structure.error)}</p>;
  const s = structure.data;
  const key = new Set(s.primaryKey);

  return (
    <div className="structure">
      <section>
        <h2>Columns</h2>
        <table className="structure-table">
          <thead>
            <tr>
              <th>Name</th>
              <th>Type</th>
              <th>Null</th>
              <th>Default</th>
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
              </tr>
            ))}
          </tbody>
        </table>
      </section>
      <Definitions title="Indexes" items={s.indexes.map((i) => ({ name: i.name, definition: i.definition }))} />
      <Definitions title="Constraints" items={s.constraints} />
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
    </div>
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
