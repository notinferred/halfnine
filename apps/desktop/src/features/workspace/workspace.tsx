import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Activity, FileCode, Plus, Search, Table2, X } from "lucide-react";
import { commands, type SessionInfo } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { type Tab, useWorkspace } from "../../state.ts";
import { modKey, useShortcuts } from "../../ui/shortcuts.ts";
import { Palette } from "../palette/palette.tsx";
import { QueryTab } from "../query/query-tab.tsx";
import { SessionsTab } from "../sessions/sessions-tab.tsx";
import { TableTab } from "../table/table-tab.tsx";
import { Sidebar } from "./sidebar.tsx";
import "./workspace.css";

function title(tab: Tab) {
  if (tab.kind === "table") return tab.table.name;
  if (tab.kind === "query") return tab.title;
  return "Sessions";
}

const TAB_ICON = { table: Table2, query: FileCode, sessions: Activity };

export function Workspace({ session }: { session: SessionInfo }) {
  const queries = useQueryClient();
  const { tabs, active, paletteOpen, setPaletteOpen, activate, closeTab, newQuery, enter, leave } = useWorkspace();
  const schema = useQuery({ queryKey: ["schema", session.sessionId], queryFn: () => commands.schema(session.sessionId) });

  // Every query for this session has the session id second in its key, so one predicate refreshes them all.
  const ofSession = { predicate: (q: { queryKey: readonly unknown[] }) => q.queryKey[1] === session.sessionId };
  const refresh = () => void queries.invalidateQueries(ofSession);

  const disconnect = async () => {
    await commands.closeSession(session.sessionId).catch(() => undefined);
    queries.removeQueries(ofSession);
    leave();
  };

  const switchDatabase = async (database: string) => {
    const next = await commands.openSession(session.connection.id, database);
    await commands.closeSession(session.sessionId).catch(() => undefined);
    queries.removeQueries(ofSession);
    enter(next);
  };

  useShortcuts({
    "$mod+k": () => setPaletteOpen(!paletteOpen),
    "$mod+r": refresh,
    "$mod+w": () => active && closeTab(active),
    "$mod+t": () => newQuery(),
  });

  const activeTab = tabs.find((t) => t.id === active);

  return (
    <div className="workspace">
      <Sidebar session={session} schema={schema} onDisconnect={disconnect} onSwitchDatabase={switchDatabase} />
      <main className="workspace-main">
        <div className="tabs" data-tauri-drag-region>
          <nav className="tabs-list" aria-label="Open tabs">
            {tabs.map((tab) => {
              const Icon = TAB_ICON[tab.kind];
              return (
                <div key={tab.id} className="tab" data-active={tab.id === active}>
                  <button type="button" className="tab-open" aria-current={tab.id === active || undefined} onClick={() => activate(tab.id)}>
                    <Icon size={13} strokeWidth={1.5} aria-hidden="true" />
                    <span>{title(tab)}</span>
                    {tab.kind === "table" && tab.filter && <span className="tab-filtered">filtered</span>}
                  </button>
                  <button type="button" className="tab-close" aria-label={`Close ${title(tab)}`} onClick={() => closeTab(tab.id)}>
                    <X size={12} strokeWidth={1.5} aria-hidden="true" />
                  </button>
                </div>
              );
            })}
            <button type="button" className="tab-new" aria-label="New query" title={`New query (${modKey}T)`} onClick={() => newQuery()}>
              <Plus size={14} strokeWidth={1.5} aria-hidden="true" />
            </button>
          </nav>
          <button type="button" className="goto" onClick={() => setPaletteOpen(true)}>
            <Search size={12} strokeWidth={1.5} aria-hidden="true" />
            Go to…
            <kbd>{modKey}K</kbd>
          </button>
        </div>
        {activeTab?.kind === "table" && <TableTab key={activeTab.id} session={session} tab={activeTab} />}
        {activeTab?.kind === "query" && <QueryTab key={activeTab.id} session={session} tab={activeTab} />}
        {activeTab?.kind === "sessions" && <SessionsTab key={activeTab.id} session={session} />}
        {!activeTab && (
          <div className="workspace-empty">
            {schema.isError ? (
              <p className="status-error">{describe(schema.error)}</p>
            ) : (
              <p className="muted">
                Choose a table in the sidebar, press {modKey}K to go to one, or {modKey}T for a new query.
              </p>
            )}
          </div>
        )}
      </main>
      {paletteOpen && <Palette session={session} schema={schema.data ?? []} onRefresh={refresh} onDisconnect={disconnect} />}
    </div>
  );
}
