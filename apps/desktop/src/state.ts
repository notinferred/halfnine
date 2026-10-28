import { create } from "zustand";
import type { Filter, SessionInfo, TableRef } from "./ipc/bindings.ts";

export type TableTab = {
  id: string;
  table: TableRef;
  /** Set when the tab was opened by following a foreign key, or by typing a condition. */
  filter: Filter | null;
};

type Workspace = {
  session: SessionInfo | null;
  tabs: TableTab[];
  active: string | null;
  paletteOpen: boolean;
  enter: (session: SessionInfo) => void;
  leave: () => void;
  openTable: (table: TableRef, filter?: Filter | null) => void;
  closeTab: (id: string) => void;
  activate: (id: string) => void;
  setFilter: (id: string, filter: Filter | null) => void;
  setPaletteOpen: (open: boolean) => void;
};

const sameTable = (a: TableRef, b: TableRef) => a.schema === b.schema && a.name === b.name;

export const useWorkspace = create<Workspace>((set, get) => ({
  session: null,
  tabs: [],
  active: null,
  paletteOpen: false,
  enter: (session) => set({ session, tabs: [], active: null, paletteOpen: false }),
  leave: () => set({ session: null, tabs: [], active: null, paletteOpen: false }),
  openTable: (table, filter = null) => {
    const existing = filter === null && get().tabs.find((t) => t.filter === null && sameTable(t.table, table));
    if (existing) {
      set({ active: existing.id, paletteOpen: false });
      return;
    }
    const tab = { id: crypto.randomUUID(), table, filter };
    set((s) => ({ tabs: [...s.tabs, tab], active: tab.id, paletteOpen: false }));
  },
  closeTab: (id) =>
    set((s) => {
      const index = s.tabs.findIndex((t) => t.id === id);
      const tabs = s.tabs.filter((t) => t.id !== id);
      const active = s.active === id ? (tabs[Math.min(index, tabs.length - 1)]?.id ?? null) : s.active;
      return { tabs, active };
    }),
  activate: (id) => set({ active: id }),
  setFilter: (id, filter) => set((s) => ({ tabs: s.tabs.map((t) => (t.id === id ? { ...t, filter } : t)) })),
  setPaletteOpen: (paletteOpen) => set({ paletteOpen }),
}));
