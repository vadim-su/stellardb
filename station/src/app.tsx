import {
  Activity,
  ArrowUpRight,
  Boxes,
  Check,
  CircleAlert,
  Code2,
  Database,
  EyeOff,
  Gauge,
  HardDrive,
  KeyRound,
  LoaderCircle,
  LockKeyhole,
  Play,
  Plus,
  Radio,
  RefreshCw,
  RotateCcw,
  Search,
  Server,
  ShieldCheck,
  Square,
  Table2,
  TerminalSquare,
  UserRound,
  X,
  Zap,
} from "lucide-react";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
} from "react";
import type { CollectionOverview } from "@stellardb/client";
import {
  createManagedApiKey,
  createManagedUser,
  dropManagedApiKey,
  dropManagedUser,
  executeStationQuery,
  isAuthenticationError,
  listDatabases,
  loginWithCredentials,
  loadAccessSnapshot,
  loadSnapshot,
  normalizeBaseUrl,
  readableError,
  updateManagedUser,
  updateManagedUserPassword,
  type AccessSnapshot,
  type CreatedApiKey,
  type QueryStat,
  type StationConnection,
  type StationQueryResult,
  type StationSnapshot,
} from "./lib/station-api";
import {
  ConnectionMenu,
  DatabaseMenu,
  type StationConnectionRecord,
} from "./components/station-selectors";
import {
  StationEditor,
  type StationEditorHandle,
} from "./components/station-editor";
import { StationSchema } from "./components/station-schema";
import { StationResults } from "./components/station-results";
import { StationAccess } from "./components/station-access";

type StationView = "overview" | "query" | "access";
type ConnectionPhase = "connecting" | "connected" | "disconnected";
type ConnectionAuthMode = "anonymous" | "credentials" | "apiKey";

const SETTINGS_KEY = "stellardb:station:settings";
const DEFAULT_QUERY = `SELECT *
FROM users
LIMIT 100;`;

interface SavedSettings {
  connections: Array<Omit<StationConnectionRecord, "token" | "password">>;
  activeConnectionId: string;
  database?: string;
}

interface QueryTab {
  id: string;
  name: string;
  sql: string;
  result: StationQueryResult | null;
  error: string | null;
}

function defaultConnection(
  baseUrl = import.meta.env.DEV
    ? __STELLARDB_DEV_PROXY_TARGET__
    : window.location.origin,
): StationConnectionRecord {
  const displayUrl = normalizeBaseUrl(baseUrl);
  return {
    id: crypto.randomUUID(),
    name: "Local StellarDB",
    baseUrl: routeThroughDevProxy(displayUrl),
    displayUrl,
    authMode: "anonymous",
  };
}

function hydrateSavedConnection(
  connection: Omit<StationConnectionRecord, "token">,
): StationConnectionRecord {
  const displayUrl = normalizeBaseUrl(connection.displayUrl ?? connection.baseUrl);
  return {
    ...connection,
    baseUrl: routeThroughDevProxy(displayUrl),
    displayUrl,
  };
}

function readSettings(): SavedSettings {
  try {
    const value = localStorage.getItem(SETTINGS_KEY);
    if (!value) {
      const connection = defaultConnection();
      return { connections: [connection], activeConnectionId: connection.id };
    }
    const parsed = JSON.parse(value) as Partial<SavedSettings> & { baseUrl?: string };
    if (Array.isArray(parsed.connections) && parsed.connections.length > 0) {
      const connections = parsed.connections.filter(
        (connection): connection is Omit<StationConnectionRecord, "token" | "password"> =>
          Boolean(connection) &&
          typeof connection.id === "string" &&
          typeof connection.name === "string" &&
          typeof connection.baseUrl === "string",
      ).map(hydrateSavedConnection);
      if (connections.length > 0) {
        const activeConnectionId = connections.some((connection) => connection.id === parsed.activeConnectionId)
          ? parsed.activeConnectionId as string
          : connections[0]!.id;
        return { connections, activeConnectionId, database: typeof parsed.database === "string" ? parsed.database : undefined };
      }
    }
    const connection = defaultConnection(typeof parsed.baseUrl === "string" ? parsed.baseUrl : window.location.origin);
    return {
      connections: [connection],
      activeConnectionId: connection.id,
      database: typeof parsed.database === "string" ? parsed.database : undefined,
    };
  } catch {
    const connection = defaultConnection();
    return { connections: [connection], activeConnectionId: connection.id };
  }
}

function saveSettings(
  connections: StationConnectionRecord[],
  activeConnectionId: string,
  database?: string,
) {
  localStorage.setItem(
    SETTINGS_KEY,
    JSON.stringify({
      connections: connections.map(({ token: _token, password: _password, ...connection }) => connection),
      activeConnectionId,
      database,
    }),
  );
}

function routeThroughDevProxy(baseUrl: string): string {
  if (!import.meta.env.DEV) return baseUrl;
  try {
    const requestedOrigin = new URL(baseUrl).origin;
    const proxyTargetOrigin = new URL(__STELLARDB_DEV_PROXY_TARGET__).origin;
    if (requestedOrigin === proxyTargetOrigin) return window.location.origin;
  } catch {
    // normalizeBaseUrl reports malformed endpoints before this helper runs.
  }
  return baseUrl;
}

function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const exponent = Math.min(
    Math.floor(Math.log(value) / Math.log(1024)),
    units.length - 1,
  );
  const amount = value / 1024 ** exponent;
  return `${amount >= 10 ? amount.toFixed(0) : amount.toFixed(1)} ${units[exponent]}`;
}

function formatDuration(seconds: number): string {
  if (seconds < 60) return `${Math.floor(seconds)}s`;
  if (seconds < 3_600) return `${Math.floor(seconds / 60)}m`;
  if (seconds < 86_400) return `${Math.floor(seconds / 3_600)}h`;
  return `${Math.floor(seconds / 86_400)}d`;
}

