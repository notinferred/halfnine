import { useEffect, useRef, useState } from "react";
import type { ConnectionSpec, Typed } from "../../ipc/bindings.ts";

/** Asks for the passwords of a connection that doesn't save them, each time it connects. */
export function PasswordSheet(props: {
  connection: ConnectionSpec;
  busy: boolean;
  error: string | null;
  onConnect: (typed: Typed) => void;
  onClose: () => void;
}) {
  const { connection } = props;
  const ssh = connection.ssh?.auth.method === "agent" ? null : connection.ssh;
  const [password, setPassword] = useState("");
  const [sshSecret, setSshSecret] = useState("");
  const first = useRef<HTMLInputElement>(null);
  useEffect(() => first.current?.focus(), []);

  return (
    <div className="sheet-layer">
      <button type="button" className="sheet-scrim" aria-label="Cancel" tabIndex={-1} onClick={props.onClose} />
      <form
        className="sheet"
        role="dialog"
        aria-modal="true"
        aria-labelledby="password-title"
        onSubmit={(e) => {
          e.preventDefault();
          props.onConnect({ password, sshSecret: ssh ? sshSecret : null });
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape") props.onClose();
        }}
      >
        <h2 id="password-title">Connect to {connection.name}</h2>
        <p className="muted">
          {connection.user}@{connection.host}. This connection doesn't save its password, so it's asked for each time.
        </p>
        <label className="sheet-label" htmlFor="typed-password">
          Password
        </label>
        <input id="typed-password" ref={first} type="password" className="input" value={password} onChange={(e) => setPassword(e.target.value)} />
        {ssh && (
          <>
            <label className="sheet-label" htmlFor="typed-ssh">
              {ssh.auth.method === "key" ? "Key passphrase" : "SSH password"}
            </label>
            <input id="typed-ssh" type="password" className="input" value={sshSecret} onChange={(e) => setSshSecret(e.target.value)} />
          </>
        )}
        {props.error && (
          <p className="status-error" role="alert">
            {props.error}
          </p>
        )}
        <div className="sheet-actions">
          <button type="button" className="btn" onClick={props.onClose}>
            Cancel
          </button>
          <button type="submit" className="btn btn-primary" disabled={props.busy}>
            {props.busy ? "Connecting…" : "Connect"}
          </button>
        </div>
      </form>
    </div>
  );
}
