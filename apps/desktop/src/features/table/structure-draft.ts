import type { ColumnInfo, NewColumn, Structure, StructureChange } from "../../ipc/bindings.ts";

export type ColumnDraft = {
  id: string;
  /** The column as the table has it, or `null` for a new one. */
  was: ColumnInfo | null;
  name: string;
  typeName: string;
  nullable: boolean;
  default: string;
  /** Part of the primary key. Only a new table chooses this. */
  key: boolean;
  dropped: boolean;
};

export type IndexDraft = { id: string; columns: string[]; unique: boolean };

/** A table's structure as someone is editing it. The changes to commit are what differs from the table. */
export type StructureDraft = { columns: ColumnDraft[]; indexes: IndexDraft[]; droppedIndexes: string[] };

export const draftFrom = (structure: Structure): StructureDraft => ({
  columns: structure.columns.map((c) => ({
    id: c.name,
    was: c,
    name: c.name,
    typeName: c.typeName,
    nullable: c.nullable,
    default: c.default ?? "",
    key: structure.primaryKey.includes(c.name),
    dropped: false,
  })),
  indexes: [],
  droppedIndexes: [],
});

export const newColumnDraft = (): ColumnDraft => ({
  id: crypto.randomUUID(),
  was: null,
  name: "",
  typeName: "text",
  nullable: true,
  default: "",
  key: false,
  dropped: false,
});

/** A new table starts with the key most tables want: a number Postgres counts up by itself. */
export const newTableDraft = (): StructureDraft => ({
  columns: [{ ...newColumnDraft(), name: "id", typeName: "bigint", nullable: false, default: "generated always as identity", key: true }],
  indexes: [],
  droppedIndexes: [],
});

const newColumn = (c: ColumnDraft): NewColumn => ({
  name: c.name,
  typeName: c.typeName,
  nullable: c.nullable,
  default: c.default.trim() || null,
  key: c.key,
});

/** What changed in a column, field by field, so each changed field can be marked. */
export function changed(c: ColumnDraft) {
  const was = c.was;
  return {
    name: was !== null && c.name.trim() !== was.name,
    typeName: was !== null && c.typeName.trim() !== was.typeName,
    nullable: was !== null && c.nullable !== was.nullable,
    default: was !== null && !was.generated && c.default.trim() !== (was.default ?? ""),
  };
}

export function structureChanges(draft: StructureDraft): StructureChange[] {
  const changes: StructureChange[] = [];
  for (const c of draft.columns) {
    const was = c.was;
    if (was === null) {
      if (!c.dropped) changes.push({ kind: "addColumn", column: newColumn(c) });
      continue;
    }
    if (c.dropped) {
      changes.push({ kind: "dropColumn", column: was.name });
      continue;
    }
    const differs = changed(c);
    if (differs.typeName) changes.push({ kind: "setType", column: was.name, to: c.typeName });
    if (differs.nullable) changes.push({ kind: "setNullable", column: was.name, nullable: c.nullable });
    if (differs.default) changes.push({ kind: "setDefault", column: was.name, to: c.default.trim() || null });
    if (differs.name) changes.push({ kind: "renameColumn", column: was.name, to: c.name });
  }
  for (const name of draft.droppedIndexes) changes.push({ kind: "dropIndex", name });
  for (const index of draft.indexes) changes.push({ kind: "createIndex", columns: index.columns, unique: index.unique });
  return changes;
}

export const createTable = (draft: StructureDraft): StructureChange[] => [
  { kind: "createTable", columns: draft.columns.filter((c) => !c.dropped).map(newColumn) },
];

/** Types offered while typing one. Any type Postgres knows can be typed in full. */
export const COMMON_TYPES = [
  "text",
  "varchar(255)",
  "integer",
  "bigint",
  "numeric(12,2)",
  "boolean",
  "date",
  "timestamptz",
  "uuid",
  "jsonb",
  "bytea",
  "text[]",
];
