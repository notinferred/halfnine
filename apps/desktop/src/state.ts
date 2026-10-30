import { create } from "zustand";
import { countEdits, fromChanges, noEdits, type RowEdits } from "./features/table/edits.ts";
import { newTableDraft, type StructureDraft, structureChanges } from "./features/table/structure-draft.ts";
import type { Filter, SessionInfo, TableRef, UndoRecord } from "./ipc/bindings.ts";

export type TableTab = {
  kind: "table";
  id: string;
  table: TableRef;
  /** Set when the tab was opened by following a foreign key, or by typing a condition. */
  filter: Filter | null;
  /** Kept here, not in the grid, so switching tabs never loses what was staged. */
  edits: RowEdits;
  /** The structure as it is being edited, or `null` while nothing has changed. */
  draft: StructureDraft | null;
  /** The agent call whose change these edits undo, if they do. */
  undoOf?: string;
};

export type NewTableTab = { kind: "newTable"; id: string; schema: string; name: string; draft: StructureDraft };

export type QueryTab = {
  kind: "query";
  id: string;
  title: string;
  /** Kept here, not in the editor, so switching tabs never loses what was typed. */
  sql: string;
  /** Set once the query is saved, so saving again updates it. */
  savedId: string | null;
};

/** The app's pages, opened from the sidebar's footer. Each is open at most once. */
export type PageTab = { kind: "queries" | "sessions" | "agents" | "settings"; id: string };

/**
 * A change an agent proposed, shown on the agent's own session until the person answers it. What it holds lives in
 * the app, which the agent layer keeps in step; the tab only names it.
 */
export type ProposalTab = { kind: "proposal"; id: string; proposalId: string; settled: boolean };

export type Tab = TableTab | NewTableTab | QueryTab | PageTab | ProposalTab;

/** Whether a tab holds changes that closing it would lose, or an agent's change still waiting for an answer. */
export const hasStaged = (tab: Tab) =>
  (tab.kind === "table" && (countEdits(tab.edits) > 0 || (tab.draft !== null && structureChanges(tab.draft).length > 0))) ||
  (tab.kind === "proposal" && !tab.settled);

type Workspace = {
  session: SessionInfo | null;
  tabs: Tab[];
  active: string | null;
  /** A tab someone tried to close, or leave with the connection, while it held staged changes. */
  held: string | null;
  paletteOpen: boolean;
  queriesOpened: number;
  enter: (session: SessionInfo) => void;
  leave: () => void;
  /** Activates the first tab with staged changes and holds it, so they aren't lost. `true` when there was one. */
  holdStaged: () => boolean;
  openTable: (table: TableRef, filter?: Filter | null) => void;
  newTable: (schema: string) => void;
  newQuery: (sql?: string, title?: string, savedId?: string | null) => void;
  openPage: (kind: PageTab["kind"]) => void;
  updateQuery: (id: string, change: Partial<Pick<QueryTab, "sql" | "title" | "savedId">>) => void;
  updateTable: (id: string, change: Partial<Pick<TableTab, "edits" | "draft">>) => void;
  updateNewTable: (id: string, change: Partial<Pick<NewTableTab, "name" | "draft">>) => void;
  /** Closes a tab, unless it holds staged changes: then it is held open until they are committed or discarded. */
  closeTab: (id: string) => void;
  activate: (id: string) => void;
  setFilter: (id: string, filter: Filter | null) => void;
  setPaletteOpen: (open: boolean) => void;
  /** Opens the reverse of an agent's change in its own tab, staged like the person's own edits. */
  openUndo: (undo: UndoRecord) => void;
  /** Opens a tab for an agent's proposal, beside the others, and shows it. */
  openProposal: (proposalId: string) => void;
  /** Marks a proposal answered, so its tab no longer holds the connection open. */
  settleProposal: (proposalId: string) => void;
  /** Closes a proposal's tab whatever it holds: the proposal ended in the app. */
  closeProposal: (proposalId: string) => void;
};

const sameTable = (a: TableRef, b: TableRef) => a.schema === b.schema && a.name === b.name;

