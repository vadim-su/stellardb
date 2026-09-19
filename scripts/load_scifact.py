#!/usr/bin/env python3
"""Download SciFact and load its scientific evidence graph into StellarDB.

Only Python's standard library is required.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import os
import shutil
import sys
import time
import urllib.error
import urllib.request
import zipfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable, Iterator, Sequence


BEIR_BASE_URL = "https://public.ukp.informatik.tu-darmstadt.de/thakur/BEIR/datasets"


@dataclass(frozen=True)
class DatasetSpec:
    name: str
    md5: str


SCIFACT = DatasetSpec("scifact", "5f7d1de60b170fc8027bb7898e2efca1")


class LoaderError(RuntimeError):
    """A user-facing loader failure."""


class StellarClient:
    def __init__(
        self,
        base_url: str,
        database: str,
        token: str | None,
        timeout: float,
        query_timeout: str | None,
    ) -> None:
        self.base_url = base_url.rstrip("/")
        self.endpoint = f"{self.base_url}/v1/query"
        self.database = database
        self.token = token
        self.timeout = timeout
        self.query_timeout = query_timeout

    def execute(self, query: str) -> dict[str, Any]:
        headers = {
            "Content-Type": "application/json",
            "X-Database": self.database,
            "User-Agent": "stellardb-beir-loader/1.0",
        }
        if self.token:
            headers["Authorization"] = f"Bearer {self.token}"
        if self.query_timeout:
            headers["X-Query-Timeout"] = self.query_timeout

        request = urllib.request.Request(
            self.endpoint,
            data=json.dumps({"query": query}).encode("utf-8"),
            headers=headers,
            method="POST",
        )
        try:
            with urllib.request.urlopen(request, timeout=self.timeout) as response:
                body = json.load(response)
        except urllib.error.HTTPError as error:
            detail = error.read().decode("utf-8", errors="replace")
            raise LoaderError(f"HTTP {error.code} from StellarDB: {detail[:1000]}") from error
        except urllib.error.URLError as error:
            raise LoaderError(f"Cannot reach StellarDB at {self.endpoint}: {error.reason}") from error
        except json.JSONDecodeError as error:
            raise LoaderError(f"StellarDB returned invalid JSON: {error}") from error

        if body.get("error"):
            error = body["error"]
            message = error.get("message", error) if isinstance(error, dict) else error
            raise LoaderError(f"StellarDB query failed: {message}")
        return body

    def ensure_database(self) -> None:
        headers = {
            "Content-Type": "application/json",
            "User-Agent": "stellardb-beir-loader/1.0",
        }
        if self.token:
            headers["Authorization"] = f"Bearer {self.token}"
        request = urllib.request.Request(
            f"{self.base_url}/databases",
            data=json.dumps({"name": self.database}).encode("utf-8"),
            headers=headers,
            method="POST",
        )
        try:
            with urllib.request.urlopen(request, timeout=self.timeout):
                pass
        except urllib.error.HTTPError as error:
            if error.code == 409:
                return
            detail = error.read().decode("utf-8", errors="replace")
            raise LoaderError(
                f"Cannot create database {self.database!r}: HTTP {error.code}: {detail[:1000]}"
            ) from error
        except urllib.error.URLError as error:
            raise LoaderError(
                f"Cannot reach StellarDB at {self.base_url}: {error.reason}"
            ) from error


def verify_server_compatibility(client: StellarClient) -> None:
    """Fail early when the server cannot parse JSON-escaped BEIR text."""
    try:
        client.execute(r'''SELECT "BEIR says: \"ready\"" AS escaped_string''')
    except LoaderError as error:
        message = str(error)
        if "SDB-QP001" in message or "Parse error" in message:
            raise LoaderError(
                "This StellarDB server cannot parse JSON-escaped string literals. "
                "Rebuild and restart StellarDB with the escaped-string parser fix "
                "(commit abc990d or newer), then retry the import."
            ) from error
        raise


def file_md5(path: Path) -> str:
    digest = hashlib.md5(usedforsecurity=False)
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def download_dataset(spec: DatasetSpec, cache_dir: Path, redownload: bool) -> Path:
    cache_dir.mkdir(parents=True, exist_ok=True)
    archive = cache_dir / f"{spec.name}.zip"
    extracted = cache_dir / spec.name

    if redownload:
        archive.unlink(missing_ok=True)
        if extracted.exists():
            shutil.rmtree(extracted)

    if archive.exists() and file_md5(archive) != spec.md5:
        print(f"Cached {archive} has the wrong checksum; downloading it again.")
        archive.unlink()

    if not archive.exists():
        url = f"{BEIR_BASE_URL}/{spec.name}.zip"
        partial = archive.with_suffix(".zip.part")
        partial.unlink(missing_ok=True)
        print(f"Downloading {url}")
        request = urllib.request.Request(url, headers={"User-Agent": "stellardb-beir-loader/1.0"})
        try:
            with urllib.request.urlopen(request, timeout=120) as response, partial.open("wb") as target:
                shutil.copyfileobj(response, target, length=1024 * 1024)
        except (OSError, urllib.error.URLError) as error:
            partial.unlink(missing_ok=True)
            raise LoaderError(f"Failed to download {url}: {error}") from error
        partial.replace(archive)

    actual_md5 = file_md5(archive)
    if actual_md5 != spec.md5:
        raise LoaderError(
            f"Checksum mismatch for {archive}: expected {spec.md5}, got {actual_md5}"
        )

    if not extracted.exists():
        print(f"Extracting {archive}")
        extracted.mkdir(parents=True)
        with zipfile.ZipFile(archive) as source:
            root = extracted.resolve()
            for member in source.infolist():
                destination = (extracted / member.filename).resolve()
                if destination != root and root not in destination.parents:
                    raise LoaderError(f"Unsafe path in {archive}: {member.filename}")
                if member.is_dir():
                    destination.mkdir(parents=True, exist_ok=True)
                    continue
                destination.parent.mkdir(parents=True, exist_ok=True)
                with source.open(member) as input_file, destination.open("wb") as output_file:
                    shutil.copyfileobj(input_file, output_file)

    return find_dataset_root(extracted)


def find_dataset_root(extracted: Path) -> Path:
    candidates = [extracted, *[path.parent for path in extracted.rglob("corpus.jsonl")]]
    for candidate in candidates:
        if (candidate / "corpus.jsonl").is_file() and (candidate / "queries.jsonl").is_file():
            return candidate
    raise LoaderError(f"No BEIR corpus.jsonl and queries.jsonl found under {extracted}")


def stable_id(dataset: str, kind: str, source_id: str) -> str:
    digest = hashlib.sha256(source_id.encode("utf-8")).hexdigest()[:32]
    return f"{dataset}_{kind}_{digest}"


def read_jsonl(path: Path) -> Iterator[dict[str, Any]]:
    with path.open("r", encoding="utf-8") as source:
        for line_number, line in enumerate(source, 1):
            if not line.strip():
                continue
            try:
                value = json.loads(line)
            except json.JSONDecodeError as error:
                raise LoaderError(f"Invalid JSON in {path}:{line_number}: {error}") from error
            if not isinstance(value, dict):
                raise LoaderError(f"Expected an object in {path}:{line_number}")
            yield value


def document_rows(dataset: str, root: Path) -> Iterator[dict[str, Any]]:
    for source in read_jsonl(root / "corpus.jsonl"):
        source_id = str(source.get("_id", ""))
        if not source_id:
            raise LoaderError(f"A document in {root / 'corpus.jsonl'} has no _id")
        yield {
            "id": stable_id(dataset, "doc", source_id),
            "dataset": dataset,
            "source_id": source_id,
            "title": str(source.get("title") or ""),
            "text": str(source.get("text") or ""),
            "metadata": source.get("metadata") or {},
        }


def query_rows(dataset: str, root: Path) -> Iterator[dict[str, Any]]:
    for source in read_jsonl(root / "queries.jsonl"):
        source_id = str(source.get("_id", ""))
        if not source_id:
            raise LoaderError(f"A query in {root / 'queries.jsonl'} has no _id")
        yield {
            "id": stable_id(dataset, "query", source_id),
            "dataset": dataset,
            "source_id": source_id,
            "text": str(source.get("text") or ""),
            "metadata": source.get("metadata") or {},
        }


def qrel_rows(dataset: str, root: Path, splits: set[str] | None) -> Iterator[dict[str, Any]]:
    qrels_dir = root / "qrels"
    if not qrels_dir.is_dir():
        raise LoaderError(f"No qrels directory found under {root}")

    qrel_files = sorted(qrels_dir.glob("*.tsv"))
    if not qrel_files:
        raise LoaderError(f"No qrels TSV files found under {qrels_dir}")

    found_splits: set[str] = set()
    for path in qrel_files:
        split = path.stem
        if splits is not None and split not in splits:
            continue
        found_splits.add(split)
        with path.open("r", encoding="utf-8", newline="") as source:
            reader = csv.DictReader(source, delimiter="\t")
            fields = set(reader.fieldnames or [])
            query_key = next((key for key in ("query-id", "query_id", "qid") if key in fields), None)
            corpus_key = next(
                (key for key in ("corpus-id", "corpus_id", "doc-id", "doc_id") if key in fields),
                None,
            )
            score_key = next((key for key in ("score", "relevance") if key in fields), None)
            if not query_key or not corpus_key or not score_key:
                raise LoaderError(f"Unexpected qrels header in {path}: {reader.fieldnames}")

            for line_number, row in enumerate(reader, 2):
                query_id = str(row[query_key])
                document_id = str(row[corpus_key])
                try:
                    relevance = int(row[score_key])
                except ValueError as error:
                    raise LoaderError(
                        f"Invalid relevance score in {path}:{line_number}: {row[score_key]!r}"
                    ) from error
                identity = f"{split}\0{query_id}\0{document_id}"
                yield {
                    "id": stable_id(dataset, "qrel", identity),
                    "dataset": dataset,
                    "split": split,
                    "query_id": query_id,
                    "query_record_id": stable_id(dataset, "query", query_id),
                    "document_id": document_id,
                    "document_record_id": stable_id(dataset, "doc", document_id),
                    "relevance": relevance,
                }

    if splits is not None:
        missing = splits - found_splits
        if missing:
            raise LoaderError(f"Dataset {dataset} does not contain qrels split(s): {', '.join(sorted(missing))}")


def split_rows(dataset: str, root: Path, splits: set[str] | None) -> Iterator[dict[str, Any]]:
    for path in sorted((root / "qrels").glob("*.tsv")):
        split = path.stem
        if splits is None or split in splits:
            yield {
                "id": stable_id(dataset, "split", split),
                "dataset": dataset,
                "name": split,
            }


def evidence_rows(root: Path) -> Iterator[dict[str, Any]]:
    for query in read_jsonl(root / "queries.jsonl"):
        claim_id = str(query["_id"])
        metadata = query.get("metadata") or {}
        for paper_id, evidence_sets in metadata.items():
            for ordinal, evidence in enumerate(evidence_sets):
                identity = f"{claim_id}\0{paper_id}\0{ordinal}"
                yield {
                    "id": stable_id("scifact", "evidence", identity),
                    "claim_id": claim_id,
                    "claim_record_id": stable_id("scifact", "query", claim_id),
                    "paper_id": str(paper_id),
                    "paper_record_id": stable_id("scifact", "doc", str(paper_id)),
                    "sentence_indices": evidence.get("sentences") or [],
                    "label": str(evidence["label"]),
                }


def label_rows(root: Path) -> Iterator[dict[str, Any]]:
    labels = sorted({row["label"] for row in evidence_rows(root)})
    for label in labels:
        yield {"id": label.lower(), "name": label}


def collection_names() -> tuple[str, str, str, str, str, str]:
    return "papers", "claims", "judgments", "evidence_sets", "labels", "splits"


def create_collections(client: StellarClient, replace: bool) -> tuple[str, str, str, str, str, str]:
    papers, claims, judgments, evidence_sets, labels, splits = collection_names()
    if replace:
        for collection in (evidence_sets, judgments, labels, splits, claims, papers):
            try:
                client.execute(f"DROP COLLECTION {collection} CASCADE")
            except LoaderError as error:
                if "not found" not in str(error).lower():
                    raise

    client.execute(
        f"""DEFINE COLLECTION {papers} (
            SCHEMA FLEXIBLE,
            dataset string REQUIRED,
            source_id string REQUIRED,
            title string,
            text string REQUIRED,
            metadata object
        )"""
    )
    client.execute(
        f"""DEFINE COLLECTION {claims} (
            SCHEMA FLEXIBLE,
            dataset string REQUIRED,
            source_id string REQUIRED,
            text string REQUIRED,
            metadata object
        )"""
    )
    client.execute(
        f"""DEFINE COLLECTION {judgments} (
            SCHEMA FLEXIBLE,
            dataset string REQUIRED,
            split string REQUIRED,
            query_id string REQUIRED,
            query_record_id string REQUIRED,
            document_id string REQUIRED,
            document_record_id string REQUIRED,
            relevance int REQUIRED
        )"""
    )
    client.execute(
        f"""DEFINE COLLECTION {evidence_sets} (
            SCHEMA FLEXIBLE,
            claim_id string REQUIRED,
            claim_record_id string REQUIRED,
            paper_id string REQUIRED,
            paper_record_id string REQUIRED,
            sentence_indices array REQUIRED,
            label string REQUIRED
        )"""
    )
    client.execute(
        f"""DEFINE COLLECTION {labels} (
            SCHEMA FLEXIBLE,
            name string REQUIRED
        )"""
    )
    client.execute(
        f"""DEFINE COLLECTION {splits} (
            SCHEMA FLEXIBLE,
            dataset string REQUIRED,
            name string REQUIRED
        )"""
    )
    return papers, claims, judgments, evidence_sets, labels, splits


def serialized_batches(
    rows: Iterable[dict[str, Any]], batch_size: int, max_batch_bytes: int
) -> Iterator[tuple[list[str], list[str]]]:
    values: list[str] = []
    record_ids: list[str] = []
    size = 0
    for row in rows:
        try:
            value = json.dumps(
                row,
                ensure_ascii=False,
                separators=(",", ":"),
                allow_nan=False,
            )
        except (TypeError, ValueError) as error:
            raise LoaderError(f"Cannot serialize BEIR record {row.get('id')}: {error}") from error
        value_size = len(value.encode("utf-8"))
        if values and (len(values) >= batch_size or size + value_size > max_batch_bytes):
            yield values, record_ids
            values, record_ids, size = [], [], 0
        values.append(value)
        record_ids.append(str(row.get("id", "<unknown>")))
        size += value_size
    if values:
        yield values, record_ids


def insert_rows(
    client: StellarClient,
    collection: str,
    rows: Iterable[dict[str, Any]],
    batch_size: int,
    max_batch_bytes: int,
) -> int:
    inserted = 0
    started = time.monotonic()
    for values, record_ids in serialized_batches(rows, batch_size, max_batch_bytes):
        query = f"INSERT INTO {collection} {','.join(values)}"
        try:
            client.execute(query)
        except LoaderError as error:
            raise LoaderError(
                f"Failed to insert batch into {collection}; first record is {record_ids[0]}: {error}"
            ) from error
        inserted += len(values)
        if inserted % 1000 < len(values):
            elapsed = max(time.monotonic() - started, 0.001)
            print(f"  {collection}: {inserted:,} rows ({inserted / elapsed:,.0f} rows/s)")
    elapsed = max(time.monotonic() - started, 0.001)
    print(f"  {collection}: loaded {inserted:,} rows in {elapsed:.1f}s")
    return inserted


def judgment_edge_statements(rows: Iterable[dict[str, Any]]) -> Iterator[tuple[str, str]]:
    papers, claims, judgments, _, _, splits = collection_names()
    for row in rows:
        fields = {
            "dataset": "scifact",
            "split": row["split"],
            "relevance": row["relevance"],
            "query_id": row["query_id"],
            "document_id": row["document_id"],
        }
        content = json.dumps(fields, ensure_ascii=False, separators=(",", ":"))
        claim = f"{claims}:{row['query_record_id']}"
        judgment = f"{judgments}:{row['id']}"
        paper = f"{papers}:{row['document_record_id']}"
        split = f"{splits}:{stable_id('scifact', 'split', str(row['split']))}"
        yield f"RELATE {claim}->has_judgment->{judgment} RETURN NONE", str(row["id"])
        yield f"RELATE {judgment}->ranks->{paper} RETURN NONE", str(row["id"])
        yield f"RELATE {judgment}->in_split->{split} RETURN NONE", str(row["id"])
        yield (
            f"RELATE {claim}->relevant_to->{paper} CONTENT {content} RETURN NONE",
            str(row["id"]),
        )


def evidence_edge_statements(rows: Iterable[dict[str, Any]]) -> Iterator[tuple[str, str]]:
    papers, claims, _, evidence_sets, labels, _ = collection_names()
    for row in rows:
        claim = f"{claims}:{row['claim_record_id']}"
        evidence = f"{evidence_sets}:{row['id']}"
        paper = f"{papers}:{row['paper_record_id']}"
        label = f"{labels}:{str(row['label']).lower()}"
        identity = str(row["id"])
        yield f"RELATE {claim}->has_evidence->{evidence} RETURN NONE", identity
        yield f"RELATE {evidence}->extracted_from->{paper} RETURN NONE", identity
        yield f"RELATE {evidence}->classified_as->{label} RETURN NONE", identity
        relation = "supported_by" if row["label"] == "SUPPORT" else "contradicted_by"
        yield f"RELATE {claim}->{relation}->{paper} RETURN NONE", identity


def insert_edges(
    client: StellarClient,
    statements_iter: Iterable[tuple[str, str]],
    batch_size: int,
    max_batch_bytes: int,
) -> int:
    inserted = 0
    started = time.monotonic()
    statements: list[str] = []
    record_ids: list[str] = []
    size = 0

    def flush() -> None:
        nonlocal inserted, statements, record_ids, size
        if not statements:
            return
        previous = inserted
        try:
            client.execute(";".join(statements))
        except LoaderError as error:
            raise LoaderError(
                f"Failed to create benchmark edges; first judgment is {record_ids[0]}: {error}"
            ) from error
        inserted += len(statements)
        statements, record_ids, size = [], [], 0
        if inserted // 1000 > previous // 1000:
            elapsed = max(time.monotonic() - started, 0.001)
            print(f"  graph edges: {inserted:,} ({inserted / elapsed:,.0f} edges/s)")

    for statement, record_id in statements_iter:
        statement_size = len(statement.encode("utf-8"))
        if statements and (
            len(statements) >= batch_size or size + statement_size > max_batch_bytes
        ):
            flush()
        statements.append(statement)
        record_ids.append(record_id)
        size += statement_size
    flush()
    elapsed = max(time.monotonic() - started, 0.001)
    print(f"  graph edges: loaded {inserted:,} edges in {elapsed:.1f}s")
    return inserted


def create_indexes(client: StellarClient, analyzer: str | None) -> None:
    papers, claims, judgments, evidence_sets, labels, splits = collection_names()
    client.execute(f"CREATE INDEX ON {papers}(source_id) UNIQUE")
    analyzer_clause = f" ANALYZER {analyzer}" if analyzer else ""
    client.execute(f"CREATE INDEX ON {papers}(title, text) FULLTEXT{analyzer_clause}")
    client.execute(f"CREATE INDEX ON {claims}(source_id) UNIQUE")
    client.execute(f"CREATE INDEX ON {claims}(text) FULLTEXT{analyzer_clause}")
    client.execute(f"CREATE INDEX ON {judgments}(query_id, document_id)")
    client.execute(f"CREATE INDEX ON {judgments}(split, relevance)")
    client.execute(f"CREATE INDEX ON {evidence_sets}(claim_id, paper_id)")
    client.execute(f"CREATE INDEX ON {evidence_sets}(label)")
    client.execute(f"CREATE INDEX ON {labels}(name) UNIQUE")
    client.execute(f"CREATE INDEX ON {splits}(name) UNIQUE")


def count_rows(rows: Iterable[dict[str, Any]]) -> int:
    return sum(1 for _ in rows)


def load_one_dataset(
    client: StellarClient | None,
    spec: DatasetSpec,
    root: Path,
    splits: set[str] | None,
    replace: bool,
    analyzer: str | None,
    batch_size: int,
    max_batch_bytes: int,
) -> dict[str, int]:
    print(f"\n[{spec.name}] source: {root}")
    if client is None:
        judgment_count = count_rows(qrel_rows(spec.name, root, splits))
        evidence_count = count_rows(evidence_rows(root))
        counts = {
            "papers": count_rows(document_rows(spec.name, root)),
            "claims": count_rows(query_rows(spec.name, root)),
            "judgments": judgment_count,
            "evidence_sets": evidence_count,
            "labels": count_rows(label_rows(root)),
            "splits": count_rows(split_rows(spec.name, root, splits)),
            "edges": (judgment_count + evidence_count) * 4,
        }
        print(
            f"  dry run: {counts['papers']:,} papers, {counts['claims']:,} claims, "
            f"{counts['judgments']:,} judgments, {counts['evidence_sets']:,} evidence sets, "
            f"{counts['labels']:,} labels, {counts['splits']:,} splits, "
            f"{counts['edges']:,} graph edges"
        )
        return counts

    papers, claims, judgments, evidence_sets, labels, split_collection = create_collections(
        client, replace
    )
    counts = {
        "papers": insert_rows(
            client, papers, document_rows(spec.name, root), batch_size, max_batch_bytes
        ),
        "claims": insert_rows(
            client, claims, query_rows(spec.name, root), batch_size, max_batch_bytes
        ),
        "judgments": insert_rows(
            client,
            judgments,
            qrel_rows(spec.name, root, splits),
            batch_size,
            max_batch_bytes,
        ),
        "evidence_sets": insert_rows(
            client, evidence_sets, evidence_rows(root), batch_size, max_batch_bytes
        ),
        "labels": insert_rows(client, labels, label_rows(root), batch_size, max_batch_bytes),
        "splits": insert_rows(
            client,
            split_collection,
            split_rows(spec.name, root, splits),
            batch_size,
            max_batch_bytes,
        ),
        "judgment_edges": insert_edges(
            client,
            judgment_edge_statements(qrel_rows(spec.name, root, splits)),
            batch_size,
            max_batch_bytes,
        ),
        "evidence_edges": insert_edges(
            client,
            evidence_edge_statements(evidence_rows(root)),
            batch_size,
            max_batch_bytes,
        ),
    }
    counts["edges"] = counts.pop("judgment_edges") + counts.pop("evidence_edges")
    print(f"  {spec.name}: creating indexes")
    create_indexes(client, analyzer)
    return counts


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Download SciFact from BEIR and load its scientific evidence graph into StellarDB."
    )
    parser.add_argument("--url", default="http://127.0.0.1:3000", help="StellarDB base URL")
    parser.add_argument(
        "--database",
        required=True,
        help="Target StellarDB database name",
    )
    parser.add_argument(
        "--splits",
        nargs="+",
        help="Only load selected qrels splits, for example: --splits test (default: all)",
    )
    parser.add_argument(
        "--cache-dir",
        type=Path,
        default=Path(".cache/beir"),
        help="Download/extraction cache (default: .cache/beir)",
    )
    parser.add_argument("--token", help="Bearer token; defaults to STELLARDB_TOKEN")
    parser.add_argument("--replace", action="store_true", help="Drop and recreate target collections")
    parser.add_argument("--redownload", action="store_true", help="Discard cached copies and download again")
    parser.add_argument(
        "--analyzer",
        help="Optional existing StellarDB analyzer for the full-text indexes",
    )
    parser.add_argument("--batch-size", type=int, default=100, help="Maximum rows per INSERT")
    parser.add_argument(
        "--max-batch-bytes",
        type=int,
        default=512 * 1024,
        help="Approximate maximum serialized bytes per INSERT",
    )
    parser.add_argument("--timeout", type=float, default=900, help="HTTP timeout in seconds")
    parser.add_argument(
        "--query-timeout",
        help="Optional X-Query-Timeout value sent to StellarDB, for example 2m",
    )
    parser.add_argument("--dry-run", action="store_true", help="Download, validate, and count without loading")
    args = parser.parse_args(argv)

    if args.batch_size < 1:
        parser.error("--batch-size must be at least 1")
    if args.max_batch_bytes < 1024:
        parser.error("--max-batch-bytes must be at least 1024")
    if args.analyzer and not args.analyzer.replace("_", "").isalnum():
        parser.error("--analyzer must contain only letters, digits, and underscores")
    return args


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    if args.token is None:
        args.token = os.environ.get("STELLARDB_TOKEN")

    spec = SCIFACT
    client = None
    if not args.dry_run:
        client = StellarClient(
            args.url,
            args.database,
            args.token,
            args.timeout,
            args.query_timeout,
        )
        client.ensure_database()
        print(f"Target: {args.url.rstrip('/')} database={args.database}")
        verify_server_compatibility(client)
    root = download_dataset(spec, args.cache_dir, args.redownload)
    counts = load_one_dataset(
        client=client,
        spec=spec,
        root=root,
        splits=set(args.splits) if args.splits else None,
        replace=args.replace,
        analyzer=args.analyzer,
        batch_size=args.batch_size,
        max_batch_bytes=args.max_batch_bytes,
    )

    print("\nCompleted:")
    print(
        f"  scifact: {counts['papers']:,} papers, {counts['claims']:,} claims, "
        f"{counts['judgments']:,} judgments, {counts['evidence_sets']:,} evidence sets, "
        f"{counts['labels']:,} labels, {counts['splits']:,} splits, "
        f"{counts['edges']:,} graph edges"
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        print("Interrupted.", file=sys.stderr)
        raise SystemExit(130)
    except LoaderError as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1)