function formatRelativeTime(value?: string): string {
  if (!value) return "—";
  const timestamp = new Date(value).getTime();
  if (!Number.isFinite(timestamp)) return "—";
  const elapsedSeconds = Math.max(0, Math.floor((Date.now() - timestamp) / 1_000));
  if (elapsedSeconds < 60) return `${elapsedSeconds}s ago`;
  if (elapsedSeconds < 3_600) return `${Math.floor(elapsedSeconds / 60)}m ago`;
  if (elapsedSeconds < 86_400) return `${Math.floor(elapsedSeconds / 3_600)}h ago`;
  return `${Math.floor(elapsedSeconds / 86_400)}d ago`;
}

function queryPatternTemplate(query: QueryStat): string {
  return query.query_example || query.query_normalized;
}

function queryPatternDisplay(query: QueryStat): string {
  return query.query_display || queryPatternTemplate(query) || `Unparsed query · ${query.query_hash.slice(0, 8)}`;
}

function queryPatternKind(query: QueryStat): string {
  return query.statement_kind || queryPatternTemplate(query).trim().split(/\s+/, 1)[0]?.toUpperCase() || "UNPARSED";
}

function shortEndpoint(baseUrl: string): string {
  try {
    return new URL(baseUrl).host;
  } catch {
    return baseUrl;
  }
}

async function withCredentialRefresh<T>(
  connection: StationConnectionRecord,
  operation: (connection: StationConnectionRecord) => Promise<T>,
  onRenewed: (connection: StationConnectionRecord) => void,
): Promise<T> {
  try {
    return await operation(connection);
  } catch (error) {
    if (
      !isAuthenticationError(error) ||
      connection.authMode !== "credentials" ||
      !connection.username ||
      !connection.password
    ) {
      throw error;
    }
    const token = await loginWithCredentials(connection.baseUrl, {
      username: connection.username,
      password: connection.password,
    });
    const renewed = { ...connection, token };
    onRenewed(renewed);
    return operation(renewed);
  }
}

