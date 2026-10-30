import type { QueryClient } from "@tanstack/react-query";
import { type Backup, commands, type TableRef } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { showNote } from "../../ui/note.tsx";

type Holds = Backup["holds"];

/** Names in a sentence: "a", "a and b", "a, b and c". */
export const inWords = (names: string[]) => (names.length < 2 ? names.join("") : `${names.slice(0, -1).join(", ")} and ${names.at(-1)}`);

/** What was done to what a backup holds: "Dropped notes.", "Emptied notes.", "Dropped body from notes.", "Deleted db." */
export function doneInWords(holds: Holds): string {
  if (holds.kind === "database") return `Deleted ${holds.container.name}.`;
  const { table, into } = holds;
  if (into.kind === "recreate") return `Dropped ${table.name}.`;
  if (into.kind === "refill") return `Emptied ${table.name}.`;
  return `Dropped ${inWords(into.names)} from ${table.name}.`;
}

/** What putting a backup back did: "Put notes back as it was.", "Put body back in notes.", "Put db back." */
function restoredInWords(holds: Holds, name?: string): string {
  if (holds.kind === "database") return `Put ${name ?? holds.container.name} back.`;
  const { table, into } = holds;
  return into.kind === "columns" ? `Put ${inWords(into.names)} back in ${table.name}.` : `Put ${table.name} back as it was.`;
}

/** Everything the window loaded about a table, loaded again after it changed. */
export function refreshTable(queries: QueryClient, sessionId: string, table: TableRef) {
  void queries.invalidateQueries({ queryKey: ["schema", sessionId] });
  void queries.invalidateQueries({ queryKey: ["page", sessionId, table.schema, table.name] });
  void queries.invalidateQueries({ queryKey: ["structure", sessionId, table.schema, table.name] });
}

/**
 * Puts a backup back: a table on the session it was taken on, or a Docker database as a new container. Says how it
 * went in the note.
 */
export async function putBack(queries: QueryClient, backup: Backup, sessionId: string | null) {
  const { holds } = backup;
  try {
    if (holds.kind === "table") {
      if (sessionId === null) throw new Error(`Connect to ${holds.database} on ${holds.connectionName} to put it back.`);
      await commands.restoreBackup(sessionId, backup.id);
      refreshTable(queries, sessionId, holds.table);
      showNote({ text: restoredInWords(holds) });
    } else {
      showNote({ text: `Putting ${holds.container.name} back…` });
      const back = await commands.restoreDatabaseBackup(backup.id);
      showNote({ text: restoredInWords(holds, back.name) });
    }
  } catch (e) {
    showNote({ text: describe(e), error: true });
  } finally {
    void queries.invalidateQueries({ queryKey: ["backups"] });
    void queries.invalidateQueries({ queryKey: ["docker"] });
    void queries.invalidateQueries({ queryKey: ["connections"] });
  }
}

/** Says what was just done, with Undo, which puts the backup made before it back. */
export function noteUndo(queries: QueryClient, backup: Backup, sessionId: string | null, extra = "") {
  void queries.invalidateQueries({ queryKey: ["backups"] });
  showNote({
    text: `${doneInWords(backup.holds)}${extra}`,
    action: { label: "Undo", run: () => putBack(queries, backup, sessionId) },
  });
}
