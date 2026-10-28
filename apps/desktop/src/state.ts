import { create } from "zustand";
import type { Filter, SessionInfo, TableRef } from "./ipc/bindings.ts";

export type TableTab = {
  kind: "table";
  id: string;
  table: TableRef;
  /** Set when the tab was opened by following a foreign key, or by typing a condition. */
  filter: Filter | null;
};

export type QueryTab = {
  kind: "query";
  id: string;
  title: string;
  /** Kept here, not in the editor, so switching tabs never loses what was typed. */
  sql: string;
  /** Set once the query is saved, so saving again updates it. */
  savedId: string | null;
};

export type SessionsTab = { kind: "sessions"; id: string };

export type Tab = TableTab | QueryTab | SessionsTab;

type Workspace = {
  session: SessionInfo | null;
  tabs: Tab[];
  active: string | null;
  paletteOpen: boolean;
  queriesOpened: number;
  enter: (session: SessionInfo) => void;
  leave: () => void;
  openTable: (table: TableRef, filter?: Filter | null) => void;
  newQuery: (sql?: string, title?: string, savedId?: string | null) => void;
  openSessions: () => void;
  updateQuery: (id: string, change: Partial<Pick<QueryTab, "sql" | "title" | "savedId">>) => void;
  closeTab: (id: string) => void;
  activate: (id: string) => void;
  setFilter: (id: string, filter: Filter | null) => void;
  setPaletteOpen: (open: boolean) => void;
};

const sameTable = (a: TableRef, b: TableRef) => a.schema === b.schema && a.name === b.name;

export const useWorkspace = create<Workspace>((set, get) => {
  const add = (tab: Tab) => set((s) => ({ tabs: [...s.tabs, tab], active: tab.id, paletteOpen: false }));
  return {
    session: null,
    tabs: [],
    active: null,
    paletteOpen: false,
    queriesOpened: 0,
    enter: (session) => set({ session, tabs: [], active: null, paletteOpen: false, queriesOpened: 0 }),
    leave: () => set({ session: null, tabs: [], active: null, paletteOpen: false }),
    openTable: (table, filter = null) => {
      const existing = filter === null && get().tabs.find((t) => t.kind === "table" && t.filter === null && sameTable(t.table, table));
      if (existing) set({ active: existing.id, paletteOpen: false });
      else add({ kind: "table", id: crypto.randomUUID(), table, filter });
    },
    newQuery: (sql = "", title, savedId = null) => {
      const opened = get().queriesOpened + 1;
      set({ queriesOpened: opened });
      add({ kind: "query", id: crypto.randomUUID(), title: title ?? `Query ${opened}`, sql, savedId });
    },
    openSessions: () => {
      const existing = get().tabs.find((t) => t.kind === "sessions");
      if (existing) set({ active: existing.id, paletteOpen: false });
      else add({ kind: "sessions", id: crypto.randomUUID() });
    },
    updateQuery: (id, change) => set((s) => ({ tabs: s.tabs.map((t) => (t.id === id && t.kind === "query" ? { ...t, ...change } : t)) })),
    closeTab: (id) =>
      set((s) => {
        const index = s.tabs.findIndex((t) => t.id === id);
        const tabs = s.tabs.filter((t) => t.id !== id);
        const active = s.active === id ? (tabs[Math.min(index, tabs.length - 1)]?.id ?? null) : s.active;
        return { tabs, active };
      }),
    activate: (id) => set({ active: id }),
    setFilter: (id, filter) => set((s) => ({ tabs: s.tabs.map((t) => (t.id === id && t.kind === "table" ? { ...t, filter } : t)) })),
    setPaletteOpen: (paletteOpen) => set({ paletteOpen }),
  };
});
