import { type ColumnRegular, RevoGrid } from "@revolist/react-datagrid";
import { useEffect, useMemo, useRef } from "react";
import type { ForeignKey, PageColumn, Sort } from "../../ipc/bindings.ts";
import { isMac } from "../../ui/shortcuts.ts";
import type { At, Cell } from "./edits.ts";

const ROW_HEIGHT = 26;
/** A cell shows at most this much text; the row panel shows the rest. */
const SHOWN = 1_000;

type Row = Record<string, string | number | boolean | null | undefined>;

export type CellEdit = { at: At; column: number; value: Cell | undefined };

/** What the grid needs to edit a table. Without it the grid only reads. */
export type GridEditing = {
  /** Columns nothing can be written to, such as generated ones. */
  locked: Set<string>;
  /** Loaded rows' staged cells, by row index, as column indexes. */
  staged: Map<number, Set<number>>;
  deleted: Set<number>;
  /** Cells too long to have come whole, which can't be edited here. */
  cut: Map<number, Set<number>>;
  /** New rows, shown above the others. `undefined` is a cell left to its default. */
  inserts: { id: string; values: (Cell | undefined)[] }[];
  onEdit: (edits: CellEdit[]) => void;
  /** Asked before the grid's own editor opens: `true` when the app opened its own editor for that cell instead. */
  onOpen: (at: At, column: number) => boolean;
  onDeleteRows: (rows: At[]) => void;
};

function width(typeName: string): number {
  if (/^(smallint|integer|bigint|real|double|numeric|money|oid)/.test(typeName)) return 110;
  if (typeName === "boolean") return 80;
  if (typeName === "date") return 110;
  if (typeName.startsWith("timestamp")) return 200;
  if (typeName === "uuid") return 290;
  return 220;
}

const atOf = (model: Row): At => (typeof model.__new === "string" ? { insert: model.__new } : { row: Number(model.__row) });
const columnOf = (prop: unknown) => Number(String(prop).slice(1));
const isTyping = (target: EventTarget | null) => target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement;

/** The cells selected now, or the focused one, as rows and column props. */
async function selected(grid: HTMLRevoGridElement): Promise<{ models: Row[]; props: string[] }> {
  const range = await grid.getSelectedRange();
  const focused = await grid.getFocused();
  const area = range ?? (focused ? { x: focused.cell.x, y: focused.cell.y, x1: focused.cell.x, y1: focused.cell.y } : null);
  if (!area) return { models: [], props: [] };
  const rows = (await grid.getVisibleSource(range?.rowType ?? focused?.rowType ?? "rgRow")) as Row[];
  // The columns in the order they are shown, which dragging a header can change.
  const store = await grid.getColumnStore("rgCol");
  const source = store.get("source");
  const columns = store.get("items").map((i) => source[i]);
  return {
    models: rows.slice(area.y, area.y1 + 1),
    props: columns.slice(area.x, area.x1 + 1).map((c) => String(c?.prop)),
  };
}

/**
 * The grid: rows exactly as Postgres printed them, sorted by the server, loading more as it scrolls. Given `editing`,
 * cells edit in place and every change is handed back to be staged, never written by the grid itself.
 */
