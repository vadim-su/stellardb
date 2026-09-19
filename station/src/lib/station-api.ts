import {
  StellarClient,
  StellarDbError,
  type CapabilitiesResponse,
  type CollectionOverview,
  type DbRow,
  type TypedSqlResponse,
} from "@stellardb/client";

export interface StationConnection {
  baseUrl: string;
  token?: string;
  authMode?: "anonymous" | "credentials" | "apiKey";
  username?: string;
  password?: string;
}

export interface LoginCredentials {
  username: string;
  password: string;
}

export interface ServerStats {
  uptime_secs: number;
  queries_total: number;
  queries_per_sec: number;
  queries_errors: number;
  queries_active: number;
}

export interface StorageStats {
  disk_size_bytes: number;
  collections_count: number;
  btree_indexes: number;
  vector_indexes: number;
  fts_indexes: number;
}

export interface QueryStat {
  query_hash: string;
  query_normalized: string;
  query_example: string;
  query_display?: string;
  query_variants?: string[];
  statement_kind?: string;
  risk?: "read" | "session" | "transaction" | "write" | "schema" | "admin" | "unknown";
  redaction_count?: number;
  sanitizer_version?: number;
  call_count: number;
  total_time_us: number;
  avg_time_us: number;
  min_time_us: number;
  max_time_us: number;
  rows_returned: number;
  rows_affected?: number;
  errors: number;
  first_seen?: string;
  last_seen?: string;
}

export interface StatsResponse {
  server: ServerStats;
  storage: StorageStats;
  top_queries: QueryStat[];
}

interface DatabaseStatsResponse {
  collections: number;
  btree_indexes: number;
  vector_indexes: number;
  fts_indexes: number;
  disk_size: number;
}

export interface StationExample {
  title: string;
  description?: string;
  sql: string;
}

export interface StationSnapshot {
  capabilities: CapabilitiesResponse;
  collections: CollectionOverview[];
  /** `null` when the subject lacks MANAGE on the database. */
  stats: StatsResponse | null;
  /** Operator-curated queries for this database (`--station-examples`). */
  examples: StationExample[];
}

export interface StationQueryResult {
  columns: string[];
  rows: Record<string, unknown>[];
  rowCount: number;
  elapsedMs: number;
  statementType: string;
  allResults: Array<{
    data: Record<string, unknown>[];
    statementType: string;
  }>;
  response: TypedSqlResponse;
}

export interface IdentityInfo {
  username: string;
  attributes: Record<string, string>;
  authMethod: "anonymous" | "credentials" | "apiKey";
  credentialName: string | null;
}

export interface ManagedUser {
  username: string;
  attributes: Record<string, string>;
  apiKeyCount: number;
  protected: boolean;
  createdAt: string | null;
  updatedAt: string | null;
}

export interface ManagedApiKey {
  name: string;
  username: string;
  hint: string | null;
  createdAt: string | null;
}

export interface AccessSnapshot {
  identity: IdentityInfo;
  /** `false` when the subject lacks global MANAGE; lists are then empty. */
  admin: boolean;
  users: ManagedUser[];
  apiKeys: ManagedApiKey[];
}

export interface CreatedApiKey {
  key: ManagedApiKey;
  secret: string;
}

export function normalizeBaseUrl(value: string): string {
  const url = new URL(value.trim());
  if (
    url.protocol === "http:" &&
    !["localhost", "127.0.0.1", "::1"].includes(url.hostname)
  ) {
    throw new Error("Remote Station connections require HTTPS.");
  }
  return url.toString().replace(/\/$/, "");
}

function headers(connection: StationConnection, database?: string): Headers {
  const result = new Headers();
  if (connection.token) {
    result.set("Authorization", `Bearer ${connection.token}`);
  }
  if (database) result.set("X-Database", database);
  return result;
}

async function responseError(response: Response): Promise<Error> {
  const payload: unknown = await response.json().catch(() => null);
  if (
    payload &&
    typeof payload === "object" &&
    "error" in payload &&
    payload.error &&
    typeof payload.error === "object" &&
    "message" in payload.error &&
    typeof payload.error.message === "string"
  ) {
    return new Error(payload.error.message);
  }
  if (
    payload &&
    typeof payload === "object" &&
    "error" in payload &&
    typeof payload.error === "string"
  ) {
    return new Error(payload.error);
  }
  return new Error(`StellarDB returned HTTP ${response.status}.`);
}

class ForbiddenError extends Error {
  constructor(cause: Error) {
    super(cause.message);
    this.name = "ForbiddenError";
  }
}

