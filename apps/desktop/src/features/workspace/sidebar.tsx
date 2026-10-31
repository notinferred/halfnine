import { type UseQueryResult, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Activity,
  Bot,
  ChevronDown,
  ChevronRight,
  Container,
  Ellipsis,
  Eye,
  Layers,
  Lock,
  Plus,
  ScrollText,
  Settings,
  Sheet,
  Table2,
} from "lucide-react";
import { useState } from "react";
import { type Changes, type CommitDone, commands, type RelationKind, type SchemaNode, type SessionInfo, type TableRef } from "../../ipc/bindings.ts";
import { brokeRule, describe, notBackedUp } from "../../ipc/errors.ts";
import { useWorkspace } from "../../state.ts";
import { Menu } from "../../ui/menu.tsx";
import { showNote } from "../../ui/note.tsx";
import { modKey } from "../../ui/shortcuts.ts";
import { noteUndo, refreshTable } from "./backup-note.ts";
import { ConfirmByName } from "./confirm-by-name.tsx";

const KIND_ICON: Record<RelationKind, typeof Table2> = {
  table: Table2,
  partitionedTable: Table2,
  foreignTable: Sheet,
  view: Eye,
  materializedView: Layers,
};

const KIND_NAME: Record<RelationKind, string> = {
  table: "Table",
  partitionedTable: "Partitioned table",
  foreignTable: "Foreign table",
  view: "View",
  materializedView: "Materialized view",
};

export function Sidebar(props: {
  session: SessionInfo;
  schema: UseQueryResult<SchemaNode[]>;
  onClose: () => void;
  onSwitchDatabase: (database: string) => Promise<void>;
}) {
  const { connection, server } = props.session;
  const { parked, show, openAnother } = useWorkspace();
  // The connections open in the window, by name, so the list stays put as you switch.
  const named = (o: SessionInfo) => `${o.connection.name} · ${o.server.database}`;
  const open = [...parked.map((p) => p.session), props.session].sort((a, b) => named(a).localeCompare(named(b)));
  const [filter, setFilter] = useState("");
  const [switchError, setSwitchError] = useState<string | null>(null);
  const nodes = props.schema.data ?? [];
  const query = filter.trim().toLowerCase();

  return (
    <nav className="sidebar" aria-label="Schema">
      <div className="traffic-lights-space" data-tauri-drag-region />
      <Menu
        label=""
        className="sidebar-connection"
        title="Open connections"
        align="start"
        icon={
          <>
            <span className="dot" data-label={connection.label} aria-hidden="true" />
            <span className="sidebar-connection-text">
              <span className="sidebar-connection-name">{connection.name}</span>
              <span className="sidebar-connection-detail">
                {server.user} · PostgreSQL {server.version.split(" ")[0]}
              </span>
            </span>
            {connection.readOnly && <Lock size={13} strokeWidth={1.5} aria-label="Read-only" className="muted" />}
            <ChevronDown size={13} strokeWidth={1.5} aria-hidden="true" className="muted" />
          </>
        }
        items={[
          ...open.map((o) => ({
            label: named(o),
            checked: o.sessionId === props.session.sessionId,
            onSelect: () => show(o.sessionId),
          })),
          { label: "Open another…", keys: `${modKey}O`, apart: true, onSelect: openAnother },
          { label: `Close ${connection.name}`, onSelect: props.onClose },
        ]}
      />
      {server.databases.length > 1 && (
        <div className="sidebar-field">
          <label htmlFor="database-switch" className="visually-hidden">
            Database
          </label>
          <select
            id="database-switch"
            className="input"
            value={server.database}
            onChange={(e) => {
              setSwitchError(null);
              props.onSwitchDatabase(e.target.value).catch((error) => setSwitchError(describe(error)));
            }}
          >
            {server.databases.map((d) => (
              <option key={d} value={d}>
                {d}
              </option>
            ))}
          </select>
          {switchError && <p className="sidebar-error status-error">{switchError}</p>}
        </div>
      )}
      <div className="sidebar-field">
        <label htmlFor="schema-filter" className="visually-hidden">
          Filter tables
        </label>
        <input
          id="schema-filter"
          className="input"
          placeholder="Filter"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          spellCheck={false}
        />
      </div>
      <div className="tree">
        {props.schema.isPending && <p className="tree-note muted">Loading…</p>}
        {props.schema.isError && <p className="tree-note status-error">{describe(props.schema.error)}</p>}
        {nodes.map((node) => (
          <SchemaBranch key={node.name} session={props.session} node={node} query={query} startOpen={node.name === "public" || nodes.length === 1} />
        ))}
      </div>
      <Pages />
    </nav>
  );
}