export function App() {
  const initialSettings = useMemo(readSettings, []);
  const [connections, setConnections] = useState<StationConnectionRecord[]>(
    initialSettings.connections,
  );
  const [activeConnectionId, setActiveConnectionId] = useState(
    initialSettings.activeConnectionId,
  );
  const connection = useMemo(
    () => hydrateSavedConnection(
      connections.find((item) => item.id === activeConnectionId) ?? connections[0]!,
    ),
    [activeConnectionId, connections],
  );
  const [phase, setPhase] = useState<ConnectionPhase>("connecting");
  const [connectionError, setConnectionError] = useState<string | null>(null);
  const [databases, setDatabases] = useState<string[]>([]);
  const [database, setDatabase] = useState(initialSettings.database ?? "");
  const [connectionDialog, setConnectionDialog] = useState<{
    open: boolean;
    connectionId?: string;
  }>({ open: false });
  const [view, setView] = useState<StationView>("overview");
  const [snapshot, setSnapshot] = useState<StationSnapshot | null>(null);
  const [snapshotError, setSnapshotError] = useState<string | null>(null);
  const [snapshotLoading, setSnapshotLoading] = useState(false);
  const [accessSnapshot, setAccessSnapshot] = useState<AccessSnapshot | null>(null);
  const [accessError, setAccessError] = useState<string | null>(null);
  const [accessLoading, setAccessLoading] = useState(false);
  const initialTab = useMemo<QueryTab>(() => ({ id: crypto.randomUUID(), name: "Query 1", sql: DEFAULT_QUERY, result: null, error: null }), []);
  const [queryTabs, setQueryTabs] = useState<QueryTab[]>([initialTab]);
  const [activeQueryId, setActiveQueryId] = useState(initialTab.id);
  const [queryRunning, setQueryRunning] = useState(false);
  const queryController = useRef<AbortController | null>(null);
  const activeQuery = queryTabs.find((tab) => tab.id === activeQueryId) ?? queryTabs[0]!;

  const rememberRenewedConnection = useCallback((renewed: StationConnectionRecord) => {
    setConnections((current) => current.map((item) => item.id === renewed.id ? renewed : item));
  }, []);

  const refreshSnapshot = useCallback(async () => {
    if (phase !== "connected" || !database) return;
    setSnapshotLoading(true);
    setSnapshotError(null);
    try {
      const next = await withCredentialRefresh(
        connection,
        (active) => loadSnapshot(active, database),
        rememberRenewedConnection,
      );
      setSnapshot(next);
    } catch (error) {
      setSnapshotError(readableError(error));
    } finally {
      setSnapshotLoading(false);
    }
  }, [connection, database, phase, rememberRenewedConnection]);

  const refreshAccess = useCallback(async () => {
    if (phase !== "connected") return;
    setAccessLoading(true);
    setAccessError(null);
    try {
      const next = await withCredentialRefresh(
        connection,
        (active) => loadAccessSnapshot(active),
        rememberRenewedConnection,
      );
      setAccessSnapshot(next);
    } catch (error) {
      setAccessError(readableError(error));
    } finally {
      setAccessLoading(false);
    }
  }, [connection, phase, rememberRenewedConnection]);

  useEffect(() => {
    const controller = new AbortController();
    async function initialize() {
      if (
        connection.authMode !== "anonymous" &&
        !connection.token &&
        !(connection.authMode === "credentials" && connection.password)
      ) {
        setConnectionError("Session expired. Sign in again to reconnect.");
        setPhase("disconnected");
        return;
      }
      try {
        const available = await withCredentialRefresh(
          connection,
          (active) => listDatabases(active, controller.signal),
          rememberRenewedConnection,
        );
        const selected = available.includes(database)
          ? database
          : (available[0] ?? "");
        setDatabases(available);
        setDatabase(selected);
        setPhase("connected");
        setConnectionError(null);
        saveSettings(connections, activeConnectionId, selected || undefined);
      } catch (error) {
        if (controller.signal.aborted) return;
        setConnectionError(readableError(error));
        setPhase("disconnected");
      }
    }
    void initialize();
    return () => controller.abort();
    // Station only auto-connects once. Subsequent changes go through the form.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (phase !== "connected" || !database) return;
    const controller = new AbortController();
    setSnapshotLoading(true);
    setSnapshotError(null);
    withCredentialRefresh(
      connection,
      (active) => loadSnapshot(active, database, controller.signal),
      rememberRenewedConnection,
    )
      .then(setSnapshot)
      .catch((error: unknown) => {
        if (!controller.signal.aborted) setSnapshotError(readableError(error));
      })
      .finally(() => {
        if (!controller.signal.aborted) setSnapshotLoading(false);
      });
    return () => controller.abort();
  }, [connection, database, phase, rememberRenewedConnection]);

  useEffect(() => {
    if (view !== "access" || phase !== "connected" || accessSnapshot) return;
    void refreshAccess();
  }, [accessSnapshot, phase, refreshAccess, view]);

  async function handleConnect(
    nextConnection: StationConnection,
    details: { name: string; displayUrl: string },
  ) {
    const available = await listDatabases(nextConnection);
    const selected = available.includes(database)
      ? database
      : (available[0] ?? "");
    const id = connectionDialog.connectionId ?? crypto.randomUUID();
    const record: StationConnectionRecord = { ...nextConnection, ...details, id };
    const nextConnections = connections.some((item) => item.id === id)
      ? connections.map((item) => item.id === id ? record : item)
      : [...connections, record];
    setConnections(nextConnections);
    setActiveConnectionId(id);
    setDatabases(available);
    setDatabase(selected);
    setSnapshot(null);
    setAccessSnapshot(null);
    setPhase("connected");
    setConnectionError(null);
    setConnectionDialog({ open: false });
    saveSettings(nextConnections, id, selected || undefined);
  }

  async function activateConnection(
    savedConnection: StationConnectionRecord,
    availableConnections = connections,
  ) {
    const nextConnection = hydrateSavedConnection(savedConnection);
    if (
      nextConnection.authMode !== "anonymous" &&
      !nextConnection.token &&
      !(nextConnection.authMode === "credentials" && nextConnection.password)
    ) {
      setConnectionError("Authentication required for this connection.");
      setConnectionDialog({ open: true, connectionId: nextConnection.id });
      return;
    }
    try {
      const available = await withCredentialRefresh(
        nextConnection,
        (active) => listDatabases(active),
        rememberRenewedConnection,
      );
      const selected = available.includes(database) ? database : (available[0] ?? "");
      setActiveConnectionId(nextConnection.id);
      setDatabases(available);
      setDatabase(selected);
      setSnapshot(null);
      setAccessSnapshot(null);
      setPhase("connected");
      setConnectionError(null);
      setQueryTabs((tabs) => tabs.map((tab) => ({ ...tab, result: null, error: null })));
      saveSettings(availableConnections, nextConnection.id, selected || undefined);
    } catch (error) {
      setConnectionError(readableError(error));
      setConnectionDialog({ open: true, connectionId: nextConnection.id });
    }
  }

  function removeConnection(target: StationConnectionRecord) {
    if (connections.length === 1) {
      setConnectionDialog({ open: true, connectionId: target.id });
      return;
    }
    const nextConnections = connections.filter((item) => item.id !== target.id);
    setConnections(nextConnections);
    if (target.id === activeConnectionId) void activateConnection(nextConnections[0]!, nextConnections);
    else saveSettings(nextConnections, activeConnectionId, database || undefined);
  }

  function selectDatabase(nextDatabase: string) {
    setDatabase(nextDatabase);
    setQueryTabs((tabs) => tabs.map((tab) => ({ ...tab, result: null, error: null })));
    saveSettings(connections, activeConnectionId, nextDatabase);
  }

  function updateActiveQuery(patch: Partial<QueryTab>) {
    setQueryTabs((tabs) => tabs.map((tab) => tab.id === activeQueryId ? { ...tab, ...patch } : tab));
  }

  function addQuery(initialSql = "") {
    const tab: QueryTab = { id: crypto.randomUUID(), name: `Query ${queryTabs.length + 1}`, sql: initialSql, result: null, error: null };
    setQueryTabs((tabs) => [...tabs, tab]);
    setActiveQueryId(tab.id);
  }

  function closeQuery(id: string) {
    if (queryTabs.length === 1) return;
    const index = queryTabs.findIndex((tab) => tab.id === id);
    const next = queryTabs.filter((tab) => tab.id !== id);
    setQueryTabs(next);
    if (activeQueryId === id) setActiveQueryId(next[Math.min(index, next.length - 1)]!.id);
  }

  async function runQuery(sqlOverride?: string) {
    const sql = sqlOverride ?? activeQuery.sql;
    if (!database || !sql.trim()) return;
    if (queryRunning) {
      queryController.current?.abort();
      return;
    }
    const controller = new AbortController();
    queryController.current = controller;
    setQueryRunning(true);
    updateActiveQuery({ error: null });
    const queryId = activeQueryId;
    try {
      const result = await withCredentialRefresh(
        connection,
        (active) => executeStationQuery(
          active,
          database,
          sql,
          controller.signal,
        ),
        rememberRenewedConnection,
      );
      setQueryTabs((tabs) => tabs.map((tab) => tab.id === queryId ? { ...tab, result, error: null } : tab));
      void refreshSnapshot();
    } catch (error) {
      const message = readableError(error);
      setQueryTabs((tabs) => tabs.map((tab) => tab.id === queryId ? { ...tab, result: null, error: message } : tab));
    } finally {
      queryController.current = null;
      setQueryRunning(false);
    }
  }

  async function runAccessRequest<T>(
    operation: (active: StationConnectionRecord) => Promise<T>,
  ): Promise<T> {
    return withCredentialRefresh(connection, operation, rememberRenewedConnection);
  }

  async function handleCreateManagedUser(request: {
    username: string;
    password: string;
    attributes: Record<string, string>;
  }) {
    await runAccessRequest((active) => createManagedUser(active, request));
    setAccessSnapshot(null);
    await refreshAccess();
  }

  async function handleUpdateManagedUser(
    username: string,
    request: { set: Record<string, string>; remove: string[] },
  ) {
    await runAccessRequest((active) => updateManagedUser(active, username, request));
    setAccessSnapshot(null);
    await refreshAccess();
  }

  async function handleUpdateManagedUserPassword(username: string, password: string) {
    await runAccessRequest((active) => updateManagedUserPassword(active, username, password));
    if (
      accessSnapshot?.identity.username === username &&
      connection.authMode === "credentials" &&
      connection.username === username
    ) {
      const token = await loginWithCredentials(connection.baseUrl, { username, password });
      const renewed = { ...connection, password, token };
      rememberRenewedConnection(renewed);
      setAccessSnapshot(await loadAccessSnapshot(renewed));
      return;
    }
    setAccessSnapshot(null);
    await refreshAccess();
  }

  async function handleDropManagedUser(username: string) {
    await runAccessRequest((active) => dropManagedUser(active, username));
    setAccessSnapshot(null);
    await refreshAccess();
  }

  async function handleCreateManagedApiKey(request: {
    name: string;
    username: string;
  }): Promise<CreatedApiKey> {
    const created = await runAccessRequest((active) => createManagedApiKey(active, request));
    setAccessSnapshot((current) => current ? {
      ...current,
      apiKeys: [...current.apiKeys, created.key],
      users: current.users.map((user) => user.username === created.key.username
        ? { ...user, apiKeyCount: user.apiKeyCount + 1 }
        : user),
    } : current);
    return created;
  }

  async function handleDropManagedApiKey(name: string) {
    const revokesCurrent = accessSnapshot?.identity.authMethod === "apiKey"
      && accessSnapshot.identity.credentialName === name;
    const dropped = accessSnapshot?.apiKeys.find((key) => key.name === name);
    await runAccessRequest((active) => dropManagedApiKey(active, name));
    setAccessSnapshot((current) => current ? {
      ...current,
      apiKeys: current.apiKeys.filter((key) => key.name !== name),
      users: current.users.map((user) => user.username === dropped?.username
        ? { ...user, apiKeyCount: Math.max(0, user.apiKeyCount - 1) }
        : user),
    } : current);
    if (revokesCurrent) {
      setAccessError("The API key used by this connection was revoked. Reconnect with another credential.");
    }
  }

  function openCollection(collection: CollectionOverview) {
    updateActiveQuery({ sql: `SELECT *\nFROM ${collection.name}\nLIMIT 100;`, result: null, error: null });
    setView("query");
  }

  if (phase === "connecting") {
    return <StationBoot endpoint={shortEndpoint(connection.displayUrl ?? connection.baseUrl)} />;
  }

  const connectionPanelId = connectionDialog.connectionId ?? (
    phase === "disconnected" ? activeConnectionId : undefined
  );

  return (
    <div className="station-shell">
      <StationHeader
        connected={phase === "connected"}
        connection={connection}
        connections={connections}
        activeConnectionId={activeConnectionId}
        database={database}
        databases={databases}
        onDatabaseChange={selectDatabase}
        onConnectionSelect={(item) => void activateConnection(item)}
        onConnectionAdd={() => setConnectionDialog({ open: true })}
        onConnectionEdit={(item) => setConnectionDialog({ open: true, connectionId: item.id })}
        onConnectionRemove={removeConnection}
      />

      <aside className="station-rail" aria-label="Station navigation">
        <div className="rail-primary">
          <NavButton
            active={view === "overview"}
            icon={<Gauge size={18} />}
            label="Overview"
            onClick={() => setView("overview")}
          />
          <NavButton
            active={view === "query"}
            icon={<TerminalSquare size={18} />}
            label="Query"
            onClick={() => setView("query")}
          />
          <NavButton
            active={view === "access"}
            icon={<ShieldCheck size={18} />}
            label="Access"
            onClick={() => setView("access")}
          />
        </div>
      </aside>

      <main className="station-main">
        {view === "access" ? (
          <StationAccess
            snapshot={accessSnapshot}
            loading={accessLoading}
            error={accessError}
            onRefresh={() => void refreshAccess()}
            onCreateUser={handleCreateManagedUser}
            onUpdateUser={handleUpdateManagedUser}
            onUpdatePassword={handleUpdateManagedUserPassword}
            onDropUser={handleDropManagedUser}
            onCreateApiKey={handleCreateManagedApiKey}
            onDropApiKey={handleDropManagedApiKey}
          />
        ) : database ? (
          view === "overview" ? (
            <Overview
              connection={connection}
              database={database}
              snapshot={snapshot}
              loading={snapshotLoading}
              error={snapshotError}
              onRefresh={() => void refreshSnapshot()}
              onOpenQuery={(sql) => {
                if (sql) addQuery(sql);
                setView("query");
              }}
              onOpenCollection={openCollection}
            />
          ) : (
            <QueryWorkspace
              tabs={queryTabs}
              activeId={activeQueryId}
              onActiveChange={setActiveQueryId}
              onSqlChange={(sql) => updateActiveQuery({ sql, error: null })}
              onAdd={() => addQuery()}
              onClose={closeQuery}
              database={database}
              running={queryRunning}
              onRun={(sqlOverride) => void runQuery(sqlOverride)}
              onCancel={() => queryController.current?.abort()}
              collections={snapshot?.collections ?? []}
            />
          )
        ) : (
          <EmptyDatabaseState onSettings={() => setConnectionDialog({ open: true, connectionId: activeConnectionId })} />
        )}
      </main>

      {(phase === "disconnected" || connectionDialog.open) && (
        <ConnectionPanel
          key={connectionPanelId ?? "new"}
          initial={connections.find((item) => item.id === connectionPanelId) ?? defaultConnection(
            import.meta.env.DEV ? __STELLARDB_DEV_PROXY_TARGET__ : window.location.origin,
          )}
          required={phase === "disconnected"}
          initialError={connectionError}
          onClose={() => setConnectionDialog({ open: false })}
          onConnect={handleConnect}
        />
      )}
    </div>
  );
}

