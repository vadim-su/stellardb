import { decodeDbValue } from "./db-value.js";
import { errorFromResponse, StellarDbError } from "./errors.js";
import type {
  CapabilitiesResponse,
  CollectionsOverviewResponse,
  FetchLike,
  ServerFeatures,
  StellarClientOptions,
  TypedSqlResponse,
} from "./types.js";

export interface QueryOptions {
  signal?: AbortSignal;
  sessionId?: string;
  timeout?: string;
}

export class StellarClient {
  readonly baseUrl: string;
  readonly database: string;
  readonly token?: string;
  private readonly fetchImpl: FetchLike;

  constructor(options: StellarClientOptions) {
    this.baseUrl = options.baseUrl.replace(/\/+$/, "");
    this.database = options.database;
    this.token = options.token;
    // Browser fetch is a Web API method and must keep the Window/global
    // receiver. Calling a detached fetch with StellarClient as `this` throws
    // `TypeError: Illegal invocation` in Chromium-based browsers.
    this.fetchImpl = options.fetch ?? globalThis.fetch.bind(globalThis);
  }

  with(options: Partial<StellarClientOptions>): StellarClient {
    return new StellarClient({
      baseUrl: options.baseUrl ?? this.baseUrl,
      database: options.database ?? this.database,
      token: options.token ?? this.token,
      fetch: options.fetch ?? this.fetchImpl,
    });
  }

  async capabilities(signal?: AbortSignal): Promise<CapabilitiesResponse> {
    const response = await this.request("/v1/capabilities", {
      method: "GET",
      signal,
    });
    return decodeCapabilitiesResponse(await response.json().catch(() => null));
  }

