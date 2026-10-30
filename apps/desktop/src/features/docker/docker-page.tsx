import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Plus } from "lucide-react";
import { useState } from "react";
import { commands, type DockerDatabase, type Job } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { useWorkspace } from "../../state.ts";
import { BackupsView } from "./backups-view.tsx";
import { madeBy } from "./docker-detail.tsx";
import { NewDatabaseSheet } from "./new-database.tsx";
import "../queries/queries.css";
import "./docker.css";

/**
 * The Docker page: the Postgres containers on this computer, making new ones, and starting, stopping and deleting
 * them; and the backups made before something was dropped, emptied or deleted.
 */
export function DockerPage() {
  const [showing, setShowing] = useState<"databases" | "backups">("databases");
  const docker = useQuery({ queryKey: ["docker"], queryFn: commands.dockerState });
  const [creating, setCreating] = useState(false);
  return (
    <div className="queries-page">
      <header className="queries-header">
        <h1>Docker</h1>
        <fieldset className="segmented" aria-label="Show">
          <button type="button" aria-pressed={showing === "databases"} onClick={() => setShowing("databases")}>
            Databases
          </button>
          <button type="button" aria-pressed={showing === "backups"} onClick={() => setShowing("backups")}>
            Backups
          </button>
        </fieldset>
        {showing === "databases" && docker.data?.on && (
          <button type="button" className="btn btn-primary queries-new" disabled={!docker.data.engine} onClick={() => setCreating(true)}>
            <Plus size={13} strokeWidth={2} aria-hidden="true" />
            New database
          </button>
        )}
      </header>
      {showing === "databases" ? <Databases onCreate={() => setCreating(true)} /> : <BackupsView />}
      {creating && <NewDatabaseSheet taken={(docker.data?.databases ?? []).map((d) => d.connection.name)} onClose={() => setCreating(false)} />}
    </div>
  );
}

function Databases(props: { onCreate: () => void }) {
  const queries = useQueryClient();
  const docker = useQuery({ queryKey: ["docker"], queryFn: commands.dockerState });
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const state = docker.data;

  const act = async (action: () => Promise<unknown>) => {
    setError(null);
    try {
      await action();
    } catch (e) {
      setError(describe(e));
    } finally {
      void queries.invalidateQueries({ queryKey: ["docker"] });
      void queries.invalidateQueries({ queryKey: ["connections"] });
    }
  };

  const remove = (database: DockerDatabase) =>
    act(async () => {
      const kept = await commands.deleteDockerDatabase(database.connection.name);
      // The window was on that database, which is gone now.
      const { session, leave } = useWorkspace.getState();
      if (session?.connection.id === database.connection.id) leave();
      setNote(
        kept
          ? `Deleted ${database.connection.name}. Its data volume wasn't made by Halfnine, so it was kept.`
          : `Deleted ${database.connection.name}.`,
      );
    });

  const jobs = state?.jobs ?? [];
  const jobOf = (name: string) => jobs.find((j) => j.name === name);
  // A database still being made has a job but isn't listed by Docker yet.
  const making = jobs.filter((j) => !state?.databases.some((d) => d.connection.name === j.name));

  return (
    <>
      {state && !state.on && (
        <div className="queries-empty docker-off">
          <p className="muted">Docker databases are off. Turned on, the Postgres containers on this computer show up as connections.</p>
          <button type="button" className="btn btn-primary" onClick={() => void act(() => commands.setDockerEnabled(true))}>
            Turn on Docker databases
          </button>
        </div>
      )}
      {state?.on && state.problem && <p className="queries-empty muted">{state.problem}</p>}
      {(error ?? docker.error) && <p className="queries-empty status-error">{error ?? describe(docker.error)}</p>}
      {note && <p className="queries-empty muted">{note}</p>}
      {state?.on && (
        <ul className="queries-list">
          {making.map((job) => (
            <JobRow key={job.name} job={job} onDismiss={() => void act(() => commands.dismissDockerJob(job.name))} />
          ))}
          {state.databases.map((d) => (
            <DatabaseRow
              key={d.connection.id}
              database={d}
              job={jobOf(d.connection.name)}
              onStart={() => void act(() => commands.startDockerDatabase(d.connection.name))}
              onStop={() => void act(() => commands.stopDockerDatabase(d.connection.name))}
              onDelete={() => void remove(d)}
              onCopy={(name) => void act(() => commands.copyDockerDatabase(d.connection.name, name))}
              onDismiss={() => void act(() => commands.dismissDockerJob(d.connection.name))}
            />
          ))}
          {state.engine && state.databases.length === 0 && making.length === 0 && (
            <li className="queries-empty docker-off">
              <p className="muted">No Postgres containers on this computer yet.</p>
              <button type="button" className="btn btn-primary" onClick={props.onCreate}>
                New database
              </button>
            </li>
          )}
        </ul>
      )}
    </>
  );
}

