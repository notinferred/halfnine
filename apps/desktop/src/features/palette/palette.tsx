import { Command } from "cmdk";
import { ChevronRight, Eye, Layers, Search, Table2 } from "lucide-react";
import type { SchemaNode, SessionInfo } from "../../ipc/bindings.ts";
import { useWorkspace } from "../../state.ts";
import { modKey } from "../../ui/shortcuts.ts";
import "./palette.css";

/** ⌘K: go to any table or view by typing part of its name, or run an action. */
export function Palette(props: { session: SessionInfo; schema: SchemaNode[]; onRefresh: () => void; onDisconnect: () => void }) {
  const { openTable, setPaletteOpen } = useWorkspace();
  const close = () => setPaletteOpen(false);
  const relations = props.schema.flatMap((node) => node.relations.map((r) => ({ schema: node.name, ...r })));

  return (
    <div className="palette-layer">
      <button type="button" className="palette-scrim" aria-label="Close" tabIndex={-1} onClick={close} />
      <Command
        className="palette"
        label="Go to a table or action"
        loop
        onKeyDown={(e) => {
          if (e.key === "Escape") close();
        }}
      >
        <div className="palette-input">
          <Search size={16} strokeWidth={1.5} aria-hidden="true" />
          <Command.Input autoFocus placeholder="Go to a table or action" />
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
          <Command.Group heading="Actions">
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
