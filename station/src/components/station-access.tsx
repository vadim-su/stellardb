import {
  ArrowRight,
  Check,
  Copy,
  EyeOff,
  KeyRound,
  LockKeyhole,
  Plus,
  RefreshCw,
  ShieldCheck,
  Trash2,
  UserRound,
  UsersRound,
  X,
} from "lucide-react";
import { useMemo, useState, type FormEvent } from "react";
import type {
  AccessSnapshot,
  CreatedApiKey,
  ManagedApiKey,
  ManagedUser,
} from "../lib/station-api";

type AccessTab = "users" | "keys";
type AccessDialog =
  | { kind: "createUser" }
  | { kind: "editUser"; user: ManagedUser }
  | { kind: "password"; user: ManagedUser }
  | { kind: "deleteUser"; user: ManagedUser }
  | { kind: "createKey"; username: string }
  | { kind: "deleteKey"; key: ManagedApiKey; current: boolean };

interface StationAccessProps {
  snapshot: AccessSnapshot | null;
  loading: boolean;
  error: string | null;
  onRefresh: () => void;
  onCreateUser: (request: {
    username: string;
    password: string;
    attributes: Record<string, string>;
  }) => Promise<void>;
  onUpdateUser: (
    username: string,
    request: { set: Record<string, string>; remove: string[] },
  ) => Promise<void>;
  onUpdatePassword: (username: string, password: string) => Promise<void>;
  onDropUser: (username: string) => Promise<void>;
  onCreateApiKey: (request: { name: string; username: string }) => Promise<CreatedApiKey>;
  onDropApiKey: (name: string) => Promise<void>;
}

function attributesText(attributes: Record<string, string>): string {
  return Object.entries(attributes)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([key, value]) => `${key}=${value}`)
    .join("\n");
}

function parseAttributes(value: string): Record<string, string> {
  const result: Record<string, string> = {};
  for (const rawLine of value.split("\n")) {
    const line = rawLine.trim();
    if (!line) continue;
    const separator = line.indexOf("=");
    if (separator <= 0) throw new Error(`Invalid attribute “${line}”. Use key=value.`);
    const key = line.slice(0, separator).trim();
    if (!key) throw new Error("Attribute name cannot be empty.");
    result[key] = line.slice(separator + 1).trim();
  }
  return result;
}

function shortDate(value: string | null): string {
  if (!value) return "Legacy";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "Legacy";
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    year: date.getFullYear() === new Date().getFullYear() ? undefined : "numeric",
  }).format(date);
}

