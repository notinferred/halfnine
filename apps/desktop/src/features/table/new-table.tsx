import { useQueryClient } from "@tanstack/react-query";
import type { SessionInfo } from "../../ipc/bindings.ts";
import { type NewTableTab, useWorkspace } from "../../state.ts";
import { ChangesBar } from "./changes-bar.tsx";
import { createTable } from "./structure-draft.ts";
import { ColumnsEditor } from "./structure-view.tsx";
import "./table.css";

/** A table that doesn't exist yet: its name and columns, created in one statement on Commit. */
export function NewTable({ session, tab }: { session: SessionInfo; tab: NewTableTab }) {
  const queries = useQueryClient();
  const { updateNewTable, closeTab, openTable } = useWorkspace();
  const table = { schema: tab.schema, name: tab.name.trim() };

  return (
    <div className="table-tab">
      <div className="structure">
        <header className="new-table-header">
          <label htmlFor={`name-${tab.id}`} className="new-table-label muted">
            New table in {tab.schema}
          </label>
          <input
            id={`name-${tab.id}`}
            className="input new-table-name"
            placeholder="Name"
            spellCheck={false}
            autoComplete="off"
            value={tab.name}
            onChange={(e) => updateNewTable(tab.id, { name: e.target.value })}
          />
        </header>
        <section>
          <h2>Columns</h2>
          <ColumnsEditor draft={tab.draft} onDraft={(draft) => updateNewTable(tab.id, { draft })} creating />
        </section>
      </div>
      <ChangesBar
        session={session}
        changes={{ kind: "structure", table, changes: createTable(tab.draft) }}
        summary={table.name ? `${tab.schema}.${table.name}` : "Name the table"}
        promise="This statement creates the table. If it fails, nothing is created."
        commitLabel="Create table"
        onDiscard={() => closeTab(tab.id)}
        onCommitted={() => {
          void queries.invalidateQueries({ queryKey: ["schema", session.sessionId] });
          closeTab(tab.id);
          openTable(table);
        }}
      />
    </div>
  );
}
