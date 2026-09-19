# StellarDB Station

Station is StellarDB's built-in operator workspace, served by the database at
`/_station/`. It provides connection and database selection, system and
collection telemetry, query-pattern inspection, a Monaco-powered SQL workspace
with schema browsing and table, JSON, and graph results, plus user and API-key
administration.

```sh
bun install
bun run dev
```

The development server proxies StellarDB API requests to
`http://127.0.0.1:3000` by default. Set `STELLARDB_DEV_SERVER_URL` when the
server uses another port:

```sh
STELLARDB_DEV_SERVER_URL=http://127.0.0.1:4000 bun run dev
```

Station uses the shared `@stellardb/client` package for the versioned `/v1`
contract. During development, entering the configured proxy target in the
connection dialog automatically routes requests through the Station origin to
avoid local CORS failures.