export const useWorkspace = create<Workspace>((set, get) => {
  const add = (tab: Tab) => set((s) => ({ tabs: [...s.tabs, tab], active: tab.id, paletteOpen: false }));
  return {
    session: null,
    tabs: [],
    active: null,
    held: null,
    paletteOpen: false,
    queriesOpened: 0,
    enter: (session) => set({ session, tabs: [], active: null, held: null, paletteOpen: false, queriesOpened: 0 }),
    leave: () => set({ session: null, tabs: [], active: null, held: null, paletteOpen: false }),
    holdStaged: () => {
      const staged = get().tabs.find(hasStaged);
      if (staged) set({ active: staged.id, held: staged.id, paletteOpen: false });
      return staged !== undefined;
    },
    openTable: (table, filter = null) => {
      const existing = filter === null && get().tabs.find((t) => t.kind === "table" && t.filter === null && sameTable(t.table, table));
      if (existing) set({ active: existing.id, paletteOpen: false });
      else add({ kind: "table", id: crypto.randomUUID(), table, filter, edits: noEdits, draft: null });
    },
    newTable: (schema) => add({ kind: "newTable", id: crypto.randomUUID(), schema, name: "", draft: newTableDraft() }),
    newQuery: (sql = "", title, savedId = null) => {
      const opened = get().queriesOpened + 1;
      set({ queriesOpened: opened });
      add({ kind: "query", id: crypto.randomUUID(), title: title ?? `Query ${opened}`, sql, savedId });
    },
    openPage: (kind) => {
      const existing = get().tabs.find((t) => t.kind === kind);
      if (existing) set({ active: existing.id, paletteOpen: false });
      else add({ kind, id: crypto.randomUUID() });
    },
    updateQuery: (id, change) => set((s) => ({ tabs: s.tabs.map((t) => (t.id === id && t.kind === "query" ? { ...t, ...change } : t)) })),
    updateTable: (id, change) =>
      set((s) => {
        const tabs = s.tabs.map((t) => (t.id === id && t.kind === "table" ? { ...t, ...change } : t));
        const stillStaged = tabs.some((t) => t.id === s.held && hasStaged(t));
        return { tabs, held: stillStaged ? s.held : null };
      }),
    updateNewTable: (id, change) => set((s) => ({ tabs: s.tabs.map((t) => (t.id === id && t.kind === "newTable" ? { ...t, ...change } : t)) })),
    closeTab: (id) =>
      set((s) => {
        const closing = s.tabs.find((t) => t.id === id);
        if (closing && hasStaged(closing)) return { active: id, held: id };
        const index = s.tabs.findIndex((t) => t.id === id);
        const tabs = s.tabs.filter((t) => t.id !== id);
        const active = s.active === id ? (tabs[Math.min(index, tabs.length - 1)]?.id ?? null) : s.active;
        return { tabs, active };
      }),
    activate: (id) => set({ active: id }),
    setFilter: (id, filter) => set((s) => ({ tabs: s.tabs.map((t) => (t.id === id && t.kind === "table" ? { ...t, filter } : t)) })),
    setPaletteOpen: (paletteOpen) => set({ paletteOpen }),
    openUndo: (undo) =>
      add({
        kind: "table",
        id: crypto.randomUUID(),
        table: undo.table,
        filter: undo.filter,
        edits: fromChanges(undo.changes),
        draft: null,
        undoOf: undo.call,
      }),
    openProposal: (proposalId) => {
      if (get().tabs.some((t) => t.kind === "proposal" && t.proposalId === proposalId)) return;
      add({ kind: "proposal", id: crypto.randomUUID(), proposalId, settled: false });
    },
    settleProposal: (proposalId) =>
      set((s) => ({ tabs: s.tabs.map((t) => (t.kind === "proposal" && t.proposalId === proposalId ? { ...t, settled: true } : t)) })),
    closeProposal: (proposalId) => {
      const tab = get().tabs.find((t) => t.kind === "proposal" && t.proposalId === proposalId);
      if (!tab) return;
      set((s) => {
        const index = s.tabs.findIndex((t) => t.id === tab.id);
        const tabs = s.tabs.filter((t) => t.id !== tab.id);
        const active = s.active === tab.id ? (tabs[Math.min(index, tabs.length - 1)]?.id ?? null) : s.active;
        return { tabs, active, held: s.held === tab.id ? null : s.held };
      });
    },
  };
});