export function StationAccess({
  snapshot,
  loading,
  error,
  onRefresh,
  onCreateUser,
  onUpdateUser,
  onUpdatePassword,
  onDropUser,
  onCreateApiKey,
  onDropApiKey,
}: StationAccessProps) {
  const [tab, setTab] = useState<AccessTab>("users");
  const [search, setSearch] = useState("");
  const [dialog, setDialog] = useState<AccessDialog | null>(null);
  const [createdKey, setCreatedKey] = useState<CreatedApiKey | null>(null);
  const [copied, setCopied] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const users = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return (snapshot?.users ?? []).filter((user) =>
      !needle ||
      user.username.toLowerCase().includes(needle) ||
      Object.entries(user.attributes).some(([key, value]) =>
        `${key}=${value}`.toLowerCase().includes(needle),
      ),
    );
  }, [search, snapshot?.users]);

  const keys = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return (snapshot?.apiKeys ?? []).filter((key) =>
      !needle || key.name.toLowerCase().includes(needle) || key.username.toLowerCase().includes(needle),
    );
  }, [search, snapshot?.apiKeys]);

  function openDialog(next: AccessDialog) {
    setActionError(null);
    setDialog(next);
  }

  async function perform(action: () => Promise<void>) {
    setSubmitting(true);
    setActionError(null);
    try {
      await action();
      setDialog(null);
    } catch (caught) {
      setActionError(caught instanceof Error ? caught.message : "Access operation failed.");
    } finally {
      setSubmitting(false);
    }
  }

  async function copySecret() {
    if (!createdKey) return;
    await navigator.clipboard.writeText(createdKey.secret);
    setCopied(true);
  }

  function closeSecret() {
    setCreatedKey(null);
    setCopied(false);
  }

  return (
    <div className="view access-view">
      <div className="access-titlebar">
        <div>
          <p className="eyebrow"><ShieldCheck size={13} /> ACCESS CONTROL</p>
          <h1>Control who gets in</h1>
          <p>Manage identities and credentials without exposing the private system database.</p>
        </div>
        <button className="secondary-button" onClick={onRefresh} disabled={loading}>
          <RefreshCw size={15} className={loading ? "spin" : ""} /> Refresh
        </button>
      </div>

      {snapshot && (
        <section className="access-identity-card">
          <span className="access-identity-icon"><UserRound size={18} /></span>
          <div>
            <small>CONNECTED AS</small>
            <strong>{snapshot.identity.username}</strong>
            <span>
              {snapshot.identity.authMethod === "apiKey"
                ? `API key · ${snapshot.identity.credentialName ?? "legacy credential"}`
                : "Username & password session"}
            </span>
          </div>
          <span className="access-live"><i /> AUTHENTICATED</span>
        </section>
      )}

      {(error || actionError) && (
        <div className="inline-alert access-alert" role="alert">
          <LockKeyhole size={17} />
          <span>{actionError ?? error}</span>
          {error && <button onClick={onRefresh}>Retry</button>}
        </div>
      )}

      <section className="station-panel access-panel">
        <header className="access-panel-header">
          <div className="access-tabs">
            <button className={tab === "users" ? "is-active" : ""} onClick={() => setTab("users")}>
              <UsersRound size={15} /> Users <span>{snapshot?.users.length ?? 0}</span>
            </button>
            <button className={tab === "keys" ? "is-active" : ""} onClick={() => setTab("keys")}>
              <KeyRound size={15} /> API keys <span>{snapshot?.apiKeys.length ?? 0}</span>
            </button>
          </div>
          <div className="access-panel-actions">
            <input
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              placeholder={`Search ${tab === "users" ? "users" : "keys"}…`}
              aria-label={`Search ${tab === "users" ? "users" : "API keys"}`}
            />
            <button
              className="primary-button"
              onClick={() => openDialog(tab === "users"
                ? { kind: "createUser" }
                : { kind: "createKey", username: snapshot?.identity.username ?? "root" })}
              disabled={!snapshot}
            >
              <Plus size={14} /> {tab === "users" ? "New user" : "New key"}
            </button>
          </div>
        </header>

        {tab === "users" ? (
          <div className="access-list">
            <div className="access-list-labels user-columns">
              <span>User</span><span>Attributes</span><span>API keys</span><span />
            </div>
            {users.map((user) => (
              <article className="access-row user-columns" key={user.username}>
                <div className="access-principal">
                  <span><UserRound size={15} /></span>
                  <div>
                    <strong>{user.username}</strong>
                    <small>
                      {user.username === snapshot?.identity.username && "You · "}
                      {user.protected ? "Protected" : `Created ${shortDate(user.createdAt)}`}
                    </small>
                  </div>
                </div>
                <div className="attribute-chips">
                  {Object.entries(user.attributes).slice(0, 3).map(([key, value]) => (
                    <span key={key}>{key}=<strong>{value}</strong></span>
                  ))}
                  {Object.keys(user.attributes).length === 0 && <i>No attributes</i>}
                  {Object.keys(user.attributes).length > 3 && <i>+{Object.keys(user.attributes).length - 3}</i>}
                </div>
                <span className="access-key-count"><KeyRound size={13} /> {user.apiKeyCount}</span>
                <div className="access-row-actions">
                  <button onClick={() => openDialog({ kind: "createKey", username: user.username })}>Add key</button>
                  <button onClick={() => openDialog({ kind: "editUser", user })}>Edit</button>
                  <button onClick={() => openDialog({ kind: "password", user })}>Password</button>
                  <button
                    className="danger"
                    disabled={user.protected}
                    onClick={() => openDialog({ kind: "deleteUser", user })}
                    aria-label={`Delete ${user.username}`}
                  ><Trash2 size={13} /></button>
                </div>
              </article>
            ))}
            {!loading && users.length === 0 && <div className="panel-empty">No users match this search.</div>}
          </div>
        ) : (
          <div className="access-list">
            <div className="access-list-labels key-columns">
              <span>Credential</span><span>Owner</span><span>Created</span><span />
            </div>
            {keys.map((key) => {
              const current = snapshot?.identity.authMethod === "apiKey" && snapshot.identity.credentialName === key.name;
              return (
                <article className="access-row key-columns" key={key.name}>
                  <div className="access-principal">
                    <span><KeyRound size={15} /></span>
                    <div><strong>{key.name}</strong><small>{key.hint ?? "Legacy key"}</small></div>
                  </div>
                  <span>{key.username}{current && <em>THIS CONNECTION</em>}</span>
                  <span>{shortDate(key.createdAt)}</span>
                  <div className="access-row-actions">
                    <button
                      className="danger-text"
                      onClick={() => openDialog({ kind: "deleteKey", key, current })}
                    >Revoke</button>
                  </div>
                </article>
              );
            })}
            {!loading && keys.length === 0 && <div className="panel-empty">No API keys match this search.</div>}
          </div>
        )}
      </section>

      {dialog && (
        <AccessDialogPanel
          dialog={dialog}
          users={snapshot?.users ?? []}
          currentUsername={snapshot?.identity.username ?? ""}
          submitting={submitting}
          error={actionError}
          onClose={() => setDialog(null)}
          onSubmit={(event) => {
            event.preventDefault();
            const form = new FormData(event.currentTarget);
            switch (dialog.kind) {
              case "createUser":
                try {
                  parseAttributes(String(form.get("attributes") ?? ""));
                } catch (caught) {
                  setActionError(caught instanceof Error ? caught.message : "Invalid attributes.");
                  return;
                }
                void perform(() => onCreateUser({
                  username: String(form.get("username") ?? ""),
                  password: String(form.get("password") ?? ""),
                  attributes: parseAttributes(String(form.get("attributes") ?? "")),
                }));
                break;
              case "editUser": {
                let next: Record<string, string>;
                try {
                  next = parseAttributes(String(form.get("attributes") ?? ""));
                } catch (caught) {
                  setActionError(caught instanceof Error ? caught.message : "Invalid attributes.");
                  return;
                }
                const remove = Object.keys(dialog.user.attributes).filter((key) => !(key in next));
                void perform(() => onUpdateUser(dialog.user.username, { set: next, remove }));
                break;
              }
              case "password":
                void perform(() => onUpdatePassword(
                  dialog.user.username,
                  String(form.get("password") ?? ""),
                ));
                break;
              case "deleteUser":
                void perform(() => onDropUser(dialog.user.username));
                break;
              case "createKey":
                setSubmitting(true);
                setActionError(null);
                void onCreateApiKey({
                  name: String(form.get("name") ?? ""),
                  username: String(form.get("username") ?? dialog.username),
                }).then((created) => {
                  setDialog(null);
                  setCreatedKey(created);
                }).catch((caught: unknown) => {
                  setActionError(caught instanceof Error ? caught.message : "Could not create API key.");
                }).finally(() => setSubmitting(false));
                break;
              case "deleteKey":
                void perform(() => onDropApiKey(dialog.key.name));
                break;
            }
          }}
        />
      )}

      {createdKey && (
        <div className="access-dialog-overlay" role="presentation">
          <section className="access-secret-dialog" role="dialog" aria-modal="true" aria-labelledby="secret-title">
            <div className="secret-success"><Check size={20} /></div>
            <p className="eyebrow">ONE-TIME SECRET</p>
            <h2 id="secret-title">API key created</h2>
            <p>Copy this value now. StellarDB will never return it again.</p>
            <code>{createdKey.secret}</code>
            <button className="primary-button" onClick={() => void copySecret()}>
              {copied ? <Check size={15} /> : <Copy size={15} />}
              {copied ? "Copied" : "Copy API key"}
            </button>
            <div className="secret-warning"><EyeOff size={14} /> The secret exists only in this dialog and is not saved by Station.</div>
            <button className="secret-close" onClick={closeSecret}>
              {copied ? "Done" : "Close without copying"}
            </button>
          </section>
        </div>
      )}
    </div>
  );
}

