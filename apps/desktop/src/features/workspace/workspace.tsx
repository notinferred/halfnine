import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Search, Table2, X } from "lucide-react";
import { commands, type SessionInfo } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { useWorkspace } from "../../state.ts";
import { modKey, useShortcuts } from "../../ui/shortcuts.ts";
import { Palette } from "../palette/palette.tsx";
import { TableTab } from "../table/table-tab.tsx";
import { Sidebar } from "./sidebar.tsx";
import "./workspace.css";

export function Workspace({ session }: { session: SessionInfo }) {
  const queries = useQueryClient();
  const { tabs, active, paletteOpen, setPaletteOpen, activate, closeTab, enter, leave } = useWorkspace();
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
  });

  const activeTab = tabs.find((t) => t.id === active);

  return (
    <div className="workspace">
      <Sidebar session={session} schema={schema} onDisconnect={disconnect} onSwitchDatabase={switchDatabase} />
      <main className="workspace-main">
        <div className="tabs" data-tauri-drag-region>
          <nav className="tabs-list" aria-label="Open tables">
            {tabs.map((tab) => (
              <div key={tab.id} className="tab" data-active={tab.id === active}>
                <button type="button" className="tab-open" aria-current={tab.id === active || undefined} onClick={() => activate(tab.id)}>
                  <Table2 size={13} strokeWidth={1.5} aria-hidden="true" />
                  <span>{tab.table.name}</span>
                  {tab.filter && <span className="tab-filtered">filtered</span>}
                </button>
                <button type="button" className="tab-close" aria-label={`Close ${tab.table.name}`} onClick={() => closeTab(tab.id)}>
                  <X size={12} strokeWidth={1.5} aria-hidden="true" />
                </button>
              </div>
            ))}
          </nav>
          <button type="button" className="goto" onClick={() => setPaletteOpen(true)}>
            <Search size={12} strokeWidth={1.5} aria-hidden="true" />
            Go to…
            <kbd>{modKey}K</kbd>
          </button>
        </div>
        {activeTab ? (
          <TableTab key={activeTab.id} session={session} tab={activeTab} />
        ) : (
          <div className="workspace-empty">
            {schema.isError ? (
              <p className="status-error">{describe(schema.error)}</p>
            ) : (
              <p className="muted">Choose a table in the sidebar, or press {modKey}K to go to one.</p>
            )}
          </div>
        )}
      </main>
      {paletteOpen && <Palette session={session} schema={schema.data ?? []} onRefresh={refresh} onDisconnect={disconnect} />}
    </div>
  );
}
