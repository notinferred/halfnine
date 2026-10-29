import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { type Client, type ClientSetup, commands } from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { copy } from "../../ui/copy.ts";

/** Each AI client Halfnine can add itself to, and whether it already has. */
export function Connect() {
  const setups = useQuery({ queryKey: ["clientSetups"], queryFn: commands.clientSetups });
  const [planning, setPlanning] = useState<ClientSetup | null>(null);
  return (
    <>
      <p className="queries-empty muted">
        Add Halfnine to the AI tools you use. Each one then reaches your connections at the level you set for them, and asks you here before anything
        it changes, unless you allow it.
      </p>
      {setups.isError && <p className="queries-empty status-error">{describe(setups.error)}</p>}
      <ul className="queries-list">
        {(setups.data ?? []).map((setup) => (
          <li key={setup.client} className="queries-item agents-client">
            <span className="queries-name">{setup.name}</span>
            {setup.connected && <span className="status-ok agents-connected">Connected</span>}
            <button type="button" className="btn" onClick={() => setPlanning(setup)}>
              {setup.connected ? "Connect again" : "Connect"}
            </button>
          </li>
        ))}
      </ul>
      {planning && <ConnectSheet setup={planning} onClose={() => setPlanning(null)} />}
    </>
  );
}

/** What connecting a client will do, shown before anything runs, and the same as a command to paste instead. */
function ConnectSheet(props: { setup: ClientSetup; onClose: () => void }) {
  const queries = useQueryClient();
  const client: Client = props.setup.client;
  const plan = useQuery({ queryKey: ["clientPlan", client], queryFn: () => commands.planClient(client), gcTime: 0 });
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const connect = async () => {
    setBusy(true);
    setError(null);
    try {
      setDone(await commands.connectClient(client));
      void queries.invalidateQueries({ queryKey: ["clientSetups"] });
    } catch (e) {
      setError(describe(e));
    } finally {
      setBusy(false);
    }
  };

  const p = plan.data;
  return (
    <div className="sheet-layer">
      <button type="button" className="sheet-scrim" aria-label="Close" tabIndex={-1} onClick={props.onClose} />
      <div
        className="sheet"
        role="dialog"
        aria-modal="true"
        aria-labelledby="connect-title"
        onKeyDown={(e) => {
          if (e.key === "Escape") props.onClose();
        }}
      >
        <h2 id="connect-title">Connect {props.setup.name}</h2>
        {plan.isPending && <p className="muted">Checking…</p>}
        {plan.isError && <p className="status-error">{describe(plan.error)}</p>}
        {p && !done && (
          <>
            {p.blocked ? (
              <p className="muted">{p.blocked}</p>
            ) : (
              <ol className="agents-steps">
                {p.steps.map((step) => (
                  <li key={step}>{step}</li>
                ))}
              </ol>
            )}
            <pre className="review-sql mono agents-manual">{p.manual}</pre>
          </>
        )}
        {done && <p className="status-ok">{done}</p>}
        {error && (
          <p className="status-error" role="alert">
            {error}
          </p>
        )}
        <div className="sheet-actions">
          {p && !done && (
            <button
              type="button"
              className="btn"
              onClick={() => {
                void copy(p.manual);
                setCopied(true);
              }}
            >
              {copied ? "Copied" : p.blocked ? "Copy" : "Copy instead"}
            </button>
          )}
          <button type="button" className="btn" onClick={props.onClose}>
            {done ? "Done" : "Cancel"}
          </button>
          {p && !p.blocked && !done && (
            <button type="button" className="btn btn-primary" disabled={busy} onClick={() => void connect()}>
              {busy ? "Connecting…" : "Connect"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