function AccessDialogPanel({
  dialog,
  users,
  currentUsername,
  submitting,
  error,
  onClose,
  onSubmit,
}: {
  dialog: AccessDialog;
  users: ManagedUser[];
  currentUsername: string;
  submitting: boolean;
  error: string | null;
  onClose: () => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
}) {
  const title = {
    createUser: "Create user",
    editUser: "Edit attributes",
    password: "Reset password",
    deleteUser: "Delete user",
    createKey: "Create API key",
    deleteKey: "Revoke API key",
  }[dialog.kind];

  return (
    <div className="access-dialog-overlay" role="presentation">
      <section className="access-dialog" role="dialog" aria-modal="true" aria-labelledby="access-dialog-title">
        <header>
          <div><small>ACCESS CONTROL</small><h2 id="access-dialog-title">{title}</h2></div>
          <button onClick={onClose} aria-label="Close"><X size={16} /></button>
        </header>
        <form onSubmit={onSubmit}>
          {dialog.kind === "createUser" && (
            <>
              <label><span>Username</span><input name="username" autoFocus required autoComplete="off" /></label>
              <label><span>Temporary password</span><input name="password" type="password" required autoComplete="new-password" /></label>
              <label><span>Attributes <small>one key=value per line</small></span><textarea name="attributes" placeholder={"role=reader\nteam=analytics"} /></label>
            </>
          )}
          {dialog.kind === "editUser" && (
            <label><span>Attributes for {dialog.user.username} <small>one key=value per line</small></span><textarea name="attributes" autoFocus defaultValue={attributesText(dialog.user.attributes)} /></label>
          )}
          {dialog.kind === "password" && (
            <>
              <p>Set a new password for <strong>{dialog.user.username}</strong>.</p>
              {dialog.user.username === currentUsername && <div className="dialog-warning">This revokes your current session. Station will reconnect using the new password when possible.</div>}
              <label><span>New password</span><input name="password" type="password" autoFocus required autoComplete="new-password" /></label>
            </>
          )}
          {dialog.kind === "deleteUser" && (
            <div className="dialog-danger">
              <Trash2 size={20} />
              <strong>Delete {dialog.user.username}?</strong>
              <span>{dialog.user.apiKeyCount} associated API keys will be revoked permanently.</span>
            </div>
          )}
          {dialog.kind === "createKey" && (
            <>
              <label><span>Key name</span><input name="name" autoFocus required placeholder="station-production" autoComplete="off" /></label>
              <fieldset className="access-user-picker">
                <legend>Owner</legend>
                {users.map((user) => (
                  <label key={user.username}>
                    <input type="radio" name="username" value={user.username} defaultChecked={user.username === dialog.username} />
                    <span>{user.username}{user.username === currentUsername && <small>You</small>}</span>
                  </label>
                ))}
              </fieldset>
              <div className="dialog-warning">The key inherits all permissions and attributes of its owner.</div>
            </>
          )}
          {dialog.kind === "deleteKey" && (
            <div className="dialog-danger">
              <KeyRound size={20} />
              <strong>Revoke {dialog.key.name}?</strong>
              <span>Applications using this credential will immediately lose access.</span>
              {dialog.current
                ? <small>This is the current Station connection. You will need another credential to reconnect.</small>
                : <small>Owner: {dialog.key.username}</small>}
            </div>
          )}
          {error && <div className="connection-error">{error}</div>}
          <footer>
            <button type="button" className="secondary-button" onClick={onClose}>Cancel</button>
            <button className={`primary-button ${dialog.kind === "deleteUser" || dialog.kind === "deleteKey" ? "danger-button" : ""}`} disabled={submitting}>
              {submitting ? "Working…" : title} <ArrowRight size={14} />
            </button>
          </footer>
        </form>
      </section>
    </div>
  );
}