async function adminRequest(
  connection: StationConnection,
  path: string,
  init: RequestInit = {},
): Promise<Response> {
  const requestHeaders = headers(connection);
  if (init.body !== undefined) requestHeaders.set("Content-Type", "application/json");
  const response = await fetch(`${connection.baseUrl}${path}`, {
    ...init,
    headers: requestHeaders,
  });
  if (response.status === 403) throw new ForbiddenError(await responseError(response));
  if (!response.ok) throw await responseError(response);
  return response;
}

async function loadAdminPages<T>(
  connection: StationConnection,
  path: string,
  signal?: AbortSignal,
): Promise<T[]> {
  const items: T[] = [];
  let after: string | null = null;
  do {
    const separator = path.includes("?") ? "&" : "?";
    const cursor = after ? `&after=${encodeURIComponent(after)}` : "";
    const response = await adminRequest(
      connection,
      `${path}${separator}limit=100${cursor}`,
      { signal },
    );
    const page = await response.json() as { items: T[]; nextCursor: string | null };
    items.push(...page.items);
    after = page.nextCursor;
  } while (after);
  return items;
}

export async function loadAccessSnapshot(
  connection: StationConnection,
  signal?: AbortSignal,
): Promise<AccessSnapshot> {
  const [identityResponse, admin] = await Promise.all([
    adminRequest(connection, "/v1/auth/me", { signal }),
    Promise.all([
      loadAdminPages<ManagedUser>(connection, "/v1/admin/users", signal),
      loadAdminPages<ManagedApiKey>(connection, "/v1/admin/api-keys", signal),
    ]).then(
      ([users, apiKeys]) => ({ users, apiKeys }),
      (error: unknown) => {
        if (error instanceof ForbiddenError) return null;
        throw error;
      },
    ),
  ]);
  const identity = await identityResponse.json() as IdentityInfo;
  return {
    identity,
    admin: admin !== null,
    users: admin?.users ?? [],
    apiKeys: admin?.apiKeys ?? [],
  };
}

export async function createManagedUser(
  connection: StationConnection,
  request: { username: string; password: string; attributes: Record<string, string> },
): Promise<ManagedUser> {
  const response = await adminRequest(connection, "/v1/admin/users", {
    method: "POST",
    body: JSON.stringify(request),
  });
  const payload = await response.json() as { user: ManagedUser };
  return payload.user;
}

export async function updateManagedUser(
  connection: StationConnection,
  username: string,
  request: { set: Record<string, string>; remove: string[] },
): Promise<ManagedUser> {
  const response = await adminRequest(
    connection,
    `/v1/admin/users/${encodeURIComponent(username)}`,
    { method: "PATCH", body: JSON.stringify(request) },
  );
  const payload = await response.json() as { user: ManagedUser };
  return payload.user;
}

export async function updateManagedUserPassword(
  connection: StationConnection,
  username: string,
  password: string,
): Promise<ManagedUser> {
  const response = await adminRequest(
    connection,
    `/v1/admin/users/${encodeURIComponent(username)}/password`,
    { method: "PUT", body: JSON.stringify({ password }) },
  );
  const payload = await response.json() as { user: ManagedUser };
  return payload.user;
}

export async function dropManagedUser(
  connection: StationConnection,
  username: string,
): Promise<void> {
  await adminRequest(connection, `/v1/admin/users/${encodeURIComponent(username)}`, {
    method: "DELETE",
  });
}

export async function createManagedApiKey(
  connection: StationConnection,
  request: { name: string; username: string },
): Promise<CreatedApiKey> {
  const response = await adminRequest(connection, "/v1/admin/api-keys", {
    method: "POST",
    body: JSON.stringify(request),
  });
  return response.json() as Promise<CreatedApiKey>;
}

export async function dropManagedApiKey(
  connection: StationConnection,
  name: string,
): Promise<void> {
  await adminRequest(connection, `/v1/admin/api-keys/${encodeURIComponent(name)}`, {
    method: "DELETE",
  });
}

export async function loginWithCredentials(
  baseUrl: string,
  credentials: LoginCredentials,
  signal?: AbortSignal,
): Promise<string> {
  const response = await fetch(`${baseUrl}/auth/login`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(credentials),
    signal,
  });
  if (!response.ok) throw await responseError(response);
  const payload: unknown = await response.json().catch(() => null);
  if (
    !payload ||
    typeof payload !== "object" ||
    !("token" in payload) ||
    typeof payload.token !== "string" ||
    payload.token.length === 0
  ) {
    throw new Error("StellarDB returned an invalid login response.");
  }
  return payload.token;
}

export async function listDatabases(
  connection: StationConnection,
  signal?: AbortSignal,
): Promise<string[]> {
  const response = await fetch(`${connection.baseUrl}/databases`, {
    headers: headers(connection),
    signal,
  });
  if (!response.ok) throw await responseError(response);
  const payload: unknown = await response.json();
  if (!Array.isArray(payload) || !payload.every((item) => typeof item === "string")) {
    throw new Error("StellarDB returned an invalid database list.");
  }
  return payload;
}

