# Local AI Agent Memory

This reference project shows StellarDB as a local AI-agent memory store using
graph + vector + FTS over the same records. It keeps conversation memories,
mentioned entities, semantic embeddings, keyword-searchable text, and graph
edges in one portable data directory.

## Run StellarDB

From the repository root:

```bash
docker compose up --build
```

Or run a local binary:

```bash
stellar serve --data ./examples/ai-agent-memory/data --port 3000
```

## Load The Example

Create the database, then apply the schema and seed data:

```bash
curl -X POST http://localhost:3000/databases \
  -H "Content-Type: application/json" \
  -d '{"name":"agent_memory"}'
stellar sql "$(cat examples/ai-agent-memory/schema.stellar)" --host localhost:3000 --database agent_memory
stellar sql "$(cat examples/ai-agent-memory/seed.stellarql)" --host localhost:3000 --database agent_memory
```

The same statements can be sent to `/v1/query`:

```bash
curl -X POST http://localhost:3000/v1/query \
  -H "Content-Type: application/json" \
  -H "X-Database: agent_memory" \
  -d @- < examples/ai-agent-memory/v1-query.json
```

## Query Memory

Run the hybrid retrieval examples:

```bash
stellar sql "$(cat examples/ai-agent-memory/query.stellarql)" --host localhost:3000 --database agent_memory
```

The query file demonstrates:

- keyword retrieval with `MATCH(text) @@`;
- semantic retrieval with `embedding <|10|>`;
- fusion with `search::rrf`;
- graph expansion through `->mentions->entity`.

## TypeScript SDK

The same flow works through `@stellardb/client`:

```ts
import { StellarClient } from "@stellardb/client";

const client = new StellarClient({
  baseUrl: "http://localhost:3000",
  database: "agent_memory",
});

const result = await client.query(
  `SELECT * FROM memory
   WHERE MATCH(text) @@ "billing policy"
   LIMIT 5`,
);
```

In a real agent, replace the sample embedding arrays with vectors produced by
your embedding model, then persist the StellarDB data directory as the portable
agent-memory backup.
