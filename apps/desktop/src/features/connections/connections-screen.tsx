import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Lock, Plus, Search } from "lucide-react";
import { useState } from "react";
import { type ConnectionSpec, commands, type Typed } from "../../ipc/bindings.ts";
import { describe, needsPassword } from "../../ipc/errors.ts";
import { useWorkspace } from "../../state.ts";
import { ConnectionForm } from "./connection-form.tsx";
import { PasswordSheet } from "./password-sheet.tsx";
import "./connections.css";

export function ConnectionsScreen() {
  const queries = useQueryClient();
  const enter = useWorkspace((s) => s.enter);
  const list = useQuery({ queryKey: ["connections"], queryFn: commands.listConnections });
  const [chosen, setChosen] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [opening, setOpening] = useState<string | null>(null);
  const [openError, setOpenError] = useState<string | null>(null);
  const [asking, setAsking] = useState<ConnectionSpec | null>(null);
  const [askError, setAskError] = useState<string | null>(null);

  const connections = list.data ?? [];
  const selectedId = chosen ?? connections[0]?.id ?? "new";
  const selected = connections.find((c) => c.id === selectedId) ?? null;
  const visible = connections.filter((c) => `${c.name} ${c.host} ${c.database}`.toLowerCase().includes(search.toLowerCase()));

  // A connection that asks for its password each time, with none typed yet this launch, asks before it opens.
  const open = async (connection: ConnectionSpec, typed: Typed | null) => {
    setOpening(connection.id);
    setOpenError(null);
    try {
      enter(await commands.openSession(connection.id, null, typed));
    } catch (error) {
      if (!needsPassword(error)) throw error;
      setAskError(null);
      setAsking(connection);
    } finally {
      setOpening(null);
    }
  };

  const openFromList = (connection: ConnectionSpec) => {
    open(connection, null).catch((error) => {
      setChosen(connection.id);
      setOpenError(describe(error));
    });
  };

  const refresh = (id: string | null) => {
    setChosen(id);
    return queries.invalidateQueries({ queryKey: ["connections"] });
  };

  return (
    <div className="connections">
      <nav className="connections-list" aria-label="Saved connections">
        <div className="traffic-lights-space" data-tauri-drag-region />
        <div className="connections-search">
          <label htmlFor="connection-search" className="visually-hidden">
            Search connections
          </label>
          <Search size={14} strokeWidth={1.5} aria-hidden="true" />
          <input
            id="connection-search"
            className="input"
            placeholder="Search"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            spellCheck={false}
          />
        </div>
        <div className="connections-heading">Saved</div>
        <div className="connections-items">
          {visible.map((c) => (
            <ConnectionRow
              key={c.id}
              connection={c}
              current={c.id === selectedId}
              busy={opening === c.id}
              onSelect={() => setChosen(c.id)}
              onOpen={() => openFromList(c)}
            />
          ))}
          {list.isSuccess && connections.length === 0 && <p className="connections-empty muted">No connections yet.</p>}
          {list.isError && <p className="connections-empty status-error">{describe(list.error)}</p>}
        </div>
        <div className="connections-new">
          <button type="button" className="btn" onClick={() => setChosen("new")}>
            <Plus size={14} strokeWidth={1.5} aria-hidden="true" />
            New Connection
          </button>
        </div>
      </nav>
      <main className="connections-detail">
        <div className="traffic-lights-space" data-tauri-drag-region />
        <ConnectionForm
          key={selected?.id ?? "new"}
          saved={selected}
          openError={selected && openError ? openError : null}
          opening={opening !== null}
          onSaved={(id) => refresh(id)}
          onDeleted={() => refresh(null)}
          onOpen={open}
        />
      </main>
      {asking && (
        <PasswordSheet
          connection={asking}
          busy={opening !== null}
          error={askError}
          onConnect={(typed) => {
            setAskError(null);
            open(asking, typed).catch((error) => setAskError(describe(error)));
          }}
          onClose={() => setAsking(null)}
        />
      )}
    </div>
  );
}

function ConnectionRow(props: { connection: ConnectionSpec; current: boolean; busy: boolean; onSelect: () => void; onOpen: () => void }) {
  const spec = props.connection;
  return (
    <button
      type="button"
      className="connection-row"
      aria-current={props.current || undefined}
      onClick={props.onSelect}
      onDoubleClick={props.onOpen}
      title="Double-click to connect"
    >
      <span className="dot" data-label={spec.label} aria-hidden="true" />
      <span className="connection-row-text">
        <span className="connection-row-name">{spec.name}</span>
        <span className="connection-row-detail">
          {props.busy ? "Connecting…" : `${spec.host}${spec.port === 5432 ? "" : `:${spec.port}`} · ${spec.database}`}
        </span>
      </span>
      {spec.readOnly && <Lock size={13} strokeWidth={1.5} aria-label="Read-only" className="muted" />}
    </button>
  );
}