function StationBoot({ endpoint }: { endpoint: string }) {
  return (
    <div className="station-boot">
      <LoaderCircle size={28} className="spin" aria-hidden="true" />
      <div>
        <h1>Connecting</h1>
        <p>{endpoint}</p>
      </div>
    </div>
  );
}

interface HeaderProps {
  connected: boolean;
  connection: StationConnectionRecord;
  connections: StationConnectionRecord[];
  activeConnectionId: string;
  database: string;
  databases: string[];
  onDatabaseChange: (database: string) => void;
  onConnectionSelect: (connection: StationConnectionRecord) => void;
  onConnectionAdd: () => void;
  onConnectionEdit: (connection: StationConnectionRecord) => void;
  onConnectionRemove: (connection: StationConnectionRecord) => void;
}

function StationHeader({
  connected,
  connection,
  connections,
  activeConnectionId,
  database,
  databases,
  onDatabaseChange,
  onConnectionSelect,
  onConnectionAdd,
  onConnectionEdit,
  onConnectionRemove,
}: HeaderProps) {
  return (
    <header className="station-header">
      <div className="station-brand">
        <div className="brand-beacon" aria-hidden="true"><span /></div>
        <div>
          <strong>StellarDB</strong>
          <span>Station</span>
        </div>
      </div>
      <div className="station-context">
        <span className={`link-state ${connected ? "is-online" : ""}`}>
          <Radio size={13} />
          {connected ? "LINKED" : "OFFLINE"}
        </span>
        <span className="context-endpoint">{shortEndpoint(connection.displayUrl ?? connection.baseUrl)}</span>
        <span className="context-divider" />
        <ConnectionMenu
          connections={connections}
          activeId={activeConnectionId}
          onSelect={onConnectionSelect}
          onAdd={onConnectionAdd}
          onEdit={onConnectionEdit}
          onRemove={onConnectionRemove}
        />
        <DatabaseMenu databases={databases} value={database} onChange={onDatabaseChange} />
      </div>
    </header>
  );
}

