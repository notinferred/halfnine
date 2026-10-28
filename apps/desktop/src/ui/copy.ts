import type { TableRef } from "../ipc/bindings.ts";

type Rows = (string | null)[][];

const quoteCsv = (value: string | null) => (value === null ? "" : /[",\n\r]/.test(value) ? `"${value.replaceAll('"', '""')}"` : value);

/** RFC 4180 CSV. NULL becomes an empty field, as CSV has no other way to write it. */
export const toCsv = (columns: string[], rows: Rows) => [columns, ...rows].map((r) => r.map(quoteCsv).join(",")).join("\n");

/** Each row as an object keyed by column, with values as Postgres printed them, or null. */
export const toJson = (columns: string[], rows: Rows) =>
  JSON.stringify(
    rows.map((r) => Object.fromEntries(columns.map((c, i) => [c, r[i] ?? null]))),
    null,
    2,
  );

const ident = (name: string) => `"${name.replaceAll('"', '""')}"`;
const literal = (value: string | null) => (value === null ? "NULL" : `'${value.replaceAll("'", "''")}'`);

/** One INSERT per row. Postgres reads each quoted value with the column's own type, so every type round-trips. */
export const toInsert = (table: TableRef, columns: string[], rows: Rows) =>
  rows
    .map((r) => `INSERT INTO ${ident(table.schema)}.${ident(table.name)} (${columns.map(ident).join(", ")}) VALUES (${r.map(literal).join(", ")});`)
    .join("\n");

export const copy = (text: string) => navigator.clipboard.writeText(text);
