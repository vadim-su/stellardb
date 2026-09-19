# @stellardb/client

Typed JavaScript/TypeScript client surface for StellarDB HTTP v1.

This package is intentionally framework-free. Connection state, authentication
prompts, caching, and UI concerns belong in the host application; this package
owns the transport contract, typed `/v1/query` responses, collection overview
fetches, structured errors, capability discovery, and `DbValue` runtime decoding.

## Install

```bash
bun add @stellardb/client
```

```bash
npm install @stellardb/client
```

## Query

```ts
import { StellarClient, decodeDbValue } from "@stellardb/client";

const client = new StellarClient({
  baseUrl: "http://localhost:3000",
  database: "default",
});

const result = await client.query("SELECT user:alice AS owner");
const owner = decodeDbValue(result.results[0].data?.[0]?.owner);
```

`client.query` uses `POST /v1/query` and returns the typed response contract,
including `elapsed_us` as the authoritative numeric server duration.

## Collections

```ts
const overview = await client.collections(["stats", "schema"]);

for (const collection of overview.collections) {
  console.log(collection.name, collection.stats?.document_count);
}
```

`client.collections` uses `GET /v1/collections?include=...` so consumers can
load collection names, stats, and schemas in one request.

## Capabilities

```ts
const capabilities = await client.capabilities();
console.log(capabilities.apiVersion, capabilities.features.explain);
```

`client.capabilities` validates the exact `/v1/capabilities` feature contract.

## Structured Errors

```ts
import {
  StellarClient,
  StellarDbError,
  errorFromResponse,
} from "@stellardb/client";

try {
  await new StellarClient({
    baseUrl: "http://localhost:3000",
    database: "default",
  }).query("SELECT * FROM missing");
} catch (error) {
  if (error instanceof StellarDbError) {
    console.error(error.status, error.code, error.hint);
  }
}

const response = await fetch("http://localhost:3000/v1/collections");
if (!response.ok) {
  throw await errorFromResponse(response);
}
```

`StellarDbError` preserves server metadata such as `status`, `code`,
`severity`, `detail`, `hint`, and `statementIndex`.

The client targets StellarDB's stable HTTP v1 endpoints and typed JSON value format.
