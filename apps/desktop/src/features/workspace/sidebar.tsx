import type { UseQueryResult } from "@tanstack/react-query";
import { ChevronRight, Eye, Layers, Lock, Sheet, Table2 } from "lucide-react";
import { useState } from "react";
import type { RelationKind, SchemaNode, SessionInfo } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { useWorkspace } from "../../state.ts";

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
  onDisconnect: () => void;
  onSwitchDatabase: (database: string) => Promise<void>;
}) {
  const { connection, server } = props.session;
  const [filter, setFilter] = useState("");
  const [switchError, setSwitchError] = useState<string | null>(null);
  const nodes = props.schema.data ?? [];
  const query = filter.trim().toLowerCase();

  return (
    <nav className="sidebar" aria-label="Schema">
      <div className="traffic-lights-space" data-tauri-drag-region />
      <button type="button" className="sidebar-connection" onClick={props.onDisconnect} title="Switch connection">
        <span className="dot" data-label={connection.label} aria-hidden="true" />
        <span className="sidebar-connection-text">
          <span className="sidebar-connection-name">{connection.name}</span>
          <span className="sidebar-connection-detail">
            {server.user} · PostgreSQL {server.version.split(" ")[0]}
          </span>
        </span>
        {connection.readOnly && <Lock size={13} strokeWidth={1.5} aria-label="Read-only" className="muted" />}
      </button>
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
          <SchemaBranch key={node.name} node={node} query={query} startOpen={node.name === "public" || nodes.length === 1} />
        ))}
      </div>
    </nav>
  );
}

function SchemaBranch(props: { node: SchemaNode; query: string; startOpen: boolean }) {
  const openTable = useWorkspace((s) => s.openTable);
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
      <button type="button" className="branch-toggle" aria-expanded={expanded} onClick={() => setOpen(!open)}>
        <ChevronRight size={12} strokeWidth={2} aria-hidden="true" />
        {node.name}
      </button>
      {expanded && (
        <>
          {relations.map((r) => {
            const Icon = KIND_ICON[r.kind];
            const current = active?.schema === node.name && active.name === r.name;
            return (
              <button
                key={r.name}
                type="button"
                className="leaf"
                aria-current={current || undefined}
                onClick={() => openTable({ schema: node.name, name: r.name })}
                title={r.estimatedRows === null ? KIND_NAME[r.kind] : `${KIND_NAME[r.kind]} · ≈ ${r.estimatedRows.toLocaleString()} rows`}
              >
                <Icon size={14} strokeWidth={1.5} aria-label={KIND_NAME[r.kind]} />
                <span>{r.name}</span>
              </button>
            );
          })}
          <Group title="Functions" items={functions.map((f) => ({ name: f }))} searching={query.length > 0} />
          <Group title="Sequences" items={sequences.map((s) => ({ name: s }))} searching={query.length > 0} />
          <Group title="Enums" items={enums.map((e) => ({ name: e.name, detail: e.values.join(", ") }))} searching={query.length > 0} />
        </>
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
