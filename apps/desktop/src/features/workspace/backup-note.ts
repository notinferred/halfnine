import type { QueryClient } from "@tanstack/react-query";
import { type Backup, commands, type TableRef } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { showNote } from "../../ui/note.tsx";

/** Names in a sentence: "a", "a and b", "a, b and c". */
export const inWords = (names: string[]) => (names.length < 2 ? names.join("") : `${names.slice(0, -1).join(", ")} and ${names.at(-1)}`);

/** What was done to the table a backup holds: "Dropped notes.", "Emptied notes.", "Dropped body from notes." */
export function doneInWords(backup: Backup): string {
  const { table, into } = backup.holds;
  if (into.kind === "recreate") return `Dropped ${table.name}.`;
  if (into.kind === "refill") return `Emptied ${table.name}.`;
  return `Dropped ${inWords(into.names)} from ${table.name}.`;
}

/** What putting a backup back did: "Put notes back as it was.", "Put body back in notes." */
function restoredInWords(backup: Backup): string {
  const { table, into } = backup.holds;
  return into.kind === "columns" ? `Put ${inWords(into.names)} back in ${table.name}.` : `Put ${table.name} back as it was.`;
}

/** Everything the window loaded about a table, loaded again after it changed. */
export function refreshTable(queries: QueryClient, sessionId: string, table: TableRef) {
  void queries.invalidateQueries({ queryKey: ["schema", sessionId] });
  void queries.invalidateQueries({ queryKey: ["page", sessionId, table.schema, table.name] });
  void queries.invalidateQueries({ queryKey: ["structure", sessionId, table.schema, table.name] });
}

/** Says what was just done, with Undo, which puts the backup made before it back. */
export function noteUndo(queries: QueryClient, sessionId: string, backup: Backup) {
  const { table } = backup.holds;
  showNote({
    text: doneInWords(backup),
    action: {
      label: "Undo",
      run: async () => {
        try {
          await commands.restoreBackup(sessionId, backup.id);
          showNote({ text: restoredInWords(backup) });
        } catch (e) {
          showNote({ text: describe(e), error: true });
        } finally {
          refreshTable(queries, sessionId, table);
          void queries.invalidateQueries({ queryKey: ["backups"] });
        }
      },
    },
  });
}