function JobRow(props: { job: Job; onDismiss: () => void }) {
  const { job } = props;
  return (
    <li className="queries-item docker-row">
      <span className="docker-row-text">
        <span className="queries-name">{job.name}</span>
        <span className={job.failed ? "status-error docker-detail" : "muted docker-detail"}>{job.failed ?? job.doing}</span>
      </span>
      {job.failed && (
        <button type="button" className="btn btn-quiet" onClick={props.onDismiss}>
          Close
        </button>
      )}
    </li>
  );
}

function DatabaseRow(props: {
  database: DockerDatabase;
  job: Job | undefined;
  onStart: () => void;
  onStop: () => void;
  onDelete: () => void;
  onCopy: (name: string) => void;
  onDismiss: () => void;
}) {
  const { database, job } = props;
  const name = database.connection.name;
  const [confirming, setConfirming] = useState(false);
  const [typed, setTyped] = useState("");
  const [copying, setCopying] = useState<string | null>(null);
  const made = madeBy(database);
  const detail = [
    database.running ? "Running" : "Stopped",
    database.major === null ? null : `PostgreSQL ${database.major}`,
    database.port === null ? "no published port" : `:${database.port}`,
    made,
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <li className="queries-item docker-row">
      <span className="docker-row-text">
        <span className="queries-name">{name}</span>
        <span className={job?.failed ? "status-error docker-detail" : "muted docker-detail"}>{job ? (job.failed ?? job.doing) : detail}</span>
      </span>
      {job?.failed ? (
        <button type="button" className="btn btn-quiet" onClick={props.onDismiss}>
          Close
        </button>
      ) : job ? null : copying !== null ? (
        <form
          className="docker-confirm"
          onSubmit={(e) => {
            e.preventDefault();
            props.onCopy(copying.trim());
            setCopying(null);
          }}
        >
          <label htmlFor={`copy-${name}`} className="visually-hidden">
            Name the copy of {name}
          </label>
          <input id={`copy-${name}`} className="input" spellCheck={false} value={copying} onChange={(e) => setCopying(e.target.value)} />
          <button type="button" className="btn btn-quiet" onClick={() => setCopying(null)}>
            Cancel
          </button>
          <button type="submit" className="btn btn-primary" disabled={copying.trim() === ""}>
            Copy
          </button>
        </form>
      ) : confirming ? (
        <form
          className="docker-confirm"
          onSubmit={(e) => {
            e.preventDefault();
            if (typed === name) props.onDelete();
          }}
        >
          <label htmlFor={`delete-${name}`} className="visually-hidden">
            Type {name} to delete it
          </label>
          <input
            id={`delete-${name}`}
            className="input"
            placeholder={`Type ${name} to delete it`}
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
          />
          <button type="button" className="btn btn-quiet" onClick={() => setConfirming(false)}>
            Keep
          </button>
          <button type="submit" className="btn btn-danger" disabled={typed !== name}>
            Delete
          </button>
        </form>
      ) : (
        <span className="docker-actions">
          {database.running ? (
            <button type="button" className="btn btn-quiet" onClick={props.onStop}>
              Stop
            </button>
          ) : (
            <button type="button" className="btn btn-quiet" onClick={props.onStart}>
              Start
            </button>
          )}
          <button type="button" className="btn btn-quiet" title="Copy it before risky work" onClick={() => setCopying(`${name}-copy`)}>
            Copy
          </button>
          <button type="button" className="btn btn-quiet" onClick={() => setConfirming(true)}>
            Delete
          </button>
        </span>
      )}
    </li>
  );
}