export function DataGrid(props: {
  columns: PageColumn[];
  rows: (string | null)[][];
  hidden?: Set<string>;
  /** Given with `onSort` for a table, which the server sorts. Without them the grid sorts its own rows, as for a result. */
  sort?: Sort | null;
  keys?: Map<string, ForeignKey>;
  editing?: GridEditing;
  onSort?: (column: string) => void;
  onFollow?: (key: ForeignKey, row: number) => void;
  onFocus?: (at: At) => void;
  onNearEnd?: () => void;
}) {
  const grid = useRef<HTMLRevoGridElement>(null);
  // Templates are created once per column set, so they call the latest handlers through this ref.
  const handlers = useRef(props);
  handlers.current = props;
  const editing = props.editing;

  const source = useMemo<Row[]>(
    () =>
      props.rows.map((row, index) => {
        const record: Row = { __row: index, __class: editing?.deleted.has(index) ? "row-deleted" : "" };
        for (let c = 0; c < row.length; c++) record[`c${c}`] = row[c] ?? null;
        for (const c of editing?.staged.get(index) ?? []) record[`s${c}`] = true;
        for (const c of editing?.cut.get(index) ?? []) record[`x${c}`] = true;
        return record;
      }),
    [props.rows, editing?.deleted, editing?.staged, editing?.cut],
  );

  const pinned = useMemo<Row[]>(
    () =>
      (editing?.inserts ?? []).map((row) => {
        const record: Row = { __new: row.id, __class: "row-new" };
        row.values.forEach((value, c) => {
          record[`c${c}`] = value;
          if (value !== undefined) record[`s${c}`] = true;
        });
        return record;
      }),
    [editing?.inserts],
  );

  const columns = useMemo<ColumnRegular[]>(
    () =>
      props.columns.flatMap((column, index) => {
        if (props.hidden?.has(column.name)) return [];
        const key = props.keys?.get(column.name);
        const locked = handlers.current.editing?.locked.has(column.name) ?? true;
        const regular: ColumnRegular = {
          prop: `c${index}`,
          name: column.name,
          size: width(column.typeName),
          minSize: 60,
          sortable: true,
          order: props.sort?.column === column.name ? (props.sort.descending ? "desc" : "asc") : undefined,
          readonly: ({ model }) => locked || model[`x${index}`] === true || model.__class === "row-deleted",
          cellProperties: ({ model }) => (model[`s${index}`] ? { class: "cell-staged" } : undefined),
          columnTemplate: (h) => [h("span", { class: "grid-name" }, column.name), h("span", { class: "grid-type" }, column.typeName)],
          cellTemplate: (h, cell) => {
            const value = cell.model[cell.prop] as string | null | undefined;
            if (value === undefined) return h("span", { class: "grid-null" }, "DEFAULT");
            if (value === null) return h("span", { class: "grid-null" }, "NULL");
            const text = value.length > SHOWN ? `${value.slice(0, SHOWN)}…` : value;
            if (!key || cell.model.__new !== undefined) return text;
            return h("span", { class: "grid-key" }, [
              h("span", { class: "grid-key-value" }, text),
              h(
                "button",
                {
                  class: "grid-key-link",
                  title: `Open the ${key.target.name} row`,
                  "aria-label": `Open the ${key.target.name} row`,
                  onClick: (event: MouseEvent) => {
                    event.stopPropagation();
                    handlers.current.onFollow?.(key, cell.model.__row as number);
                  },
                },
                "→",
              ),
            ]);
          },
        };
        return [regular];
      }),
    // The locked set changes only with the table, as the columns do.
    [props.columns, props.hidden, props.sort, props.keys],
  );

  // Delete clears the selected cells: NULL in a loaded row, the default in a new one. ⌘⌫ deletes the selected rows.
  const canEdit = editing !== undefined;
  useEffect(() => {
    const element = grid.current;
    if (!element || !canEdit) return;
    const onKey = async (event: Event) => {
      const original = (event as CustomEvent<{ original: KeyboardEvent }>).detail.original;
      if (isTyping(original.target) || (original.key !== "Backspace" && original.key !== "Delete")) return;
      const withMod = isMac ? original.metaKey : original.ctrlKey;
      if (!withMod && (original.metaKey || original.ctrlKey || original.altKey)) return;
      event.preventDefault();
      original.preventDefault();
      const { models, props: chosen } = await selected(element);
      const current = handlers.current.editing;
      if (!current) return;
      if (withMod) {
        current.onDeleteRows(models.map(atOf));
        return;
      }
      const edits: CellEdit[] = [];
      for (const model of models) {
        if (model.__class === "row-deleted") continue;
        for (const prop of chosen) {
          const column = columnOf(prop);
          const name = handlers.current.columns[column]?.name;
          if (name === undefined || current.locked.has(name) || model[`x${column}`]) continue;
          edits.push({ at: atOf(model), column, value: model.__new === undefined ? null : undefined });
        }
      }
      if (edits.length > 0) current.onEdit(edits);
    };
    const listener = (event: Event) => void onKey(event);
    element.addEventListener("beforekeydown", listener);
    return () => element.removeEventListener("beforekeydown", listener);
  }, [canEdit]);

  // An edit that leaves an empty cell empty changes nothing, so NULL is never turned into empty text by accident.
  const edited = (model: Row, prop: unknown, value: unknown): CellEdit | null => {
    const column = columnOf(prop);
    const name = handlers.current.columns[column]?.name;
    const locked = handlers.current.editing?.locked.has(name ?? "") ?? true;
    if (name === undefined || locked || model[`x${column}`] || model.__class === "row-deleted") return null;
    const text = value === undefined || value === null ? "" : String(value);
    const current = model[`c${column}`];
    if (text === "" && (current === null || current === undefined)) return null;
    return { at: atOf(model), column, value: text };
  };

  return (
    <RevoGrid
      ref={grid}
      className="grid"
      columns={columns}
      source={source}
      pinnedTopSource={pinned}
      rowClass="__class"
      readonly={!canEdit}
      range
      resize
      canMoveColumns
      useClipboard
      rowHeaders
      rowSize={ROW_HEIGHT}
      hideAttribution
      onBeforesorting={(event) => {
        const onSort = handlers.current.onSort;
        if (!onSort) return;
        event.preventDefault();
        const name = event.detail.column.name;
        if (typeof name === "string") onSort(name);
      }}
      onBeforeeditstart={(event) => {
        const current = handlers.current.editing;
        if (current?.onOpen(atOf(event.detail.model as Row), columnOf(event.detail.prop))) event.preventDefault();
      }}
      onBeforeedit={(event) => {
        event.preventDefault();
        const edit = edited(event.detail.model as Row, event.detail.prop, event.detail.val);
        if (edit) handlers.current.editing?.onEdit([edit]);
      }}
      onBeforerangeedit={(event) => {
        event.preventDefault();
        const edits: CellEdit[] = [];
        for (const [index, values] of Object.entries(event.detail.data)) {
          const model = event.detail.models[Number(index)] as Row | undefined;
          if (!model) continue;
          for (const [prop, value] of Object.entries(values)) {
            const edit = edited(model, prop, value);
            if (edit) edits.push(edit);
          }
        }
        if (edits.length > 0) handlers.current.editing?.onEdit(edits);
      }}
      onAfterfocus={(event) => {
        const model = event.detail.model as Row | undefined;
        if (model && (typeof model.__row === "number" || typeof model.__new === "string")) handlers.current.onFocus?.(atOf(model));
      }}
      onViewportscroll={(event) => {
        if (event.detail.dimension !== "rgRow") return;
        const visible = grid.current?.clientHeight ?? 0;
        if (event.detail.coordinate + visible > (source.length - 40) * ROW_HEIGHT) handlers.current.onNearEnd?.();
      }}
    />
  );
}
