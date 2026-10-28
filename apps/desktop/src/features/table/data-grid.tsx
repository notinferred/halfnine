import { type ColumnRegular, RevoGrid } from "@revolist/react-datagrid";
import { useMemo, useRef } from "react";
import type { ForeignKey, PageColumn, Sort } from "../../ipc/bindings.ts";

const ROW_HEIGHT = 26;
/** A cell shows at most this much text; the row panel shows the rest. */
const SHOWN = 1_000;

type Row = Record<string, string | number | null>;

function width(typeName: string): number {
  if (/^(smallint|integer|bigint|real|double|numeric|money|oid)/.test(typeName)) return 110;
  if (typeName === "boolean") return 80;
  if (typeName === "date") return 110;
  if (typeName.startsWith("timestamp")) return 200;
  if (typeName === "uuid") return 290;
  return 220;
}

/** The grid: rows exactly as Postgres printed them, read-only for now, sorted by the server, loading more as it scrolls. */
export function DataGrid(props: {
  columns: PageColumn[];
  rows: (string | null)[][];
  hidden?: Set<string>;
  /** Given with `onSort` for a table, which the server sorts. Without them the grid sorts its own rows, as for a result. */
  sort?: Sort | null;
  keys?: Map<string, ForeignKey>;
  onSort?: (column: string) => void;
  onFollow?: (key: ForeignKey, row: number) => void;
  onFocusRow?: (row: number) => void;
  onNearEnd?: () => void;
}) {
  const grid = useRef<HTMLRevoGridElement>(null);
  // Templates are created once per column set, so they call the latest handlers through this ref.
  const handlers = useRef(props);
  handlers.current = props;

  const source = useMemo<Row[]>(
    () =>
      props.rows.map((row, index) => {
        const record: Row = { __row: index };
        for (let c = 0; c < row.length; c++) record[`c${c}`] = row[c] ?? null;
        return record;
      }),
    [props.rows],
  );

  const columns = useMemo<ColumnRegular[]>(
    () =>
      props.columns.flatMap((column, index) => {
        if (props.hidden?.has(column.name)) return [];
        const key = props.keys?.get(column.name);
        const regular: ColumnRegular = {
          prop: `c${index}`,
          name: column.name,
          size: width(column.typeName),
          minSize: 60,
          sortable: true,
          order: props.sort?.column === column.name ? (props.sort.descending ? "desc" : "asc") : undefined,
          columnTemplate: (h) => [h("span", { class: "grid-name" }, column.name), h("span", { class: "grid-type" }, column.typeName)],
          cellTemplate: (h, cell) => {
            const value = cell.model[cell.prop] as string | null;
            if (value === null) return h("span", { class: "grid-null" }, "NULL");
            const text = value.length > SHOWN ? `${value.slice(0, SHOWN)}…` : value;
            if (!key) return text;
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
    [props.columns, props.hidden, props.sort, props.keys],
  );

  return (
    <RevoGrid
      ref={grid}
      className="grid"
      columns={columns}
      source={source}
      readonly
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
      onAfterfocus={(event) => {
        const row = event.detail.model?.__row;
        if (typeof row === "number") handlers.current.onFocusRow?.(row);
      }}
      onViewportscroll={(event) => {
        if (event.detail.dimension !== "rgRow") return;
        const visible = grid.current?.clientHeight ?? 0;
        if (event.detail.coordinate + visible > (source.length - 40) * ROW_HEIGHT) handlers.current.onNearEnd?.();
      }}
    />
  );
}