async function fetchStats(
  connection: StationConnection,
  database: string,
  signal?: AbortSignal,
): Promise<StatsResponse | null> {
  const response = await fetch(`${connection.baseUrl}/stats`, {
    headers: headers(connection, database),
    signal,
  });
  if (response.status === 403) return null;
  if (!response.ok) throw await responseError(response);
  return response.json() as Promise<StatsResponse>;
}

async function fetchDatabaseStats(
  connection: StationConnection,
  database: string,
  signal?: AbortSignal,
): Promise<DatabaseStatsResponse | null> {
  const response = await fetch(`${connection.baseUrl}/databases/${encodeURIComponent(database)}/stats`, {
    headers: headers(connection),
    signal,
  });
  if (response.status === 403) return null;
  if (!response.ok) throw await responseError(response);
  return response.json() as Promise<DatabaseStatsResponse>;
}

async function fetchExamples(
  connection: StationConnection,
  database: string,
  signal?: AbortSignal,
): Promise<StationExample[]> {
  const response = await fetch(`${connection.baseUrl}/v1/station/examples`, {
    headers: headers(connection, database),
    signal,
  });
  // Older servers have no endpoint; forbidden subjects get nothing rather than an error.
  if (response.status === 403 || response.status === 404) return [];
  if (!response.ok) throw await responseError(response);
  const payload = await response.json() as { items: StationExample[] };
  return payload.items;
}

export async function loadSnapshot(
  connection: StationConnection,
  database: string,
  signal?: AbortSignal,
): Promise<StationSnapshot> {
  const client = new StellarClient({
    baseUrl: connection.baseUrl,
    database,
    token: connection.token,
  });
  const [capabilities, collections, stats, databaseStats, examples] = await Promise.all([
    client.capabilities(signal),
    client.collections(["stats", "schema"], signal),
    fetchStats(connection, database, signal),
    fetchDatabaseStats(connection, database, signal),
    fetchExamples(connection, database, signal),
  ]);
  return {
    capabilities,
    collections: collections.collections,
    stats: stats && databaseStats ? {
      ...stats,
      storage: {
        disk_size_bytes: databaseStats.disk_size,
        collections_count: databaseStats.collections,
        btree_indexes: databaseStats.btree_indexes,
        vector_indexes: databaseStats.vector_indexes,
        fts_indexes: databaseStats.fts_indexes,
      },
    } : null,
    examples,
  };
}

function normalizeRows(rows: DbRow[] | undefined): Record<string, unknown>[] {
  return (rows ?? []).map((row) => {
    if (
      row !== null &&
      typeof row === "object" &&
      !Array.isArray(row) &&
      !("$type" in row)
    ) {
      return row as Record<string, unknown>;
    }
    return { value: row };
  });
}

export async function executeStationQuery(
  connection: StationConnection,
  database: string,
  sql: string,
  signal?: AbortSignal,
): Promise<StationQueryResult> {
  const client = new StellarClient({
    baseUrl: connection.baseUrl,
    database,
    token: connection.token,
  });
  try {
    const response = await client.query(sql, { signal });
    const statement = response.results.at(-1);
    const rows = normalizeRows(statement?.data);
    const keys = rows.length > 0 ? Object.keys(rows[0] ?? {}) : [];
    const columns = keys.includes("id")
      ? ["id", ...keys.filter((key) => key !== "id")]
      : keys;
    return {
      columns,
      rows,
      rowCount: statement?.count ?? statement?.rows_affected ?? rows.length,
      elapsedMs: response.elapsed_us / 1_000,
      statementType: statement?.statement_type ?? "QUERY",
      allResults: response.results.map((result) => ({
        data: normalizeRows(result.data),
        statementType: result.statement_type,
      })),
      response,
    };
  } catch (error) {
    if (error instanceof StellarDbError && error.hint) {
      throw new Error(`${error.message} ${error.hint}`);
    }
    throw error;
  }
}

export function readableError(error: unknown): string {
  if (error instanceof DOMException && error.name === "AbortError") {
    return "Request cancelled.";
  }
  if (error instanceof TypeError && /fetch/i.test(error.message)) {
    return "Could not reach StellarDB. Check that the server is running and the endpoint is correct.";
  }
  if (error instanceof Error) return error.message;
  return "Station could not complete the request.";
}

export function isAuthenticationError(error: unknown): boolean {
  if (error instanceof StellarDbError) return error.status === 401;
  return error instanceof Error && /auth|unauthorized|token|authorization header required/i.test(error.message);
}