function NavButton({
  active,
  icon,
  label,
  onClick,
}: {
  active: boolean;
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      className={`nav-button ${active ? "is-active" : ""}`}
      onClick={onClick}
      aria-current={active ? "page" : undefined}
    >
      {icon}
      <span>{label}</span>
    </button>
  );
}

interface OverviewProps {
  connection: StationConnectionRecord;
  database: string;
  snapshot: StationSnapshot | null;
  loading: boolean;
  error: string | null;
  onRefresh: () => void;
  onOpenQuery: (sql?: string) => void;
  onOpenCollection: (collection: CollectionOverview) => void;
}

function Overview({
  connection,
  database,
  snapshot,
  loading,
  error,
  onRefresh,
  onOpenQuery,
  onOpenCollection,
}: OverviewProps) {
  const stats = snapshot?.stats;
  const collections = snapshot?.collections ?? [];
  const [patternSearch, setPatternSearch] = useState("");
  const [patternSort, setPatternSort] = useState<"total" | "calls" | "latency" | "errors">("total");
  const [selectedPatternHash, setSelectedPatternHash] = useState<string | null>(null);
  const documents = collections.reduce(
    (total, collection) => total + (collection.stats?.document_count ?? 0),
    0,
  );
  const patterns = useMemo(() => {
    const search = patternSearch.trim().toLowerCase();
    const filtered = (stats?.top_queries ?? []).filter((query) => {
      if (!search) return true;
      return [queryPatternDisplay(query), queryPatternKind(query), query.risk ?? ""]
        .some((value) => value.toLowerCase().includes(search));
    });
    return [...filtered].sort((left, right) => {
      switch (patternSort) {
        case "calls": return right.call_count - left.call_count;
        case "latency": return right.avg_time_us - left.avg_time_us;
        case "errors": return right.errors - left.errors;
        default: return right.total_time_us - left.total_time_us;
      }
    });
  }, [patternSearch, patternSort, stats?.top_queries]);
  const selectedPattern = (stats?.top_queries ?? []).find(
    (query) => query.query_hash === selectedPatternHash,
  ) ?? null;

  return (
    <div className="view overview-view">
      <section className="overview-hero">
        <div className="hero-copy">
          <h1>{database}</h1>
          <p className="hero-description">
            <strong>{shortEndpoint(connection.displayUrl ?? connection.baseUrl)}</strong>
            {" · "}
            <strong>v{snapshot?.capabilities.version ?? "—"}</strong>
            {" · "}
            <strong>{collections.length} collections</strong>
          </p>
          <div className="hero-actions">
            <button className="primary-button" onClick={() => onOpenQuery()}>
              <TerminalSquare size={16} /> Open query
              <ArrowUpRight size={15} />
            </button>
            <button className="secondary-button" onClick={onRefresh} disabled={loading}>
              <RefreshCw size={15} className={loading ? "spin" : ""} /> Refresh
            </button>
          </div>
        </div>
      </section>

      {error && (
        <div className="inline-alert" role="alert">
          <CircleAlert size={17} />
          <span>{error}</span>
          <button onClick={onRefresh}>Retry</button>
        </div>
      )}

      {snapshot && !stats ? (
        <p className="metrics-unavailable">
          Server metrics and query patterns require MANAGE permission on <strong>{database}</strong>.
        </p>
      ) : (
      <section className="metric-grid" aria-label="Database metrics">
        <MetricCard
          label="Documents"
          value={documents.toLocaleString()}
          detail={`Across ${collections.length} collections`}
          icon={<Boxes size={18} />}
          accent="violet"
          loading={loading && !snapshot}
        />
        <MetricCard
          label="Queries"
          value={(stats?.server.queries_per_sec ?? 0).toFixed(1)}
          suffix="/ sec"
          detail={`${(stats?.server.queries_total ?? 0).toLocaleString()} total`}
          icon={<Zap size={18} />}
          accent="lime"
          loading={loading && !snapshot}
        />
        <MetricCard
          label="Storage"
          value={formatBytes(stats?.storage.disk_size_bytes ?? 0)}
          detail={`${stats?.storage.btree_indexes ?? 0} B-tree · ${stats?.storage.vector_indexes ?? 0} vector · ${stats?.storage.fts_indexes ?? 0} FTS`}
          icon={<HardDrive size={18} />}
          accent="cyan"
          loading={loading && !snapshot}
        />
        <MetricCard
          label="Uptime"
          value={formatDuration(stats?.server.uptime_secs ?? 0)}
          detail={`${stats?.server.queries_active ?? 0} active queries`}
          icon={<Activity size={18} />}
          accent="amber"
          loading={loading && !snapshot}
        />
      </section>
      )}

      <div className="overview-grid">
        <section className="station-panel collections-panel">
          <PanelHeader title="Collections" action={`${documents.toLocaleString()} documents`} />
          <div className="collection-list">
            {collections.length === 0 && !loading ? (
              <div className="panel-empty">
                <Boxes size={20} />
                <span>No collections in this database yet.</span>
              </div>
            ) : (
              collections.map((collection, index) => (
                <button
                  key={collection.name}
                  className="collection-row"
                  onClick={() => onOpenCollection(collection)}
                >
                  <span className="collection-index">{String(index + 1).padStart(2, "0")}</span>
                  <span className="collection-icon"><Table2 size={15} /></span>
                  <span className="collection-name">
                    <strong>{collection.name}</strong>
                    <small>{collection.schema?.fields.length ?? 0} fields · {collection.mode.toLowerCase()}</small>
                  </span>
                  <span className="collection-count">
                    {(collection.stats?.document_count ?? 0).toLocaleString()}
                  </span>
                  <ArrowUpRight size={14} />
                </button>
              ))
            )}
          </div>
        </section>

        {(!snapshot || stats) && (
        <section className="station-panel activity-panel pattern-explorer">
          <PanelHeader title="Query patterns" action={`${patterns.length} visible`} />
          <div className="pattern-toolbar">
            <label className="pattern-search">
              <Search size={13} />
              <input
                value={patternSearch}
                onChange={(event) => setPatternSearch(event.target.value)}
                placeholder="Search template, kind or risk"
                aria-label="Search query patterns"
              />
            </label>
            <div className="pattern-sort" aria-label="Sort query patterns">
              {([
                ["total", "Total"],
                ["calls", "Calls"],
                ["latency", "Latency"],
                ["errors", "Errors"],
              ] as const).map(([value, label]) => (
                <button
                  key={value}
                  className={patternSort === value ? "is-active" : ""}
                  onClick={() => setPatternSort(value)}
                  aria-pressed={patternSort === value}
                >
                  {label}
                </button>
              ))}
            </div>
          </div>
          <div className="query-log">
            {patterns.map((query, index) => {
              const template = queryPatternTemplate(query);
              const kind = queryPatternKind(query);
              const risk = query.risk ?? "unknown";
              return (
                <button
                  className={`query-log-row ${selectedPatternHash === query.query_hash ? "is-active" : ""}`}
                  key={query.query_hash}
                  onClick={() => setSelectedPatternHash(
                    selectedPatternHash === query.query_hash ? null : query.query_hash,
                  )}
                  aria-expanded={selectedPatternHash === query.query_hash}
                  title={template ? "Inspect safe query template" : "The original query could not be parsed safely"}
                >
                  <span className="log-rank">{index + 1}</span>
                  <div>
                    <span className="pattern-row-heading">
                      <span className={`pattern-risk risk-${risk}`}>{kind}</span>
                      <code>{queryPatternDisplay(query)}</code>
                    </span>
                    <span>
                      {query.call_count.toLocaleString()} calls · {(query.rows_returned + (query.rows_affected ?? 0)).toLocaleString()} rows
                      {query.errors > 0 && ` · ${query.errors.toLocaleString()} errors`}
                      {` · ${formatRelativeTime(query.last_seen)}`}
                    </span>
                  </div>
                  <span className="query-latency">{(query.avg_time_us / 1_000).toFixed(2)} ms</span>
                  <ArrowUpRight className="query-replay-icon" size={13} />
                </button>
              );
            })}
            {patterns.length === 0 && (
              <div className="panel-empty compact">
                <Radio size={19} />
                <span>{(stats?.top_queries.length ?? 0) === 0 ? "Query activity will appear here." : "No patterns match this filter."}</span>
              </div>
            )}
          </div>

          {selectedPattern && (
            <div className="pattern-inspector">
              <div className="pattern-inspector-heading">
                <div>
                  <span className={`pattern-risk risk-${selectedPattern.risk ?? "unknown"}`}>
                    {queryPatternKind(selectedPattern)} · {(selectedPattern.risk ?? "unknown").toUpperCase()}
                  </span>
                  <strong>Safe query template</strong>
                </div>
                <button
                  className="icon-button pattern-close"
                  onClick={() => setSelectedPatternHash(null)}
                  aria-label="Close pattern details"
                >
                  <X size={14} />
                </button>
              </div>
              <pre className="pattern-code"><code>{queryPatternDisplay(selectedPattern)}</code></pre>
              <div className="pattern-facts">
                <span><EyeOff size={13} /> {selectedPattern.redaction_count ?? 0} values hidden</span>
                <span><ShieldCheck size={13} /> sanitizer v{selectedPattern.sanitizer_version ?? 1}</span>
                <span>#{selectedPattern.query_hash.slice(0, 8)}</span>
                <span>max {(selectedPattern.max_time_us / 1_000).toFixed(2)} ms</span>
              </div>
              {(selectedPattern.query_variants?.length ?? 0) > 1 && (
                <div className="pattern-variants">
                  <span>Safe variants</span>
                  {selectedPattern.query_variants!.map((variant, index) => (
                    <button key={`${selectedPattern.query_hash}-${index}`} onClick={() => onOpenQuery(variant)}>
                      <code>{variant}</code>
                      <ArrowUpRight size={12} />
                    </button>
                  ))}
                </div>
              )}
              <div className="pattern-actions">
                <p>
                  Hidden values are never stored. The template opens in the editor and is not executed automatically.
                </p>
                <button
                  className="primary-button"
                  disabled={!queryPatternTemplate(selectedPattern)}
                  onClick={() => onOpenQuery(queryPatternTemplate(selectedPattern))}
                >
                  <Code2 size={15} /> Open safe template
                </button>
              </div>
            </div>
          )}
        </section>
        )}
      </div>
    </div>
  );
}