const PAGES = [
  { kind: "queries", name: "Queries", Icon: ScrollText },
  { kind: "sessions", name: "Sessions", Icon: Activity },
  { kind: "agents", name: "Agents", Icon: Bot },
  { kind: "docker", name: "Docker", Icon: Container },
  { kind: "settings", name: "Settings", Icon: Settings },
] as const;

/** The app's pages, always in view at the foot of the sidebar. Docker joins them later in M5. */
function Pages() {
  const openPage = useWorkspace((s) => s.openPage);
  const current = useWorkspace((s) => s.tabs.find((t) => t.id === s.active)?.kind);
  return (
    <div className="pages">
      {PAGES.map(({ kind, name, Icon }) => (
        <button key={kind} type="button" className="page-link" aria-current={current === kind || undefined} onClick={() => openPage(kind)}>
          <Icon className="page-icon" size={14} strokeWidth={1.5} aria-hidden="true" />
          {name}
        </button>
      ))}
    </div>
  );
}

type Emptying = { table: TableRef; kind: RelationKind; action: "truncate" | "drop" };
type Confirming = Emptying & { why: string };

function SchemaBranch(props: { session: SessionInfo; node: SchemaNode; query: string; startOpen: boolean }) {
  const queries = useQueryClient();
  const openTable = useWorkspace((s) => s.openTable);
  const newTable = useWorkspace((s) => s.newTable);
  const [confirming, setConfirming] = useState<Confirming | null>(null);
  const settings = useQuery({ queryKey: ["settings"], queryFn: commands.getSettings });
  const backingUp = settings.data?.backups ?? true;
  const writable = !props.session.connection.readOnly;
  const id = props.session.sessionId;
  const changesOf = ({ table, action }: Emptying): Extract<Changes, { kind: "structure" }> => ({
    kind: "structure",
    table,
    changes: [{ kind: action }],
  });

  const done = ({ table, action }: Emptying, result: CommitDone) => {
    if (action === "drop") {
      const { tabs, closeTab } = useWorkspace.getState();
      for (const t of tabs) if (t.kind === "table" && t.table.schema === table.schema && t.table.name === table.name) closeTab(t.id);
    }
    refreshTable(queries, id, table);
    if (result.backup) {
      noteUndo(queries, result.backup, id);
    }
  };

  // With backups on, a table is backed up and dropped or emptied at once, with Undo. Otherwise its name is asked for.
  // One that breaks a rule set for the table says so, and the person may go ahead anyway.
  const empty = async (target: Emptying, despiteRules = false) => {
    const backingUpThis = backingUp && target.kind === "table";
    const slow = backingUpThis ? setTimeout(() => showNote({ text: `Backing up ${target.table.name}…` }), 400) : undefined;
    try {
      done(target, await commands.commitChanges(id, changesOf(target), false, despiteRules));
    } catch (e) {
      if (notBackedUp(e)) setConfirming({ ...target, why: describe(e) });
      else if (brokeRule(e)) {
        const label = target.action === "drop" ? "Drop anyway" : "Truncate anyway";
        showNote({ text: describe(e), error: true, action: { label, run: () => empty(target, true) } });
      } else showNote({ text: describe(e), error: true });
    } finally {
      clearTimeout(slow);
    }
  };
  const active = useWorkspace((s) => {
    const tab = s.tabs.find((t) => t.id === s.active);
    return tab?.kind === "table" ? tab.table : undefined;
  });
  const [open, setOpen] = useState(props.startOpen);
  const { node, query } = props;
  const matches = (name: string) => !query || name.toLowerCase().includes(query);
  const relations = node.relations.filter((r) => matches(r.name));
  const functions = node.functions.filter(matches);
  const sequences = node.sequences.filter(matches);
  const enums = node.enums.filter((e) => matches(e.name));
  if (query && relations.length + functions.length + sequences.length + enums.length === 0) return null;
  const expanded = open || query.length > 0;

  return (
    <div className="branch">
      <div className="branch-row">
        <button type="button" className="branch-toggle" aria-expanded={expanded} onClick={() => setOpen(!open)}>
          <ChevronRight size={12} strokeWidth={2} aria-hidden="true" />
          {node.name}
        </button>
        {writable && (
          <button
            type="button"
            className="row-action"
            aria-label={`New table in ${node.name}`}
            title={`New table in ${node.name}`}
            onClick={() => newTable(node.name)}
          >
            <Plus size={13} strokeWidth={1.5} aria-hidden="true" />
          </button>
        )}
      </div>
      {expanded && (
        <>
          {relations.map((r) => {
            const Icon = KIND_ICON[r.kind];
            const current = active?.schema === node.name && active.name === r.name;
            const table = { schema: node.name, name: r.name };
            const truncatable = r.kind === "table" || r.kind === "partitionedTable";
            // Only a plain table is backed up first; anything else asks for its name.
            const asks = !backingUp || r.kind !== "table";
            return (
              <div key={r.name} className="leaf-row">
                <button
                  type="button"
                  className="leaf"
                  aria-current={current || undefined}
                  onClick={() => openTable(table)}
                  title={r.estimatedRows === null ? KIND_NAME[r.kind] : `${KIND_NAME[r.kind]} · ≈ ${r.estimatedRows.toLocaleString()} rows`}
                >
                  <Icon size={14} strokeWidth={1.5} aria-label={KIND_NAME[r.kind]} />
                  <span>{r.name}</span>
                </button>
                {writable && (
                  <Menu
                    label=""
                    icon={<Ellipsis size={14} strokeWidth={1.5} aria-label={`More for ${r.name}`} />}
                    items={[
                      ...(truncatable
                        ? [{ label: asks ? "Truncate…" : "Truncate", onSelect: () => void empty({ table, kind: r.kind, action: "truncate" }) }]
                        : []),
                      { label: asks ? "Drop…" : "Drop", onSelect: () => void empty({ table, kind: r.kind, action: "drop" }) },
                    ]}
                  />
                )}
              </div>
            );
          })}
          <Group title="Functions" items={functions.map((f) => ({ name: f }))} searching={query.length > 0} />
          <Group title="Sequences" items={sequences.map((s) => ({ name: s }))} searching={query.length > 0} />
          <Group title="Enums" items={enums.map((e) => ({ name: e.name, detail: e.values.join(", ") }))} searching={query.length > 0} />
        </>
      )}
      {confirming && (
        <ConfirmByName
          session={props.session}
          changes={changesOf(confirming)}
          kind={confirming.kind}
          why={confirming.why}
          onClose={() => setConfirming(null)}
          onDone={(result) => {
            setConfirming(null);
            done(confirming, result);
          }}
        />
      )}
    </div>
  );
}

function Group(props: { title: string; items: { name: string; detail?: string }[]; searching: boolean }) {
  const [open, setOpen] = useState(false);
  if (props.items.length === 0) return null;
  const expanded = open || props.searching;
  return (
    <div className="group-branch">
      <button type="button" className="branch-toggle branch-toggle-minor" aria-expanded={expanded} onClick={() => setOpen(!open)}>
        <ChevronRight size={12} strokeWidth={2} aria-hidden="true" />
        {props.title}
        <span className="branch-count">{props.items.length}</span>
      </button>
      {expanded &&
        props.items.map((item) => (
          <div key={item.name} className="leaf leaf-static" title={item.detail}>
            <span>{item.name}</span>
          </div>
        ))}
    </div>
  );
}