  async query(
    query: string,
    options: QueryOptions = {},
  ): Promise<TypedSqlResponse> {
    const response = await this.request("/v1/query", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ query }),
      signal: options.signal,
      sessionId: options.sessionId,
      timeout: options.timeout,
    });
    try {
      const data = decodeTypedSqlResponse(
        await response.json().catch(() => null),
      );
      if (data.error) throw new StellarDbError(response.status, data.error);
      return data;
    } catch (error) {
      if (error instanceof StellarDbError) throw error;
      throw new StellarDbError(response.status, "Invalid /v1/query response");
    }
  }

  async collections(
    include: Array<"stats" | "schema"> = [],
    signal?: AbortSignal,
  ): Promise<CollectionsOverviewResponse> {
    const params = new URLSearchParams();
    if (include.length > 0) params.set("include", include.join(","));
    const suffix = params.size > 0 ? `?${params}` : "";
    const response = await this.request(`/v1/collections${suffix}`, {
      method: "GET",
      signal,
    });
    try {
      return decodeCollectionsOverviewResponse(
        await response.json().catch(() => null),
      );
    } catch {
      throw new StellarDbError(
        response.status,
        "Invalid /v1/collections response",
      );
    }
  }

  private async request(
    path: string,
    init: RequestInit & { sessionId?: string; timeout?: string },
  ): Promise<Response> {
    const headers = new Headers(init.headers);
    headers.set("X-Database", this.database);
    if (this.token) headers.set("Authorization", `Bearer ${this.token}`);
    if (init.sessionId) headers.set("X-Session-Id", init.sessionId);
    if (init.timeout) headers.set("X-Query-Timeout", init.timeout);

    const response = await this.fetchImpl(`${this.baseUrl}${path}`, {
      ...init,
      headers,
    });
    if (!response.ok) throw await errorFromResponse(response);
    return response;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasExactKeys(
  value: Record<string, unknown>,
  expected: string[],
): boolean {
  const keys = Object.keys(value).sort();
  return (
    keys.length === expected.length &&
    keys.every((key, i) => key === expected[i])
  );
}

export function decodeCapabilitiesResponse(
  value: unknown,
): CapabilitiesResponse {
  if (
    !isRecord(value) ||
    value.apiVersion !== "v1" ||
    typeof value.version !== "string"
  ) {
    throw new Error("Invalid /v1/capabilities response");
  }
  const featureKeys: Array<keyof ServerFeatures> = [
    "apiKeyManagement",
    "auditLog",
    "explain",
    "explainAnalyze",
    "importJobs",
    "queryCancellation",
    "roleManagement",
    "savedQueries",
    "schemaMutations",
    "temporalMetrics",
    "userManagement",
  ];
  const features = value.features;
  if (
    !isRecord(features) ||
    !hasExactKeys(features, [...featureKeys].sort()) ||
    !featureKeys.every((key) => typeof features[key] === "boolean")
  ) {
    throw new Error("Invalid /v1/capabilities response");
  }
  return value as unknown as CapabilitiesResponse;
}

export function decodeTypedSqlResponse(value: unknown): TypedSqlResponse {
  if (
    !isRecord(value) ||
    !Array.isArray(value.results) ||
    !value.results.every(isTypedSqlResult) ||
    typeof value.completed !== "number" ||
    !Number.isInteger(value.completed) ||
    typeof value.elapsed_us !== "number" ||
    !Number.isFinite(value.elapsed_us) ||
    ("error" in value && !isStructuredError(value.error))
  ) {
    throw new Error("Invalid /v1/query response");
  }
  return value as unknown as TypedSqlResponse;
}

function isTypedSqlResult(value: unknown): boolean {
  if (
    !isRecord(value) ||
    typeof value.statement_index !== "number" ||
    !Number.isInteger(value.statement_index) ||
    typeof value.statement_type !== "string"
  ) {
    return false;
  }
  if ("count" in value && typeof value.count !== "number") return false;
  if ("rows_affected" in value && typeof value.rows_affected !== "number")
    return false;
  if ("session_id" in value && typeof value.session_id !== "string")
    return false;
  if ("data" in value) {
    if (!Array.isArray(value.data)) return false;
    try {
      value.data.forEach(decodeDbValue);
    } catch {
      return false;
    }
  }
  return true;
}

function isStructuredError(value: unknown): boolean {
  return (
    value === undefined ||
    (isRecord(value) && typeof value.message === "string")
  );
}

export function decodeCollectionsOverviewResponse(
  value: unknown,
): CollectionsOverviewResponse {
  if (
    !isRecord(value) ||
    !Array.isArray(value.collections) ||
    !value.collections.every(isCollectionOverview) ||
    typeof value.count !== "number" ||
    !Number.isInteger(value.count)
  ) {
    throw new Error("Invalid /v1/collections response");
  }
  return value as unknown as CollectionsOverviewResponse;
}

function isCollectionOverview(value: unknown): boolean {
  return (
    isRecord(value) &&
    typeof value.name === "string" &&
    typeof value.mode === "string" &&
    ["strict", "flexible", "Strict", "Flexible"].includes(value.mode) &&
    (!("stats" in value) || isCollectionStats(value.stats)) &&
    (!("schema" in value) || isCollectionSchema(value.schema))
  );
}

function isCollectionStats(value: unknown): boolean {
  return (
    isRecord(value) &&
    typeof value.document_count === "number" &&
    Number.isInteger(value.document_count)
  );
}

function isCollectionSchema(value: unknown): boolean {
  return (
    isRecord(value) &&
    typeof value.name === "string" &&
    typeof value.mode === "string" &&
    Array.isArray(value.fields) &&
    value.fields.every(isFieldDef) &&
    Array.isArray(value.indexes) &&
    value.indexes.every(isIndexDef)
  );
}

function isFieldDef(value: unknown): boolean {
  return (
    isRecord(value) &&
    typeof value.name === "string" &&
    typeof value.type === "string" &&
    typeof value.required === "boolean" &&
    (!("fields" in value) ||
      (Array.isArray(value.fields) && value.fields.every(isFieldDef)))
  );
}

function isIndexDef(value: unknown): boolean {
  return (
    isRecord(value) &&
    typeof value.name === "string" &&
    Array.isArray(value.fields) &&
    value.fields.every((field) => typeof field === "string") &&
    typeof value.unique === "boolean" &&
    typeof value.index_type === "string"
  );
}
