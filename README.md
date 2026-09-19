<p align="center">
  <img src="assets/stellardb-hero.webp" alt="StellarDB — one database for documents, graphs, vectors, and full-text search" width="100%">
</p>

<h1 align="center">✨ StellarDB</h1>

<p align="center">
  <strong>Documents, graphs, vectors, and full-text search. One Rust database. One query language.</strong>
</p>

<p align="center">
  <a href="https://github.com/vadim-su/stellardb/actions/workflows/ci.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/vadim-su/stellardb/ci.yml?branch=main&style=for-the-badge&logo=github"></a>
  <img alt="Version 0.3.0" src="https://img.shields.io/badge/version-0.3.0-7c3aed?style=for-the-badge">
  <img alt="Rust 2024" src="https://img.shields.io/badge/Rust-2024-f97316?style=for-the-badge&logo=rust&logoColor=white">
  <a href="#license"><img alt="MIT License" src="https://img.shields.io/badge/license-MIT-22c55e?style=for-the-badge"></a>
</p>

StellarDB is an embeddable multi-model database built in Rust. It combines a document store, graph traversal, HNSW vector search, and BM25 full-text search in one binary—without stitching together four separate systems.

> [!NOTE]
> StellarDB is under active development. Pin versions and validate backups before production use.

## 🌌 Why StellarDB?

| Model | What you get |
| --- | --- |
| 📄 **Documents** | Nested records, arrays, rich scalar types, schemas, and secondary indexes |
| 🕸️ **Graphs** | Typed edges, bidirectional indexes, and variable-depth traversal |
| 🧭 **Vectors** | HNSW indexes with cosine, Euclidean, and dot-product distance |
| 🔎 **Full text** | Tantivy-powered BM25, analyzers, boosts, scores, and highlighting |
| ✨ **Hybrid search** | Reciprocal Rank Fusion across keyword and semantic results |

All models share the same storage engine, transaction layer, authorization path, and StellarQL query language.

## 🚀 Quick start

### Build

```bash
git clone https://github.com/vadim-su/stellardb.git
cd stellardb
cargo build --release
```

The executable is written to `target/release/stellar`.

### Start the server

```bash
./target/release/stellar serve --port 3000 --data ./data
```

Then open **http://localhost:3000/_station/** for the built-in Station UI.

### Run with Docker

```bash
docker compose up --build
```

The HTTP API is available at **http://localhost:3000**. Data is persisted in the `stellardb-data` volume.

### Send your first query

```bash
curl -X POST http://localhost:3000/sql \
  -H 'Content-Type: application/json' \
  -H 'X-Database: default' \
  -d '{"query":"CREATE user:alice SET name = \"Alice\", age = 30"}'
```

```bash
./target/release/stellar sql \
  'SELECT name, age FROM user:alice' \
  --host localhost:3000
```

## 🪐 One language, four models

### Documents

```sql
DEFINE COLLECTION user (
  SCHEMA STRICT,
  name string REQUIRED,
  email string REQUIRED,
  age int DEFAULT 0,
  INDEX ON email UNIQUE
);

CREATE user:alice SET
  name = "Alice",
  email = "alice@example.com",
  age = 30;

SELECT name, age
FROM user
WHERE age >= 21
ORDER age DESC
LIMIT 10;
```

### Graphs

```sql
CREATE user:bob SET name = "Bob";
CREATE user:carol SET name = "Carol";

RELATE user:alice->follows->user:bob SET since = 2024;
RELATE user:bob->follows->user:carol SET since = 2025;

SELECT ->follows{1..3}->user.{name}
FROM user:alice;
```

### Vector search

```sql
CREATE INDEX ON article(embedding)
HNSW DIMENSION 768 DIST COSINE;

SELECT title, vector::distance() AS distance
FROM article
WHERE embedding <|10|> $query_vector;
```

### Full-text search

```sql
DEFINE ANALYZER english
TOKENIZER standard
FILTERS [lowercase, stemmer(english)];

CREATE INDEX ON article(title, body)
FULLTEXT ANALYZER english;

SELECT title, fts::score() AS score
FROM article
WHERE MATCH(title^3, body) @@ "rust database"
ORDER score DESC
LIMIT 10;
```

### Hybrid search

```sql
SELECT * FROM search::rrf([
  (SELECT id, fts::score() AS score
   FROM article
   WHERE body @@ "multi model database"
   LIMIT 20),
  (SELECT id, vector::distance() AS distance
   FROM article
   WHERE embedding <|20|> $query_vector)
], 10);
```

## 🦀 Embedded Rust API

Use StellarDB directly inside a Rust process—no HTTP server and no caller-provided Tokio runtime required.

```rust
use stellardb::{EmbeddedDatabase, Value};

let db = EmbeddedDatabase::open("./data")?;
db.execute("DEFINE COLLECTION user")?;
db.execute(r#"CREATE user:alice SET name = "Alice", age = 30"#)?;

let rows = db.query("SELECT name, age FROM user:alice")?;
assert_eq!(
    rows[0].get_field("name"),
    Some(Value::String("Alice".into()))
);

db.close()?;
# Ok::<(), stellardb::EmbeddedError>(())
```

A lightweight library build excludes the CLI and server stack:

```toml
[dependencies]
stellardb = {
  git = "https://github.com/vadim-su/stellardb",
  default-features = false,
  features = ["embedded-lite"]
}
```

Add `fts`, `hnsw`, or both when an embedded application needs search indexes.

## ⚡ Core capabilities

