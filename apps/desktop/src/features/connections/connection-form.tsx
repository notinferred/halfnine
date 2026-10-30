import { type FormEvent, type ReactNode, useEffect, useState } from "react";
import {
  type AgentAccess,
  type ConnectionDraft,
  type ConnectionSpec,
  commands,
  type Label,
  type SecretChange,
  type TlsMode,
  type Typed,
} from "../../ipc/bindings.ts";
import { describe } from "../../ipc/errors.ts";
import { isMac } from "../../ui/shortcuts.ts";

type SshMethod = "agent" | "key" | "password";

type Form = {
  name: string;
  url: string;
  host: string;
  port: string;
  user: string;
  database: string;
  tls: TlsMode;
  label: Label;
  readOnly: boolean;
  /** `null` until someone picks a level, which keeps the label's default. */
  agentAccess: AgentAccess | null;
  /** `null` until typed into, which keeps the secret already in the keychain. */
  password: string | null;
  savePassword: boolean;
  ssh: boolean;
  sshHost: string;
  sshPort: string;
  sshUser: string;
  sshMethod: SshMethod;
  sshKeyPath: string;
  sshSecret: string | null;
};

type Status = { kind: "idle" } | { kind: "busy"; text: string } | { kind: "ok"; text: string } | { kind: "error"; text: string };

export const LABELS: { value: Label; name: string }[] = [
  { value: "none", name: "No label" },
  { value: "green", name: "Green label" },
  { value: "amber", name: "Amber label" },
  { value: "red", name: "Red label" },
];

export const AGENT_ACCESS: { value: AgentAccess; name: string }[] = [
  { value: "off", name: "Off" },
  { value: "read", name: "Read" },
  { value: "approvedWrites", name: "Read with approved writes" },
  { value: "full", name: "Full" },
];

/** What agents get until a level is picked, as `ConnectionSpec::agent_access` decides it. */
const defaultAccess = (label: Label): AgentAccess => (label === "red" ? "off" : "read");

const SAVE_IN = isMac ? "Save in Keychain" : "Save in Credential Manager";

function initial(spec: ConnectionSpec | null): Form {
  return {
    name: spec?.name ?? "",
    url: "",
    host: spec?.host ?? "",
    port: String(spec?.port ?? 5432),
    user: spec?.user ?? "",
    database: spec?.database ?? "",
    tls: spec?.tls ?? "prefer",
    label: spec?.label ?? "none",
    readOnly: spec?.readOnly ?? false,
    agentAccess: spec?.agentAccess ?? null,
    password: null,
    savePassword: spec?.savePassword !== false,
    ssh: spec?.ssh != null,
    sshHost: spec?.ssh?.host ?? "",
    sshPort: String(spec?.ssh?.port ?? 22),
    sshUser: spec?.ssh?.user ?? "",
    sshMethod: spec?.ssh?.auth.method ?? "agent",
    sshKeyPath: spec?.ssh?.auth.method === "key" ? spec.ssh.auth.path : "~/.ssh/id_ed25519",
    sshSecret: null,
  };
}

const secret = (value: string | null): SecretChange =>
  value === null ? { action: "keep" } : value === "" ? { action: "clear" } : { action: "set", value };

function draftOf(form: Form, id: string | null): ConnectionDraft {
  return {
    id,
    name: form.name,
    host: form.host,
    port: Number.parseInt(form.port, 10) || 5432,
    user: form.user,
    database: form.database,
    tls: form.tls,
    label: form.label,
    readOnly: form.readOnly,
    ssh: form.ssh
      ? {
          host: form.sshHost,
          port: Number.parseInt(form.sshPort, 10) || 22,
          user: form.sshUser,
          auth: form.sshMethod === "key" ? { method: "key", path: form.sshKeyPath } : { method: form.sshMethod },
        }
      : null,
    savePassword: form.savePassword,
    agentAccess: form.agentAccess,
    password: secret(form.password),
    // Turning SSH off leaves its saved secret alone, so saving the form needn't touch the keychain.
    sshSecret: form.ssh && form.sshMethod !== "agent" ? secret(form.sshSecret) : { action: "keep" },
  };
}

