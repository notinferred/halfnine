import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Activity, Bot, FileCode, Plus, ScrollText, Search, Settings, Sparkles, Table2, TableProperties, X } from "lucide-react";
import { commands, type ProposalView, type SessionInfo } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { hasStaged, type Tab, useWorkspace } from "../../state.ts";
import { Glyph } from "../../ui/glyph.tsx";
import { Keys } from "../../ui/keys.tsx";
import { modCap, modKey, useShortcuts } from "../../ui/shortcuts.ts";
import { AgentsPage } from "../agents/agents-page.tsx";
import { ProposalTab, proposalTitle } from "../agents/proposal-tab.tsx";
import { Palette } from "../palette/palette.tsx";
import { QueriesPage } from "../queries/queries-page.tsx";
import { QueryTab } from "../query/query-tab.tsx";
import { SessionsTab } from "../sessions/sessions-tab.tsx";
import { SettingsPage } from "../settings/settings-page.tsx";
import { StagedMark } from "../table/changes-bar.tsx";
import { NewTable } from "../table/new-table.tsx";
import { TableTab } from "../table/table-tab.tsx";
import { Sidebar } from "./sidebar.tsx";
import "./workspace.css";

const PAGE_TITLE = { queries: "Queries", sessions: "Sessions", agents: "Agents", settings: "Settings" };

function title(tab: Tab, proposals: ProposalView[], connectionId: string) {
  if (tab.kind === "table") return tab.table.name;
  if (tab.kind === "query") return tab.title;
  if (tab.kind === "newTable") return tab.name.trim() || "New table";
  if (tab.kind === "proposal")
    return proposalTitle(
      proposals.find((p) => p.id === tab.proposalId),
      connectionId,
    );
  return PAGE_TITLE[tab.kind];
}

const TAB_ICON = {
  table: Table2,
  newTable: TableProperties,
  query: FileCode,
  queries: ScrollText,
  sessions: Activity,
  agents: Bot,
  settings: Settings,
  proposal: Sparkles,
};

export function Workspace({ session }: { session: SessionInfo }) {
  const queries = useQueryClient();
  const { tabs, active, paletteOpen, setPaletteOpen, activate, closeTab, newQuery, enter, leave, holdStaged } = useWorkspace();
  const schema = useQuery({ queryKey: ["schema", session.sessionId], queryFn: () => commands.schema(session.sessionId) });
  const proposals = useQuery({ queryKey: ["agents"], queryFn: commands.agentState }).data?.proposals ?? [];

  // Every query for this session has the session id second in its key, so one predicate refreshes them all.
  const ofSession = { predicate: (q: { queryKey: readonly unknown[] }) => q.queryKey[1] === session.sessionId };
  const refresh = () => void queries.invalidateQueries(ofSession);

  // Leaving the connection would lose staged changes, so a tab holding some is shown instead.
  const disconnect = async () => {
    if (holdStaged()) return;
    await commands.closeSession(session.sessionId).catch(() => undefined);
    queries.removeQueries(ofSession);
    leave();
  };

  const switchDatabase = async (database: string) => {
    if (holdStaged()) throw new Error("Commit or discard the staged changes before switching databases.");
    const next = await commands.switchDatabase(session.sessionId, database);
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
              const name = title(tab, proposals, session.connection.id);
              const agent = tab.kind === "proposal" ? proposals.find((p) => p.id === tab.proposalId)?.agent : undefined;
              return (
                <div key={tab.id} className="tab" data-active={tab.id === active}>
                  <button type="button" className="tab-open" aria-current={tab.id === active || undefined} onClick={() => activate(tab.id)}>
                    <Icon size={13} strokeWidth={1.5} aria-hidden="true" />
                    <span>{name}</span>
                    {tab.kind === "table" && tab.filter && <span className="tab-filtered">filtered</span>}
                    {agent && <span className="tab-filtered">{agent}</span>}
                    {hasStaged(tab) && <StagedMark />}
                  </button>
                  <button type="button" className="tab-close" aria-label={`Close ${name}`} onClick={() => closeTab(tab.id)}>
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
        {activeTab?.kind === "newTable" && <NewTable key={activeTab.id} session={session} tab={activeTab} />}
        {activeTab?.kind === "query" && <QueryTab key={activeTab.id} session={session} tab={activeTab} />}
        {activeTab?.kind === "queries" && <QueriesPage key={activeTab.id} session={session} />}
        {activeTab?.kind === "sessions" && <SessionsTab key={activeTab.id} session={session} />}
        {activeTab?.kind === "agents" && <AgentsPage key={activeTab.id} />}
        {activeTab?.kind === "settings" && <SettingsPage key={activeTab.id} />}
        {activeTab?.kind === "proposal" && <ProposalTab key={activeTab.id} tab={activeTab} />}
        {!activeTab && (
          <div className="workspace-empty">
            {schema.isError ? (
              <p className="status-error">{describe(schema.error)}</p>
            ) : (
              <div className="workspace-start">
                <Glyph size={44} />
                <div className="workspace-actions">
                  <button type="button" className="btn btn-primary" onClick={() => newQuery()}>
                    New query <Keys keys={[modCap, "T"]} />
                  </button>
                  <button type="button" className="btn" onClick={() => setPaletteOpen(true)}>
                    Go to a table <Keys keys={[modCap, "K"]} />
                  </button>
                </div>
              </div>
            )}
          </div>
        )}
      </main>
      {paletteOpen && <Palette session={session} schema={schema.data ?? []} onRefresh={refresh} onDisconnect={disconnect} />}
    </div>
  );
}
