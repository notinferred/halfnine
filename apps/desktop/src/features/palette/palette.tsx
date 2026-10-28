import { useQuery } from "@tanstack/react-query";
import { Command } from "cmdk";
import { Activity, Bookmark, ChevronRight, Eye, History, Layers, ScrollText, Search, Table2 } from "lucide-react";
import { commands, type SchemaNode, type SessionInfo } from "../../ipc/bindings.ts";
import { useWorkspace } from "../../state.ts";
import { modKey } from "../../ui/shortcuts.ts";
import "./palette.css";

const firstLine = (sql: string) => sql.trim().split("\n")[0]?.slice(0, 80) ?? "";

/** ⌘K: go to any table, view or query by typing part of its name, or run an action. */
export function Palette(props: { session: SessionInfo; schema: SchemaNode[]; onRefresh: () => void; onDisconnect: () => void }) {
  const { openTable, newQuery, openPage, setPaletteOpen } = useWorkspace();
  const saved = useQuery({ queryKey: ["saved"], queryFn: commands.listSavedQueries });
  const history = useQuery({ queryKey: ["history"], queryFn: commands.listHistory });
  const close = () => setPaletteOpen(false);
  const relations = props.schema.flatMap((node) => node.relations.map((r) => ({ schema: node.name, ...r })));
  const recent = (history.data ?? []).filter((h) => h.connectionId === props.session.connection.id).slice(0, 15);

  return (
    <div className="palette-layer">
      <button type="button" className="palette-scrim" aria-label="Close" tabIndex={-1} onClick={close} />
      <Command
        className="palette"
        label="Go to a table, query or action"
        loop
        onKeyDown={(e) => {
          if (e.key === "Escape") close();
        }}
      >
        <div className="palette-input">
          <Search size={16} strokeWidth={1.5} aria-hidden="true" />
          <Command.Input autoFocus placeholder="Go to a table, query or action" />
          <kbd>esc</kbd>
        </div>
        <Command.List className="palette-list">
          <Command.Empty className="palette-empty">Nothing matches.</Command.Empty>
          <Command.Group heading="Tables and views">
            {relations.map((r) => {
              const Icon = r.kind === "view" ? Eye : r.kind === "materializedView" ? Layers : Table2;
              return (
                <Command.Item
                  key={`${r.schema}.${r.name}`}
                  value={`${r.schema}.${r.name}`}
                  keywords={[r.name]}
                  onSelect={() => openTable({ schema: r.schema, name: r.name })}
                >
                  <Icon size={15} strokeWidth={1.5} aria-hidden="true" />
                  <span className="palette-name">{r.name}</span>
                  <span className="palette-detail">{r.schema}</span>
                </Command.Item>
              );
            })}
          </Command.Group>
          {(saved.data ?? []).length > 0 && (
            <Command.Group heading="Saved queries">
              {(saved.data ?? []).map((q) => (
                <Command.Item
                  key={q.id}
                  value={`saved ${q.id}`}
                  keywords={[q.name, q.sql.slice(0, 300)]}
                  onSelect={() => newQuery(q.sql, q.name, q.id)}
                >
                  <Bookmark size={15} strokeWidth={1.5} aria-hidden="true" />
                  <span className="palette-name">{q.name}</span>
                  <span className="palette-detail mono">{firstLine(q.sql)}</span>
                </Command.Item>
              ))}
            </Command.Group>
          )}
          {recent.length > 0 && (
            <Command.Group heading="Recent queries">
              {recent.map((h) => (
                <Command.Item key={h.id} value={`recent ${h.id}`} keywords={[h.sql.slice(0, 300)]} onSelect={() => newQuery(h.sql)}>
                  <History size={15} strokeWidth={1.5} aria-hidden="true" />
                  <span className="palette-name mono">{firstLine(h.sql)}</span>
                  <span className="palette-detail">{new Date(h.ranAtMs ?? 0).toLocaleString()}</span>
                </Command.Item>
              ))}
            </Command.Group>
          )}
          <Command.Group heading="Actions">
            <Command.Item value="New query" onSelect={() => newQuery()}>
              <ChevronRight size={15} strokeWidth={1.5} aria-hidden="true" />
              <span className="palette-name">New query</span>
              <span className="palette-keys">{modKey}T</span>
            </Command.Item>
            <Command.Item value="Queries saved history" onSelect={() => openPage("queries")}>
              <ScrollText size={15} strokeWidth={1.5} aria-hidden="true" />
              <span className="palette-name">Queries</span>
            </Command.Item>
            <Command.Item value="Server sessions activity" onSelect={() => openPage("sessions")}>
              <Activity size={15} strokeWidth={1.5} aria-hidden="true" />
              <span className="palette-name">Server sessions</span>
            </Command.Item>
            <Command.Item
              value="Refresh schema and data"
              onSelect={() => {
                props.onRefresh();
                close();
              }}
            >
              <ChevronRight size={15} strokeWidth={1.5} aria-hidden="true" />
              <span className="palette-name">Refresh</span>
              <span className="palette-keys">{modKey}R</span>
            </Command.Item>
            <Command.Item value="Switch connection" onSelect={props.onDisconnect}>
              <ChevronRight size={15} strokeWidth={1.5} aria-hidden="true" />
              <span className="palette-name">Switch connection</span>
            </Command.Item>
          </Command.Group>
        </Command.List>
        <div className="palette-hints">
          <span>↑ ↓ to move</span>
          <span>↵ to open</span>
          <span>esc to close</span>
        </div>
      </Command>
    </div>
  );
}