function MetricCard({
  label,
  value,
  suffix,
  detail,
  icon,
  accent,
  loading,
}: {
  label: string;
  value: string;
  suffix?: string;
  detail: string;
  icon: React.ReactNode;
  accent: string;
  loading: boolean;
}) {
  return (
    <article className={`metric-card accent-${accent}`}>
      <div className="metric-top"><span>{label}</span><i>{icon}</i></div>
      {loading ? (
        <div className="metric-skeleton" />
      ) : (
        <div className="metric-value">{value}<small>{suffix}</small></div>
      )}
      <p>{detail}</p>
    </article>
  );
}

function PanelHeader({ title, action }: { title: string; action: string }) {
  return (
    <header className="panel-header">
      <h2>{title}</h2>
      <small>{action}</small>
    </header>
  );
}

interface QueryWorkspaceProps {
  tabs: QueryTab[];
  activeId: string;
  onActiveChange: (id: string) => void;
  onSqlChange: (sql: string) => void;
  onAdd: () => void;
  onClose: (id: string) => void;
  database: string;
  running: boolean;
  onRun: (sqlOverride?: string) => void;
  onCancel: () => void;
  collections: CollectionOverview[];
}

function QueryWorkspace({
  tabs,
  activeId,
  onActiveChange,
  onSqlChange,
  onAdd,
  onClose,
  database,
  running,
  onRun,
  onCancel,
  collections,
}: QueryWorkspaceProps) {
  const active = tabs.find((tab) => tab.id === activeId) ?? tabs[0]!;
  const [schemaCollapsed, setSchemaCollapsed] = useState(true);
  const [cursor, setCursor] = useState({ line: 1, column: 1 });
  const editorRef = useRef<StationEditorHandle>(null);
  const schemas = useMemo(
    () => new Map(collections.flatMap((collection) => collection.schema ? [[collection.name, collection.schema] as const] : [])),
    [collections],
  );

  return (
    <div className="view query-view">
      <div className="query-titlebar">
        <h1>Query</h1>
        <div className="query-title-actions">
          <span><Database size={13} /> {database}</span>
          <button className="run-button" onClick={() => onRun()} disabled={!active.sql.trim()}>
            {running ? <Square size={14} fill="currentColor" /> : <Play size={15} fill="currentColor" />}
            {running ? "Cancel" : "Run query"}
            <kbd>⌘ ↵</kbd>
          </button>
        </div>
      </div>

      <div className={`query-layout station-query-layout ${schemaCollapsed ? "schema-collapsed" : ""}`}>
        <aside className="query-tab-rail" aria-label="Query tabs">
          <header><span>Queries</span><button onClick={onAdd} aria-label="New query"><Plus size={13} /></button></header>
          <div role="tablist" aria-orientation="vertical">
            {tabs.map((tab) => (
              <button
                key={tab.id}
                className={`vertical-query-tab ${tab.id === activeId ? "is-active" : ""}`}
                onClick={() => onActiveChange(tab.id)}
                role="tab"
                aria-selected={tab.id === activeId}
                aria-label={`${tab.name}. ${tab.error ? "Last run failed" : tab.result ? "Last run succeeded" : "Not run yet"}`}
                title={tab.error ? "Last run failed" : tab.result ? "Last run succeeded" : "Not run yet"}
              >
                <Code2 size={13} />
                <span><strong>{tab.name}</strong><small>{tab.sql.trim().split("\n")[0] || "Empty query"}</small></span>
                {tabs.length > 1 && <i role="button" aria-label={`Close ${tab.name}`} onClick={(event) => { event.stopPropagation(); onClose(tab.id); }}><X size={12} /></i>}
                {tab.error && <em aria-hidden="true" />}
                {!tab.error && tab.result && <b aria-hidden="true" />}
              </button>
            ))}
          </div>
          <button className="new-query-vertical" onClick={onAdd}><Plus size={13} /> New query</button>
        </aside>
        <section className="editor-stack">
          <div className="editor-tabs">
            <span className="active-editor-label"><Code2 size={13} /> {active.name}</span>
            <span className="editor-language">STELLAR SQL</span>
          </div>
          <div className="station-monaco" aria-label="SQL query editor">
            <StationEditor
              ref={editorRef}
              value={active.sql}
              error={active.error}
              schemas={schemas}
              onChange={onSqlChange}
              onExecute={onRun}
              onCursorChange={(line, column) => setCursor({ line, column })}
            />
          </div>
          <div className="editor-statusbar">
            <span className={active.error ? "is-error" : undefined}>
              {active.error ? <CircleAlert size={12} /> : <Check size={12} />}
              {active.error ? "Query error" : "Ready"}
            </span>
            <span>Ln {cursor.line}, Col {cursor.column}</span>
          </div>
        </section>
        <StationSchema
          collections={collections}
          collapsed={schemaCollapsed}
          onCollapsedChange={setSchemaCollapsed}
          onInsert={(text) => editorRef.current?.insertAtCursor(text)}
        />
      </div>

      <StationResults
        result={active.result}
        error={active.error}
        running={running}
        canRun={Boolean(active.sql.trim())}
        onRun={() => onRun()}
        onCancel={onCancel}
      />
    </div>
  );
}

