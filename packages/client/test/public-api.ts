import {
  StellarClient,
  StellarDbError,
  decodeCapabilitiesResponse,
  decodeDbValue,
  errorFromResponse,
  type CapabilitiesResponse,
  type DbValue,
  type DecodedDbValue,
  type QueryOptions,
  type StellarClientOptions,
  type TypedSqlResponse,
} from "../src/index.js";

const options: StellarClientOptions = {
  baseUrl: "http://localhost:3000",
  database: "default",
};

const queryOptions: QueryOptions = {
  timeout: "5s",
  sessionId: "session-1",
};

const client = new StellarClient(options);
const decoded: DecodedDbValue = decodeDbValue({
  $type: "reference",
  value: "user:alice",
} satisfies DbValue);
const error = new StellarDbError(400, "bad request");

void client;
void decoded;
void error;
void errorFromResponse;
void queryOptions;
void decodeCapabilitiesResponse;

const capabilities = {
  apiVersion: "v1",
  version: "0.3.0",
  features: {
    explain: true,
    explainAnalyze: true,
    queryCancellation: false,
    auditLog: false,
    schemaMutations: true,
    userManagement: true,
    roleManagement: false,
    apiKeyManagement: true,
    importJobs: false,
    temporalMetrics: false,
    savedQueries: false,
  },
} satisfies CapabilitiesResponse;

void capabilities;

const response = {
  results: [],
  completed: 0,
  elapsed_us: 10,
} satisfies TypedSqlResponse;

void response;