export function ConnectionForm(props: {
  saved: ConnectionSpec | null;
  openError: string | null;
  opening: boolean;
  onSaved: (id: string) => Promise<void>;
  onDeleted: () => Promise<void>;
  /** Opens the connection, with passwords typed in the form when it asks for them each time. */
  onOpen: (connection: ConnectionSpec, typed: Typed | null) => Promise<void>;
}) {
  const { saved } = props;
  const [form, setForm] = useState(() => initial(saved));
  const [status, setStatus] = useState<Status>({ kind: "idle" });
  useEffect(() => {
    if (props.openError) setStatus({ kind: "error", text: props.openError });
  }, [props.openError]);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const set = <K extends keyof Form>(key: K, value: Form[K]) => setForm((f) => ({ ...f, [key]: value }));
  const id = saved?.id ?? null;
  const busy = status.kind === "busy" || props.opening;

  const pasteUrl = async (url: string) => {
    set("url", url);
    if (!/^postgres(ql)?:\/\//i.test(url.trim())) return;
    try {
      const parts = await commands.parseConnectionUrl(url);
      setForm((f) => ({
        ...f,
        host: parts.host,
        port: String(parts.port),
        user: parts.user || f.user,
        database: parts.database || f.database,
        tls: parts.tls ?? f.tls,
        password: parts.password ?? f.password,
        name: f.name || parts.host,
      }));
      setStatus({ kind: "idle" });
    } catch (error) {
      setStatus({ kind: "error", text: describe(error) });
    }
  };

  const run = async (text: string, action: () => Promise<void>) => {
    setStatus({ kind: "busy", text });
    try {
      await action();
    } catch (error) {
      setStatus({ kind: "error", text: describe(error) });
    }
  };

  const test = () =>
    run("Testing…", async () => {
      const result = await commands.testConnection(draftOf(form, id));
      setStatus({ kind: "ok", text: `Connected in ${Math.round(result.elapsedMs ?? 0)} ms · PostgreSQL ${result.serverVersion}` });
    });

  const save = () => commands.saveConnection(draftOf(form, id));
  const needsSshSecret = form.ssh && form.sshMethod !== "agent";

  const connect = (event: FormEvent) => {
    event.preventDefault();
    void run("Connecting…", async () => {
      // Saved and listed first, so a connection that fails to open is still there to fix.
      const spec = await save();
      await props.onSaved(spec.id);
      const typed =
        !form.savePassword && form.password !== null ? { password: form.password, sshSecret: needsSshSecret ? form.sshSecret : null } : null;
      await props.onOpen(spec, typed);
      // Still here when the password has to be asked for first.
      setStatus({ kind: "idle" });
    });
  };

  const remove = () =>
    run("Deleting…", async () => {
      if (id) await commands.deleteConnection(id);
      await props.onDeleted();
    });

  // The keychain isn't read to fill these in: an untouched field keeps whatever is saved.
  const wasSaving = saved !== null && saved.savePassword !== false;
  const hint = (value: string | null) =>
    !form.savePassword
      ? wasSaving
        ? "Removed from the Keychain when you save"
        : "Asked each time you connect"
      : wasSaving && value === null
        ? "Unchanged unless you type a new one"
        : saved
          ? "Type it to save it"
          : "Optional";

  return (
    <form className="connection-form" onSubmit={connect}>
      <header className="connection-form-header">
        <div className="connection-form-title">
          <span className="dot" data-label={form.label} aria-hidden="true" />
          <h1>{form.name || saved?.name || "New connection"}</h1>
        </div>
        <p className="muted">{saved ? `PostgreSQL · ${saved.host} · ${saved.database}` : "Paste a URL or fill in the fields."}</p>
      </header>

      <div className="field-stack">
        <label htmlFor="connection-url" className="field-caption">
          URL
        </label>
        <input
          id="connection-url"
          className="input mono"
          placeholder="postgres://user@host:5432/database"
          spellCheck={false}
          value={form.url}
          onChange={(e) => void pasteUrl(e.target.value)}
        />
        <span className="field-note">Pasting a URL fills in the fields below.</span>
      </div>

      <div className="group">
        <Row label="Name" htmlFor="connection-name">
          <input id="connection-name" placeholder="Production" value={form.name} onChange={(e) => set("name", e.target.value)} />
        </Row>
        <Row label="Host" htmlFor="connection-host">
          <input
            id="connection-host"
            placeholder="db.example.com"
            spellCheck={false}
            value={form.host}
            onChange={(e) => set("host", e.target.value)}
          />
        </Row>
        <Row label="Port" htmlFor="connection-port">
          <input id="connection-port" inputMode="numeric" value={form.port} onChange={(e) => set("port", e.target.value.replace(/\D/g, ""))} />
        </Row>
        <Row label="User" htmlFor="connection-user">
          <input id="connection-user" placeholder="postgres" spellCheck={false} value={form.user} onChange={(e) => set("user", e.target.value)} />
        </Row>
        <Row label="Password" htmlFor="connection-password" className="row-password">
          <input
            id="connection-password"
            type="password"
            placeholder={hint(form.password)}
            value={form.password ?? ""}
            onChange={(e) => set("password", e.target.value)}
          />
          <select
            aria-label="Where the passwords are kept"
            title="Applies to the SSH password too"
            value={form.savePassword ? "save" : "ask"}
            onChange={(e) => set("savePassword", e.target.value === "save")}
          >
            <option value="save">{SAVE_IN}</option>
            <option value="ask">Ask each time</option>
          </select>
        </Row>
        <Row label="Database" htmlFor="connection-database">
          <input
            id="connection-database"
            placeholder="postgres"
            spellCheck={false}
            value={form.database}
            onChange={(e) => set("database", e.target.value)}
          />
        </Row>
        <Row label="TLS" htmlFor="connection-tls">
          <select id="connection-tls" value={form.tls} onChange={(e) => set("tls", e.target.value as TlsMode)}>
            <option value="disable">Off</option>
            <option value="prefer">Prefer</option>
            <option value="require">Require</option>
            <option value="verify-full">Verify certificate and host</option>
          </select>
        </Row>
      </div>

      <div className="group">
        <div className="row row-switch">
          <span id="ssh-heading" className="row-label">
            SSH tunnel
          </span>
          <span className="field-note">Reach the database through an SSH server.</span>
          <button type="button" className="switch" aria-labelledby="ssh-heading" aria-pressed={form.ssh} onClick={() => set("ssh", !form.ssh)} />
        </div>
        {form.ssh && (
          <>
            <Row label="SSH host" htmlFor="ssh-host">
              <input
                id="ssh-host"
                placeholder="bastion.example.com"
                spellCheck={false}
                value={form.sshHost}
                onChange={(e) => set("sshHost", e.target.value)}
              />
            </Row>
            <Row label="SSH port" htmlFor="ssh-port">
              <input id="ssh-port" inputMode="numeric" value={form.sshPort} onChange={(e) => set("sshPort", e.target.value.replace(/\D/g, ""))} />
            </Row>
            <Row label="SSH user" htmlFor="ssh-user">
              <input id="ssh-user" spellCheck={false} value={form.sshUser} onChange={(e) => set("sshUser", e.target.value)} />
            </Row>
            <Row label="Sign in with" htmlFor="ssh-method">
              <select id="ssh-method" value={form.sshMethod} onChange={(e) => set("sshMethod", e.target.value as SshMethod)}>
                <option value="agent">SSH agent</option>
                <option value="key">Key file</option>
                <option value="password">Password</option>
              </select>
            </Row>
            {form.sshMethod === "key" && (
              <Row label="Key file" htmlFor="ssh-key">
                <input id="ssh-key" className="mono" spellCheck={false} value={form.sshKeyPath} onChange={(e) => set("sshKeyPath", e.target.value)} />
              </Row>
            )}
            {form.sshMethod !== "agent" && (
              <Row label={form.sshMethod === "key" ? "Passphrase" : "SSH password"} htmlFor="ssh-secret">
                <input
                  id="ssh-secret"
                  type="password"
                  placeholder={hint(form.sshSecret)}
                  value={form.sshSecret ?? ""}
                  onChange={(e) => set("sshSecret", e.target.value)}
                />
              </Row>
            )}
          </>
        )}
      </div>

      <div className="group">
        <div className="row">
          <span id="label-heading" className="row-label">
            Label
          </span>
          <fieldset aria-labelledby="label-heading" className="swatches">
            {LABELS.map((l) => (
              <button
                key={l.value}
                type="button"
                className="swatch dot"
                data-label={l.value}
                aria-label={l.name}
                aria-pressed={form.label === l.value}
                onClick={() => set("label", l.value)}
              />
            ))}
          </fieldset>
        </div>
        <div className="row row-switch">
          <span id="read-only-heading" className="row-label">
            Read-only
          </span>
          <span className="field-note">Postgres refuses every write made through this connection.</span>
          <button
            type="button"
            className="switch"
            aria-labelledby="read-only-heading"
            aria-pressed={form.readOnly}
            onClick={() => set("readOnly", !form.readOnly)}
          />
        </div>
        <div className="row row-switch">
          <label htmlFor="agent-access" className="row-label">
            Agents
          </label>
          <span className="field-note">
            {form.readOnly ? "Read-only, so agents only ever read here." : "What AI agents may do here through Halfnine."}
          </span>
          <select
            id="agent-access"
            value={form.agentAccess ?? defaultAccess(form.label)}
            onChange={(e) => set("agentAccess", e.target.value as AgentAccess)}
          >
            {AGENT_ACCESS.map((a) => (
              <option key={a.value} value={a.value}>
                {a.name}
              </option>
            ))}
          </select>
        </div>
      </div>

      <footer className="connection-form-actions">
        <p role="status" className={`connection-status ${status.kind === "ok" ? "status-ok" : status.kind === "error" ? "status-error" : "muted"}`}>
          {status.kind !== "idle" && status.text}
        </p>
        {saved &&
          (confirmingDelete ? (
            <>
              <button type="button" className="btn btn-quiet" onClick={() => setConfirmingDelete(false)}>
                Keep
              </button>
              <button type="button" className="btn btn-danger" disabled={busy} onClick={() => void remove()}>
                Delete {saved.name}
              </button>
            </>
          ) : (
            <button type="button" className="btn btn-quiet" onClick={() => setConfirmingDelete(true)}>
              Delete
            </button>
          ))}
        <button type="button" className="btn" disabled={busy} onClick={() => void test()}>
          Test
        </button>
        <button
          type="button"
          className="btn"
          disabled={busy}
          onClick={() =>
            void run("Saving…", async () => {
              await props.onSaved((await save()).id);
            })
          }
        >
          Save
        </button>
        <button type="submit" className="btn btn-primary" disabled={busy}>
          Connect
        </button>
      </footer>
    </form>
  );
}

function Row(props: { label: string; htmlFor: string; className?: string; children: ReactNode }) {
  return (
    <div className={props.className ? `row ${props.className}` : "row"}>
      <label htmlFor={props.htmlFor} className="row-label">
        {props.label}
      </label>
      {props.children}
    </div>
  );
}