function EmptyDatabaseState({ onSettings }: { onSettings: () => void }) {
  return (
    <div className="empty-database">
      <Database size={26} />
      <h1>No databases</h1>
      <p>Create a database on the server, then reconnect.</p>
      <button className="primary-button" onClick={onSettings}><RotateCcw size={15} /> Reconnect</button>
    </div>
  );
}

interface ConnectionPanelProps {
  initial: StationConnectionRecord;
  required: boolean;
  initialError: string | null;
  onClose: () => void;
  onConnect: (
    connection: StationConnection,
    details: { name: string; displayUrl: string },
  ) => Promise<void>;
}

function ConnectionPanel({ initial, required, initialError, onClose, onConnect }: ConnectionPanelProps) {
  const [name, setName] = useState(initial.name);
  const [baseUrl, setBaseUrl] = useState(initial.displayUrl ?? initial.baseUrl);
  const [authMode, setAuthMode] = useState<ConnectionAuthMode>(
    initial.authMode ?? (initial.token ? "apiKey" : "anonymous"),
  );
  const [username, setUsername] = useState(initial.username ?? "");
  const [password, setPassword] = useState("");
  const [token, setToken] = useState("");
  const [error, setError] = useState<string | null>(initialError);
  const [submitting, setSubmitting] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setSubmitting(true);
    setError(null);
    try {
      const normalized = normalizeBaseUrl(baseUrl);
      const requestBaseUrl = routeThroughDevProxy(normalized);
      if (authMode === "anonymous") {
        await onConnect({ baseUrl: requestBaseUrl, authMode }, { name: name.trim(), displayUrl: normalized });
      } else if (authMode === "credentials") {
        const sessionToken = await loginWithCredentials(requestBaseUrl, {
          username: username.trim(),
          password,
        });
        await onConnect({
          baseUrl: requestBaseUrl,
          token: sessionToken,
          authMode,
          username: username.trim(),
          password,
        }, { name: name.trim(), displayUrl: normalized });
      } else {
        await onConnect({
          baseUrl: requestBaseUrl,
          token: token.trim(),
          authMode,
        }, { name: name.trim(), displayUrl: normalized });
      }
    } catch (nextError) {
      setError(readableError(nextError));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="connection-overlay" role="presentation">
      <div className="connection-panel" role="dialog" aria-modal="true" aria-labelledby="connection-title">
        {!required && (
          <button className="panel-close" onClick={onClose} aria-label="Close connection settings"><X size={17} /></button>
        )}
        <div className="connection-copy">
          <h2 id="connection-title">Connect to StellarDB</h2>
          <p>Local server or an HTTPS remote endpoint.</p>
        </div>
        <form onSubmit={submit}>
          <label>
            <span>Connection name</span>
            <div className="field-wrap"><Database size={15} /><input value={name} onChange={(event) => setName(event.target.value)} placeholder="Local StellarDB" required /></div>
          </label>
          <label>
            <span>Server endpoint</span>
            <div className="field-wrap"><Server size={15} /><input value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} placeholder="http://localhost:3000" required /></div>
          </label>
          <fieldset className="auth-mode-fieldset">
            <legend>Authentication</legend>
            <div className="auth-mode-selector">
              <button
                type="button"
                className={authMode === "anonymous" ? "is-active" : ""}
                onClick={() => setAuthMode("anonymous")}
              >
                <UserRound size={14} /> Anonymous
              </button>
              <button
                type="button"
                className={authMode === "credentials" ? "is-active" : ""}
                onClick={() => setAuthMode("credentials")}
              >
                <LockKeyhole size={14} /> Login
              </button>
              <button
                type="button"
                className={authMode === "apiKey" ? "is-active" : ""}
                onClick={() => setAuthMode("apiKey")}
              >
                <KeyRound size={14} /> API key
              </button>
            </div>
          </fieldset>
          {authMode === "anonymous" && (
            <p className="auth-mode-hint">Works only when the server is started with <code>--anonymous-user</code>.</p>
          )}
          {authMode === "credentials" && (
            <div className="credential-fields">
              <label>
                <span>Username</span>
                <div className="field-wrap"><UserRound size={15} /><input value={username} onChange={(event) => setUsername(event.target.value)} placeholder="root" autoComplete="username" required /></div>
              </label>
              <label>
                <span>Password</span>
                <div className="field-wrap"><LockKeyhole size={15} /><input type="password" value={password} onChange={(event) => setPassword(event.target.value)} placeholder="••••••••••••" autoComplete="current-password" required /></div>
              </label>
            </div>
          )}
          {authMode === "apiKey" && (
            <label>
              <span>API key</span>
              <div className="field-wrap"><KeyRound size={15} /><input type="password" value={token} onChange={(event) => setToken(event.target.value)} placeholder="stl_…" autoComplete="off" required /></div>
            </label>
          )}
          {error && <div className="connection-error"><CircleAlert size={15} /><span>{error}</span></div>}
          <button className="connect-button" disabled={submitting}>
            {submitting ? <LoaderCircle size={16} className="spin" /> : <Server size={16} />}
            {submitting ? "Connecting…" : "Connect"}
          </button>
        </form>
        <p className="connection-footnote">Credentials stay in memory and are never written to browser storage.</p>
      </div>
    </div>
  );
}
