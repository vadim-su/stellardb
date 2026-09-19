import type { DbValue } from "./db-value.js";

export type FetchLike = (
  input: string | URL | Request,
  init?: RequestInit,
) => Promise<Response>;

export interface StellarClientOptions {
  baseUrl: string;
  database: string;
  token?: string;
  fetch?: FetchLike;
}

export interface StructuredErrorBody {
  error: {
    code?: string;
    message: string;
    severity?: string;
    detail?: string;
    hint?: string;
    statement_index?: number;
  };
}

export type DbRow = DbValue | Record<string, DbValue>;

export interface TypedSqlResult {
  count?: number;
  data?: DbRow[];
  rows_affected?: number;
  session_id?: string;
  statement_index: number;
  statement_type: string;
}

export interface TypedSqlResponse {
  results: TypedSqlResult[];
  completed: number;
  error?: StructuredErrorBody["error"];
  elapsed_us: number;
}

export interface FieldDef {
  name: string;
  type: string;
  required: boolean;
  default?: unknown;
  fields?: FieldDef[];
}

export interface IndexDef {
  name: string;
  fields: string[];
  unique: boolean;
  index_type: string;
}

export interface CollectionSchema {
  name: string;
  mode: string;
  fields: FieldDef[];
  indexes: IndexDef[];
}

export interface CollectionOverview {
  name: string;
  mode: "strict" | "flexible" | "Strict" | "Flexible";
  stats?: { document_count: number };
  schema?: CollectionSchema;
}

export interface CollectionsOverviewResponse {
  collections: CollectionOverview[];
  count: number;
}

export interface ServerFeatures {
  explain: boolean;
  explainAnalyze: boolean;
  queryCancellation: boolean;
  auditLog: boolean;
  schemaMutations: boolean;
  userManagement: boolean;
  roleManagement: boolean;
  apiKeyManagement: boolean;
  importJobs: boolean;
  temporalMetrics: boolean;
  savedQueries: boolean;
}

export interface CapabilitiesResponse {
  apiVersion: "v1";
  version: string;
  features: ServerFeatures;
}
