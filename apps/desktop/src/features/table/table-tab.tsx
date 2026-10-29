import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { Columns3, Download, PanelRight, Plus, RotateCw, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  commands,
  type ExportFormat,
  type Filter,
  type ForeignKey,
  type ImportPreview,
  type IncomingKey,
  type ProposalView,
  type SessionInfo,
  type Sort,
} from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { type TableTab as Tab, useWorkspace } from "../../state.ts";
import { copy, toCsv, toInsert, toJson } from "../../ui/copy.ts";
import { Menu } from "../../ui/menu.tsx";
import { useShortcuts } from "../../ui/shortcuts.ts";
import { ApprovalBar } from "../agents/approval-bar.tsx";
import { ChangesBar, StagedMark, stagedSummary } from "./changes-bar.tsx";
import { type CellEdit, DataGrid, type GridEditing } from "./data-grid.tsx";
import {
  type At,
  addRow,
  type Cell,
  countEdits,
  deleteRows,
  editCell,
  editNew,
  keyId,
  keyOf,
  noEdits,
  type RowEdits,
  restoreRow,
  toChanges,
} from "./edits.ts";
import { ImportSheet } from "./import-sheet.tsx";
import { KeyPicker } from "./key-picker.tsx";
import { RowPanel } from "./row-panel.tsx";
import { structureChanges } from "./structure-draft.ts";
import { StructureView } from "./structure-view.tsx";
import "./table.css";

const PAGE = 500;
/** Values longer than this, or on several lines, are edited in the row panel rather than in the cell. */
const LONG = 200;

const plural = (n: number, one: string) => `${n.toLocaleString()} ${n === 1 ? one : `${one}s`}`;

function describeFilter(filter: Filter): string {
  if (filter.kind === "where") return filter.sql;
  if (filter.kind === "keys") return plural(filter.keys.length, "row");
  return filter.values.map((v) => (v.value === null ? `${v.column} is null` : `${v.column} = ${v.value}`)).join(" and ");
}

/**
 * A table's rows or structure. Given `proposal`, it shows an agent's change staged the way the person's own edits are,
 * on the agent's session, changeable only by answering it.
 */