- **ACID writes** with optimistic concurrency control
- **Strict or flexible schemas** with unions, references, defaults, and validation
- **B-tree indexes** for equality, range, compound, and unique constraints
- **Multi-database isolation** through namespaces and `X-Database`
- **HTTP and gRPC APIs** with typed values and structured errors
- **Durable background DDL** with progress, cancellation, and restart recovery
- **Native TLS**, JWT authentication, API keys, and policy-based authorization
- **Prometheus metrics**, bounded cardinality, query timing, and index telemetry
- **Backup and restore** through the CLI
- **Station UI** embedded into the server binary

## 🏗️ Architecture

```text
Clients / Station / Embedded API
                │
        HTTP · gRPC · Rust
                │
      Parse → Bind → Optimize
                │
             Execute
       ┌────────┼────────┐
       │        │        │
   Documents  Graph   Search
       │        │     ┌──┴──┐
       │        │   HNSW  BM25
       └────────┴──────┬────┘
                      │
             Fjall + rkyv storage
```

| Layer | Technology |
| --- | --- |
| Storage | Fjall LSM-tree |
| Serialization | rkyv zero-copy reads |
| HTTP | Axum |
| gRPC | Tonic |
| Full-text search | Tantivy |
| Vector search | usearch HNSW |
| Web UI | React + TypeScript, embedded with `rust-embed` |

## 🔌 HTTP API

Every data request selects a database through `X-Database`.

```bash
# Query
curl -X POST http://localhost:3000/v1/query \
  -H 'Content-Type: application/json' \
  -H 'X-Database: default' \
  -d '{"query":"SELECT * FROM user LIMIT 20"}'

# Collection metadata
curl 'http://localhost:3000/v1/collections?include=stats,schema' \
  -H 'X-Database: default'

# Server capabilities
curl http://localhost:3000/v1/capabilities
```

A framework-free TypeScript client lives in [`packages/client`](packages/client).

```ts
import { StellarClient } from "@stellardb/client";

const db = new StellarClient({
  baseUrl: "http://localhost:3000",
  database: "default",
});

const result = await db.query("SELECT * FROM user LIMIT 20");
```

## 🧰 CLI essentials

```bash
# Server
stellar serve --host 127.0.0.1 --port 3000 --data ./data

# Query
stellar sql 'SELECT * FROM user LIMIT 10' --host localhost:3000

# Schema
stellar schema list --host localhost:3000
stellar schema show user --host localhost:3000

# Backup and restore
stellar backup --data ./data --output ./backups/snapshot
stellar restore --input ./backups/snapshot --data ./restored-data
```

## 🎛️ Feature flags

Default builds include the complete CLI, server, authentication, gRPC, TLS, full-text search, vector search, and Station UI.

| Flag | Purpose |
| --- | --- |
| `cli` | Build the `stellar` executable and CLI dependencies |
| `server` | Enable HTTP server runtime |
| `auth` | Enable JWT, password, API-key, and policy services |
| `grpc` | Enable protobuf generation and the gRPC server |
| `tls` | Enable rustls for HTTP, CLI, and gRPC transport |
| `fts` | Enable Tantivy full-text indexes |
| `hnsw` | Enable usearch vector indexes |
| `ui` | Embed Station into the server binary |
| `embedded-lite` | Allow library builds without search engines |

Useful build profiles:

```bash
# Complete release binary
cargo build --release

# Small embedded document/graph library
cargo build --no-default-features --features embedded-lite --lib

# Embedded library with both search engines
cargo build --no-default-features --features fts,hnsw --lib

# Lightweight CLI without the server
cargo build --no-default-features --features cli,embedded-lite --bin stellar
```

## 🧪 Development

Install the pinned Rust and Bun toolchains, then run the repository checks:

```bash
mise install
mise run check
```

The repository also includes:

- `station/` — the embedded administration and query workspace, served at `/_station/`
- `packages/client/` — the standalone TypeScript HTTP client
- `examples/ai-agent-memory/` — a graph + vector + FTS example
- `benches/` — reproducible Criterion benchmarks
- `scripts/` — dataset loaders and benchmark tooling

Build StellarDB with the embedded Station, or build Station independently:

```bash
mise run build
mise run station
```

With a local StellarDB server running, the scripts in `scripts/` can populate
sample and benchmark datasets. The e-commerce loader additionally requires the
Python `requests` package:

```bash
python -m pip install requests
python scripts/load_ecommerce.py
```

## 🔐 Security defaults

- Non-loopback plaintext binds require explicit opt-in.
- TLS key files must have safe Unix permissions and cannot be symlinks.
- CORS is disabled unless an allowlist is configured.
- HTTP body size, gRPC message size, concurrency, and query deadlines are bounded.
- Login rate limiting includes process-wide and per-client buckets.

For internet-facing deployments, use native TLS or place StellarDB behind a trusted TLS-terminating reverse proxy.

### Public (anonymous) access

Every request needs a bearer token unless `--anonymous-user <name>` (env `STELLAR_ANONYMOUS_USER`) names a managed user. Requests without an `Authorization` header then run as that user, so its policies decide what anonymous clients may do; invalid tokens are still rejected and `root` cannot be used.

```sql
CREATE USER 'guest' PASSWORD 'unused';
ALTER USER 'guest' SET role = 'public';
CREATE POLICY public_read
  WHEN subject.role = 'public' AND action = 'SELECT' AND resource.database = 'catalog'
  ALLOW;
```

```bash
STELLAR_ANONYMOUS_USER=guest stellar serve
```

## 📜 License

StellarDB is available under the [MIT License](LICENSE).
