import type { Changes, ColumnValue, PageColumn, TableRef } from "../../ipc/bindings.ts";

export type Cell = string | null;

/** Changes staged on one table's rows, kept until they are committed or discarded. */
export type RowEdits = {
  /** Changed cells by the row's key, each with the value it had when it was loaded. */
  updates: Record<string, { key: ColumnValue[]; cells: Record<string, { from: Cell; to: Cell }> }>;
  /** Rows to delete, by the row's key. */
  deletes: Record<string, ColumnValue[]>;
  /** New rows. A column with no value set takes its default. */
  inserts: { id: string; values: Record<string, Cell> }[];
};

export const noEdits: RowEdits = { updates: {}, deletes: {}, inserts: [] };

/** A row of the grid: one that was loaded, by its index, or a new one, by its id. */
export type At = { row: number } | { insert: string };

export const keyId = (key: ColumnValue[]) => JSON.stringify(key.map((k) => k.value));

/** A loaded row's primary key, which names it in the staged edits whatever page or order it appears in. */
export const keyOf = (row: Cell[], columns: PageColumn[], primaryKey: string[]): ColumnValue[] =>
  primaryKey.map((name) => ({ column: name, value: row[columns.findIndex((c) => c.name === name)] ?? null }));

/** Stages `to` for a loaded cell whose value is `from`. Setting a cell back to `from` unstages it. */
export function editCell(edits: RowEdits, key: ColumnValue[], column: string, from: Cell, to: Cell): RowEdits {
  const id = keyId(key);
  const cells = { ...edits.updates[id]?.cells };
  if (to === from) delete cells[column];
  else cells[column] = { from, to };
  const updates = { ...edits.updates };
  if (Object.keys(cells).length === 0) delete updates[id];
  else updates[id] = { key, cells };
  return { ...edits, updates };
}

/** Sets a new row's cell. `undefined` gives the column back to its default. */
export function editNew(edits: RowEdits, insert: string, column: string, value: Cell | undefined): RowEdits {
  return {
    ...edits,
    inserts: edits.inserts.map((row) => {
      if (row.id !== insert) return row;
      const values = { ...row.values };
      if (value === undefined) delete values[column];
      else values[column] = value;
      return { ...row, values };
    }),
  };
}

export function addRow(edits: RowEdits): RowEdits {
  return { ...edits, inserts: [...edits.inserts, { id: crypto.randomUUID(), values: {} }] };
}

/** Deletes loaded rows and drops new ones. A deleted row's edited cells are dropped with it. */
export function deleteRows(edits: RowEdits, keys: ColumnValue[][], inserts: string[]): RowEdits {
  const updates = { ...edits.updates };
  const deletes = { ...edits.deletes };
  for (const key of keys) {
    delete updates[keyId(key)];
    deletes[keyId(key)] = key;
  }
  return { updates, deletes, inserts: edits.inserts.filter((row) => !inserts.includes(row.id)) };
}

export function restoreRow(edits: RowEdits, key: ColumnValue[]): RowEdits {
  const deletes = { ...edits.deletes };
  delete deletes[keyId(key)];
  return { ...edits, deletes };
}

export const countEdits = (edits: RowEdits) =>
  Object.values(edits.updates).reduce((n, row) => n + Object.keys(row.cells).length, 0) + Object.keys(edits.deletes).length + edits.inserts.length;

export const toChanges = (table: TableRef, edits: RowEdits): Changes => ({
  kind: "rows",
  table,
  deletes: Object.values(edits.deletes),
  updates: Object.values(edits.updates).map((row) => ({
    key: row.key,
    cells: Object.entries(row.cells).map(([column, cell]) => ({ column, from: cell.from, to: cell.to })),
  })),
  inserts: edits.inserts.map((row) => Object.entries(row.values).map(([column, value]) => ({ column, value }))),
});

/** Staged edits from changes made elsewhere, such as an agent's proposal: the inverse of `toChanges`. */
export function fromChanges(changes: Changes): RowEdits {
  if (changes.kind !== "rows") return noEdits;
  return {
    updates: Object.fromEntries(
      changes.updates.map((row) => [
        keyId(row.key),
        { key: row.key, cells: Object.fromEntries(row.cells.map((c) => [c.column, { from: c.from, to: c.to }])) },
      ]),
    ),
    deletes: Object.fromEntries(changes.deletes.map((key) => [keyId(key), key])),
    inserts: changes.inserts.map((values, i) => ({ id: `insert-${i}`, values: Object.fromEntries(values.map((v) => [v.column, v.value])) })),
  };
}
