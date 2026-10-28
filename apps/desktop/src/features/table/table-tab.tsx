import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { Columns3, Download, PanelRight, RotateCw, X } from "lucide-react";
import { useMemo, useRef, useState } from "react";
import { commands, type ExportFormat, type Filter, type ForeignKey, type IncomingKey, type SessionInfo, type Sort } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { type TableTab as Tab, useWorkspace } from "../../state.ts";
import { copy, toCsv, toInsert, toJson } from "../../ui/copy.ts";
import { Menu } from "../../ui/menu.tsx";
import { useShortcuts } from "../../ui/shortcuts.ts";
import { DataGrid } from "./data-grid.tsx";
import { RowPanel } from "./row-panel.tsx";
import { StructureView } from "./structure-view.tsx";
import "./table.css";

const PAGE = 500;

function describeFilter(filter: Filter): string {
  if (filter.kind === "where") return filter.sql;
  return filter.values.map((v) => (v.value === null ? `${v.column} is null` : `${v.column} = ${v.value}`)).join(" and ");
}

export function TableTab({ session, tab }: { session: SessionInfo; tab: Tab }) {
  const id = session.sessionId;
  const { setFilter, openTable } = useWorkspace();
  const [view, setView] = useState<"data" | "structure">("data");
  const [sort, setSort] = useState<Sort | null>(null);
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const [choosingColumns, setChoosingColumns] = useState(false);
  const [panelOpen, setPanelOpen] = useState(false);
  const [focused, setFocused] = useState<number | null>(null);
  const [where, setWhere] = useState(tab.filter?.kind === "where" ? tab.filter.sql : "");
  const [count, setCount] = useState<{ text: string; error?: boolean } | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const filterInput = useRef<HTMLInputElement>(null);

  const structure = useQuery({
    queryKey: ["structure", id, tab.table.schema, tab.table.name],
    queryFn: () => commands.structure(id, tab.table),
  });
  const pages = useInfiniteQuery({
    queryKey: ["page", id, tab.table.schema, tab.table.name, sort, tab.filter],
    queryFn: ({ pageParam }) => commands.page(id, { table: tab.table, sort, filter: tab.filter, offset: pageParam, limit: PAGE }),
    initialPageParam: 0,
    getNextPageParam: (last, all) => (last.hasMore ? all.length * PAGE : undefined),
  });

  const columns = pages.data?.pages[0]?.columns ?? [];
  const rows = useMemo(() => pages.data?.pages.flatMap((p) => p.rows) ?? [], [pages.data]);
  const cut = useMemo(() => {
    const cells = new Map<number, Set<number>>();
    pages.data?.pages.forEach((p, i) => {
      for (const c of p.cut) {
        const row = i * PAGE + c.row;
        cells.set(row, (cells.get(row) ?? new Set()).add(c.column));
      }
    });
    return cells;
  }, [pages.data]);
  // Only single-column keys get a link in the grid; every key is in the row panel.
  const keys = useMemo(
    () => new Map((structure.data?.foreignKeys ?? []).filter((k) => k.columns.length === 1).map((k) => [k.columns[0] ?? "", k])),
    [structure.data],
  );

  const cellOf = (row: number, column: string) => rows[row]?.[columns.findIndex((c) => c.name === column)] ?? null;
  const follow = (key: ForeignKey, row: number) =>
    openTable(key.target, { kind: "equals", values: key.columns.map((c, i) => ({ column: key.targetColumns[i] ?? c, value: cellOf(row, c) })) });
  const references = (key: IncomingKey, row: number) =>
    openTable(key.source, { kind: "equals", values: key.sourceColumns.map((c, i) => ({ column: c, value: cellOf(row, key.columns[i] ?? c) })) });

  const applyWhere = () => {
    setCount(null);
    setFilter(tab.id, where.trim() ? { kind: "where", sql: where.trim() } : null);
  };

  const countRows = async () => {
    setCount({ text: "Counting…" });
    try {
      const n = await commands.countRows(id, tab.table, tab.filter);
      setCount({ text: `${(n ?? 0).toLocaleString()} rows` });
    } catch (error) {
      setCount({ text: describe(error), error: true });
    }
  };

  useShortcuts({ "$mod+f": () => filterInput.current?.focus() });

  const copyRows = async (format: "csv" | "json" | "insert") => {
    const names = columns.map((c) => c.name);
    await copy(format === "csv" ? toCsv(names, rows) : format === "json" ? toJson(names, rows) : toInsert(tab.table, names, rows));
    setNote(`Copied ${rows.length.toLocaleString()} loaded rows.`);
  };

  const exportTable = async (format: ExportFormat) => {
    setNote("Exporting…");
    try {
      const done = await commands.export(id, { kind: "table", table: tab.table, sort, filter: tab.filter }, format, tab.table.name);
      setNote(done ? `Saved ${(done.rows ?? 0).toLocaleString()} rows to ${done.file}.` : null);
    } catch (error) {
      setNote(describe(error));
    }
  };

  const lastPage = pages.data?.pages.at(-1);
  const estimate = structure.data?.estimatedRows;
  const status = pages.isError
    ? describe(pages.error)
    : pages.isPending
      ? "Loading…"
      : `${rows.length === 0 ? "No rows" : `Rows 1–${rows.length.toLocaleString()}${pages.hasNextPage ? "+" : ""}`} · ${Math.round(lastPage?.elapsedMs ?? 0)} ms`;

  return (
    <div className="table-tab">
      <div className="toolbar">
        <fieldset className="segmented" aria-label="View">
          <button type="button" aria-pressed={view === "data"} onClick={() => setView("data")}>
            Data
          </button>
          <button type="button" aria-pressed={view === "structure"} onClick={() => setView("structure")}>
            Structure
          </button>
        </fieldset>
        {tab.filter?.kind === "equals" ? (
          <div className="filter-chip">
            <span className="mono">{describeFilter(tab.filter)}</span>
            <button type="button" aria-label="Show every row" onClick={() => setFilter(tab.id, null)}>
              <X size={12} strokeWidth={1.5} aria-hidden="true" />
            </button>
          </div>
        ) : (
          <>
            <label htmlFor={`where-${tab.id}`} className="visually-hidden">
              Filter rows with a WHERE condition
            </label>
            <input
              id={`where-${tab.id}`}
              ref={filterInput}
              className="input mono filter-input"
              placeholder="Filter rows: status = 'overdue'"
              spellCheck={false}
              value={where}
              onChange={(e) => setWhere(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") applyWhere();
                if (e.key === "Escape") setWhere(tab.filter?.kind === "where" ? tab.filter.sql : "");
              }}
            />
          </>
        )}
        <button
          type="button"
          className={`btn btn-quiet count ${count?.error ? "status-error" : ""}`}
          onClick={() => void countRows()}
          title="Count every row exactly"
        >
          {count?.text ?? (estimate == null ? "Count rows" : `≈ ${Math.round(estimate).toLocaleString()} rows`)}
        </button>
        <div className="toolbar-menu">
          <button
            type="button"
            className="btn btn-icon"
            aria-label="Choose columns"
            aria-expanded={choosingColumns}
            onClick={() => setChoosingColumns(!choosingColumns)}
          >
            <Columns3 size={14} strokeWidth={1.5} aria-hidden="true" />
          </button>
          {choosingColumns && (
            <div className="column-chooser">
              {columns.map((c) => (
                <label key={c.name}>
                  <input
                    type="checkbox"
                    checked={!hidden.has(c.name)}
                    onChange={() => {
                      const next = new Set(hidden);
                      if (next.has(c.name)) next.delete(c.name);
                      else next.add(c.name);
                      setHidden(next);
                    }}
                  />
                  {c.name}
                </label>
              ))}
            </div>
          )}
        </div>
        <Menu
          label=""
          icon={<Download size={14} strokeWidth={1.5} aria-label="Copy or export" />}
          disabled={columns.length === 0}
          items={[
            { label: "Copy rows as CSV", onSelect: () => void copyRows("csv") },
            { label: "Copy rows as JSON", onSelect: () => void copyRows("json") },
            { label: "Copy rows as INSERT", onSelect: () => void copyRows("insert") },
            { label: "Export table as CSV…", onSelect: () => void exportTable("csv") },
            { label: "Export table as JSON…", onSelect: () => void exportTable("json") },
          ]}
        />
        <button
          type="button"
          className="btn btn-icon"
          aria-label="Show the row panel"
          aria-pressed={panelOpen}
          onClick={() => setPanelOpen(!panelOpen)}
        >
          <PanelRight size={14} strokeWidth={1.5} aria-hidden="true" />
        </button>
        <button
          type="button"
          className="btn btn-icon"
          aria-label="Refresh"
          onClick={() => {
            setCount(null);
            void structure.refetch();
            void pages.refetch();
          }}
        >
          <RotateCw size={14} strokeWidth={1.5} aria-hidden="true" />
        </button>
      </div>

      <div className="table-body">
        {view === "data" ? (
          <div className="grid-area">
            {pages.isError && rows.length === 0 ? (
              <p className="structure-note status-error">{describe(pages.error)}</p>
            ) : (
              <DataGrid
                columns={columns}
                rows={rows}
                hidden={hidden}
                sort={sort}
                keys={keys}
                onSort={(column) =>
                  setSort(sort?.column !== column ? { column, descending: false } : sort.descending ? null : { column, descending: true })
                }
                onFollow={follow}
                onFocusRow={setFocused}
                onNearEnd={() => {
                  if (pages.hasNextPage && !pages.isFetchingNextPage) void pages.fetchNextPage();
                }}
              />
            )}
          </div>
        ) : (
          <StructureView structure={structure} />
        )}
        {panelOpen && view === "data" && focused !== null && rows[focused] && (
          <RowPanel
            columns={columns}
            row={rows[focused]}
            cut={cut.get(focused) ?? new Set()}
            structure={structure.data}
            onFollow={(key) => follow(key, focused)}
            onReferences={(key) => references(key, focused)}
            onClose={() => setPanelOpen(false)}
          />
        )}
      </div>

      <div className="statusbar" role="status">
        <span className={pages.isError ? "status-error" : undefined}>{note ?? status}</span>
        {tab.filter && <span className="muted">· filtered</span>}
        <span className="statusbar-end">
          {tab.table.schema}.{tab.table.name}
          {session.connection.readOnly && " · read-only"}
        </span>
      </div>
    </div>
  );
}
