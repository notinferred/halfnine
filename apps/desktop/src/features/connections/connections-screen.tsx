import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Bot, Container, Lock, Plus, Search, Settings } from "lucide-react";
import { useState } from "react";
import { type ConnectionSpec, commands, type DockerDatabase, type Typed } from "../../ipc/bindings.ts";
import { describe, needsPassword } from "../../ipc/errors.ts";
import { useWorkspace } from "../../state.ts";
import { AgentsPage } from "../agents/agents-page.tsx";
import { DockerDetail, madeBy } from "../docker/docker-detail.tsx";
import { DockerPage } from "../docker/docker-page.tsx";
import { SettingsPage } from "../settings/settings-page.tsx";
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

  const docker = useQuery({ queryKey: ["docker"], queryFn: commands.dockerState });
  // Docker databases come from the engine, listed in their own section below the saved ones.
  const connections = (list.data ?? []).filter((c) => (c.source?.kind ?? "saved") === "saved");
  const selectedId = chosen ?? connections[0]?.id ?? "new";
  const selected = connections.find((c) => c.id === selectedId) ?? null;
  const matches = (text: string) => text.toLowerCase().includes(search.toLowerCase());
  const visible = connections.filter((c) => matches(`${c.name} ${c.host} ${c.database}`));
  const dockerVisible = (docker.data?.databases ?? []).filter((d) => matches(`${d.connection.name} ${d.image}`));
  const dockerSelected = docker.data?.databases.find((d) => d.connection.id === chosen) ?? null;

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
          {docker.data?.on && (
            <>
              <div className="connections-heading connections-heading-docker">Docker</div>
              {dockerVisible.map((d) => (
                <DockerRow
                  key={d.connection.id}
                  database={d}
                  current={d.connection.id === chosen}
                  busy={opening === d.connection.id}
                  onSelect={() => setChosen(d.connection.id)}
                  onOpen={() => openFromList(d.connection)}
                />
              ))}
              {docker.data.problem ? (
                <p className="connections-empty muted">{docker.data.problem}</p>
              ) : (
                docker.data.databases.length === 0 && <p className="connections-empty muted">No Postgres containers.</p>
              )}
            </>
          )}
        </div>
        <div className="connections-new">
          <button type="button" className="btn" onClick={() => setChosen("new")}>
            <Plus size={14} strokeWidth={1.5} aria-hidden="true" />
            New Connection
          </button>
        </div>
        <div className="pages">
          <button type="button" className="page-link" aria-current={chosen === "agents" || undefined} onClick={() => setChosen("agents")}>
            <Bot className="page-icon" size={14} strokeWidth={1.5} aria-hidden="true" />
            Agents
          </button>
          <button type="button" className="page-link" aria-current={chosen === "docker" || undefined} onClick={() => setChosen("docker")}>
            <Container className="page-icon" size={14} strokeWidth={1.5} aria-hidden="true" />
            Docker
          </button>
          <button type="button" className="page-link" aria-current={chosen === "settings" || undefined} onClick={() => setChosen("settings")}>
            <Settings className="page-icon" size={14} strokeWidth={1.5} aria-hidden="true" />
            Settings
          </button>
        </div>
      </nav>
      <main className="connections-detail">
        <div className="traffic-lights-space" data-tauri-drag-region />
        {dockerSelected ? (
          <DockerDetail
            key={dockerSelected.connection.id}
            database={dockerSelected}
            opening={opening === dockerSelected.connection.id}
            openError={openError}
            onOpen={() => openFromList(dockerSelected.connection)}
          />
        ) : chosen === "docker" ? (
          <DockerPage />
        ) : chosen === "settings" ? (
          <SettingsPage />
        ) : chosen === "agents" ? (
          <AgentsPage />
        ) : (
          <ConnectionForm
            key={selected?.id ?? "new"}
            saved={selected}
            openError={selected && openError ? openError : null}
            opening={opening !== null}
            onSaved={(id) => refresh(id)}
            onDeleted={() => refresh(null)}
            onOpen={open}
          />
        )}
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

/** A Docker database in the list: its state in words, never a coloured dot, since dots are labels. */
function DockerRow(props: { database: DockerDatabase; current: boolean; busy: boolean; onSelect: () => void; onOpen: () => void }) {
  const { database } = props;
  const spec = database.connection;
  const made = madeBy(database);
  const detail = [
    database.running ? "Running" : "Stopped",
    database.major === null ? null : String(database.major),
    database.port === null ? null : `:${database.port}`,
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <button
      type="button"
      className="connection-row"
      aria-current={props.current || undefined}
      onClick={props.onSelect}
      onDoubleClick={props.onOpen}
      title={database.running ? "Double-click to connect" : "Double-click to start and connect"}
    >
      <span className="dot" data-label={spec.label} aria-hidden="true" />
      <span className="connection-row-text">
        <span className="connection-row-name">{spec.name}</span>
        <span className="connection-row-detail">{props.busy ? "Connecting…" : detail}</span>
      </span>
      {made && made !== "made by Halfnine" && <span className="chip connection-row-chip">{made.replace("made by ", "")}</span>}
      {spec.readOnly && <Lock size={13} strokeWidth={1.5} aria-label="Read-only" className="muted" />}
    </button>
  );
}