export function TableTab({ session, tab, proposal }: { session: SessionInfo; tab: Tab; proposal?: ProposalView }) {
  const id = session.sessionId;
  const queries = useQueryClient();
  const { setFilter, openTable, updateTable } = useWorkspace();
  const held = useWorkspace((s) => s.held === tab.id);
  const [view, setView] = useState<"data" | "structure">(proposal?.kind === "structure" ? "structure" : "data");
  const [sort, setSort] = useState<Sort | null>(null);
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const [choosingColumns, setChoosingColumns] = useState(false);
  const [panelOpen, setPanelOpen] = useState(false);
  const [focused, setFocused] = useState<At | null>(null);
  const [panelFocus, setPanelFocus] = useState<{ column: number; nonce: number } | null>(null);
  const [picking, setPicking] = useState<{ at: At; column: number; key: ForeignKey } | null>(null);
  const [importing, setImporting] = useState<ImportPreview | null>(null);
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

  const s = structure.data;
  const readOnly = session.connection.readOnly;
  const holdsRows = s?.kind === "table" || s?.kind === "partitionedTable" || s?.kind === "foreignTable";
  const canEditRows = !proposal && !readOnly && holdsRows && (s?.primaryKey.length ?? 0) > 0;
  const canEditStructure = !proposal && !readOnly && (s?.kind === "table" || s?.kind === "partitionedTable");

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
  const keys = useMemo(() => new Map((s?.foreignKeys ?? []).filter((k) => k.columns.length === 1).map((k) => [k.columns[0] ?? "", k])), [s]);
  const locked = useMemo(() => new Set((s?.columns ?? []).filter((c) => c.generated).map((c) => c.name)), [s]);

  const keyFor = (row: Cell[]) => keyOf(row, columns, s?.primaryKey ?? []);

  // The rows as they will be once committed: staged values in place, deleted rows marked.
  const staged = useMemo(() => {
    const cells = new Map<number, Set<number>>();
    const deleted = new Set<number>();
    if (!canEditRows && !proposal) return { shown: rows, cells, deleted };
    const key = s?.primaryKey ?? [];
    const shown = rows.map((row, r) => {
      const rowId = keyId(keyOf(row, columns, key));
      if (tab.edits.deletes[rowId]) deleted.add(r);
      const update = tab.edits.updates[rowId];
      if (!update) return row;
      const marks = new Set<number>();
      cells.set(r, marks);
      return row.map((value, c) => {
        const cell = update.cells[columns[c]?.name ?? ""];
        if (!cell) return value;
        marks.add(c);
        return cell.to;
      });
    });
    return { shown, cells, deleted };
  }, [rows, columns, s, tab.edits, canEditRows, proposal]);

  const inserts = useMemo(
    () => tab.edits.inserts.map((row) => ({ id: row.id, values: columns.map((c) => (c.name in row.values ? row.values[c.name] : undefined)) })),
    [tab.edits.inserts, columns],
  );

  /** Changes the staged edits from the latest ones, so several changes in a row never undo each other. */
  const stage = (change: (edits: RowEdits) => RowEdits) => {
    const current = useWorkspace.getState().tabs.find((t) => t.id === tab.id);
    if (current?.kind === "table") updateTable(tab.id, { edits: change(current.edits) });
  };

  const stageCells = (cells: CellEdit[]) =>
    stage((edits) =>
      cells.reduce((next, cell) => {
        const name = columns[cell.column]?.name;
        if (name === undefined) return next;
        if ("insert" in cell.at) return editNew(next, cell.at.insert, name, cell.value);
        const row = rows[cell.at.row];
        return row ? editCell(next, keyFor(row), name, row[cell.column] ?? null, cell.value ?? null) : next;
      }, edits),
    );

  const deleteAt = (ats: At[]) =>
    stage((edits) =>
      deleteRows(
        edits,
        ats.flatMap((at) => ("row" in at && rows[at.row] ? [keyFor(rows[at.row] ?? [])] : [])),
        ats.flatMap((at) => ("insert" in at ? [at.insert] : [])),
      ),
    );

  const valueAt = (at: At, column: number): Cell | undefined =>
    "insert" in at ? inserts.find((r) => r.id === at.insert)?.values[column] : staged.shown[at.row]?.[column];

  const openInPanel = (at: At, column: number) => {
    setFocused(at);
    setPanelOpen(true);
    setPanelFocus({ column, nonce: Date.now() });
  };

  // The grid reads its handlers at the moment they fire, so this object may be new on every render.
  const editing: GridEditing | undefined = canEditRows
    ? {
        locked,
        staged: staged.cells,
        deleted: staged.deleted,
        cut,
        inserts,
        onEdit: (cells) => stageCells(cells),
        onOpen: (at, column) => {
          const key = keys.get(columns[column]?.name ?? "");
          if (key) {
            setPicking({ at, column, key });
            return true;
          }
          const value = valueAt(at, column);
          const typeName = columns[column]?.typeName ?? "";
          if (["json", "jsonb", "xml"].includes(typeName) || (typeof value === "string" && (value.includes("\n") || value.length > LONG))) {
            openInPanel(at, column);
            return true;
          }
          return false;
        },
        onDeleteRows: (ats) => deleteAt(ats),
      }
    : proposal
      ? {
          locked,
          staged: staged.cells,
          deleted: staged.deleted,
          cut,
          inserts,
          readOnly: true,
          onEdit: () => undefined,
          onOpen: () => true,
          onDeleteRows: () => undefined,
        }
      : undefined;

  const rowChanges = countEdits(tab.edits);
  const structureChangeList = tab.draft ? structureChanges(tab.draft) : [];

  // A tab held open for its staged changes shows them, whichever view has them.
  useEffect(() => {
    if (!held) return;
    if (view === "data" && rowChanges === 0 && structureChangeList.length > 0) setView("structure");
    if (view === "structure" && structureChangeList.length === 0 && rowChanges > 0) setView("data");
  }, [held, view, rowChanges, structureChangeList.length]);

  const follow = (key: ForeignKey, row: number) =>
    openTable(key.target, {
      kind: "equals",
      values: key.columns.map((c, i) => ({
        column: key.targetColumns[i] ?? c,
        value: rows[row]?.[columns.findIndex((col) => col.name === c)] ?? null,
      })),
    });
  const references = (key: IncomingKey, row: number) =>
    openTable(key.source, {
      kind: "equals",
      values: key.sourceColumns.map((c, i) => ({
        column: c,
        value: rows[row]?.[columns.findIndex((col) => col.name === (key.columns[i] ?? c))] ?? null,
      })),
    });

  const applyWhere = () => {
    setCount(null);
    setFilter(tab.id, where.trim() ? { kind: "where", sql: where.trim() } : null);
  };

  const refresh = () => {
    setCount(null);
    void structure.refetch();
    void pages.refetch();
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

  const chooseImport = async () => {
    try {
      const preview = await commands.chooseImport();
      if (preview) setImporting(preview);
    } catch (error) {
      setNote(describe(error));
    }
  };

  // The panel's row: a loaded one by index, or a new one, which leaves the panel once it is removed.
  const panel = (() => {
    if (!panelOpen || view !== "data" || focused === null) return null;
    if ("insert" in focused) {
      const values = inserts.find((r) => r.id === focused.insert)?.values;
      const set = new Set(values?.flatMap((v, i) => (v === undefined ? [] : [i])));
      return values ? { at: focused, values, staged: set, cut: new Set<number>(), deleted: false } : null;
    }
    const r = focused.row;
    const values = staged.shown[r];
    if (!values) return null;
    return {
      at: focused,
      values,
      staged: staged.cells.get(r) ?? new Set<number>(),
      cut: cut.get(r) ?? new Set<number>(),
      deleted: staged.deleted.has(r),
    };
  })();

  const lastPage = pages.data?.pages.at(-1);
  const estimate = s?.estimatedRows;
  const status = pages.isError
    ? describe(pages.error)
    : pages.isPending
      ? "Loading…"
      : `${rows.length === 0 ? "No rows" : `Rows 1–${rows.length.toLocaleString()}${pages.hasNextPage ? "+" : ""}`} · ${Math.round(lastPage?.elapsedMs ?? 0)} ms`;
  const why = proposal
    ? ` · proposed by ${proposal.agent}`
    : readOnly
      ? " · read-only"
      : holdsRows && s?.primaryKey.length === 0
        ? " · read-only: no primary key"
        : "";

  return (
    <div className="table-tab">
      <div className="toolbar">
        <fieldset className="segmented" aria-label="View">
          <button type="button" aria-pressed={view === "data"} onClick={() => setView("data")}>
            Data
            {rowChanges > 0 && <StagedMark />}
          </button>
          <button type="button" aria-pressed={view === "structure"} onClick={() => setView("structure")}>
            Structure
            {structureChangeList.length > 0 && <StagedMark />}
          </button>
        </fieldset>
        {proposal ? (
          view === "data" && (
            <div className="filter-chip">
              <span>Only the rows {proposal.agent} changes</span>
            </div>
          )
        ) : tab.filter && tab.filter.kind !== "where" ? (
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
        {canEditRows && view === "data" && (
          <button type="button" className="btn btn-icon" aria-label="Add a row" title="Add a row" onClick={() => stage(addRow)}>
            <Plus size={14} strokeWidth={1.5} aria-hidden="true" />
          </button>
        )}
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
          icon={<Download size={14} strokeWidth={1.5} aria-label="Copy, export or import" />}
          disabled={columns.length === 0}
          items={[
            { label: "Copy rows as CSV", onSelect: () => void copyRows("csv") },
            { label: "Copy rows as JSON", onSelect: () => void copyRows("json") },
            { label: "Copy rows as INSERT", onSelect: () => void copyRows("insert") },
            { label: "Export table as CSV…", onSelect: () => void exportTable("csv") },
            { label: "Export table as JSON…", onSelect: () => void exportTable("json") },
            ...(!proposal && !readOnly && holdsRows ? [{ label: "Import CSV…", onSelect: () => void chooseImport() }] : []),
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
        <button type="button" className="btn btn-icon" aria-label="Refresh" onClick={refresh}>
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
                rows={staged.shown}
                hidden={hidden}
                sort={sort}
                keys={keys}
                editing={editing}
                onSort={(column) =>
                  setSort(sort?.column !== column ? { column, descending: false } : sort.descending ? null : { column, descending: true })
                }
                onFollow={follow}
                onFocus={setFocused}
                onNearEnd={() => {
                  if (pages.hasNextPage && !pages.isFetchingNextPage) void pages.fetchNextPage();
                }}
              />
            )}
          </div>
        ) : (
          <StructureView
            structure={structure}
            draft={tab.draft}
            staged={proposal !== undefined}
            onDraft={canEditStructure ? (draft) => updateTable(tab.id, { draft }) : undefined}
          />
        )}
        {panel && (
          <RowPanel
            columns={columns}
            row={panel.values}
            staged={panel.staged}
            cut={panel.cut}
            structure={s}
            editing={
              canEditRows
                ? {
                    locked,
                    isNew: "insert" in panel.at,
                    deleted: panel.deleted,
                    focus: panelFocus,
                    onChange: (column, value) => stageCells([{ at: panel.at, column, value }]),
                    onPick: (key) => setPicking({ at: panel.at, column: columns.findIndex((c) => c.name === key.columns[0]), key }),
                    onDelete: () => deleteAt([panel.at]),
                    onRestore: () => {
                      const row = "row" in panel.at ? rows[panel.at.row] : undefined;
                      if (row) stage((edits) => restoreRow(edits, keyFor(row)));
                    },
                  }
                : null
            }
            onFollow={(key) => "row" in panel.at && follow(key, panel.at.row)}
            onReferences={(key) => "row" in panel.at && references(key, panel.at.row)}
            onClose={() => setPanelOpen(false)}
          />
        )}
      </div>

      {proposal && <ApprovalBar key={proposal.id} proposal={proposal} />}
      {!proposal && view === "data" && rowChanges > 0 && (
        <ChangesBar
          session={session}
          changes={toChanges(tab.table, tab.edits)}
          summary={stagedSummary(rowChanges)}
          promise="These statements run together in one transaction. If a row has changed since you loaded it, nothing is written."
          held={held}
          onDiscard={() => updateTable(tab.id, { edits: noEdits })}
          onCommitted={(done) => {
            updateTable(tab.id, { edits: noEdits });
            setNote(`Committed ${plural(done.statements, "statement")} in ${done.elapsedMs.toLocaleString()} ms.`);
            refresh();
          }}
        />
      )}
      {!proposal && view === "structure" && tab.draft && structureChangeList.length > 0 && (
        <ChangesBar
          session={session}
          changes={{ kind: "structure", table: tab.table, changes: structureChangeList }}
          summary={stagedSummary(structureChangeList.length)}
          promise="These statements run together in one transaction. If one fails, none of them is applied."
          held={held}
          onDiscard={() => updateTable(tab.id, { draft: null })}
          onCommitted={(done) => {
            updateTable(tab.id, { draft: null });
            setNote(`Committed ${plural(done.statements, "statement")} in ${done.elapsedMs.toLocaleString()} ms.`);
            void queries.invalidateQueries({ queryKey: ["schema", id] });
            refresh();
          }}
        />
      )}

      <div className="statusbar" role="status">
        <span className={pages.isError ? "status-error" : undefined}>{note ?? status}</span>
        {tab.filter && <span className="muted">· filtered</span>}
        <span className="statusbar-end">
          {tab.table.schema}.{tab.table.name}
          {why}
        </span>
      </div>

      {picking && (
        <KeyPicker
          session={session}
          foreignKey={picking.key}
          current={valueAt(picking.at, picking.column)}
          nullable={s?.columns.find((c) => c.name === picking.key.columns[0])?.nullable ?? true}
          onPick={(value) => stageCells([{ at: picking.at, column: picking.column, value }])}
          onClose={() => setPicking(null)}
        />
      )}
      {importing && (
        <ImportSheet
          session={session}
          table={tab.table}
          columns={columns.filter((c) => !locked.has(c.name))}
          preview={importing}
          onImported={(n) => {
            setImporting(null);
            setNote(`Imported ${plural(n, "row")}.`);
            refresh();
          }}
          onClose={() => setImporting(null)}
        />
      )}
    </div>
  );
}
