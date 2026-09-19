#!/usr/bin/env python3
"""Load NFCorpus as a health-topic-to-medical-literature graph in StellarDB.

The loader uses the official BEIR archive and only Python's standard library.
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import sys
import urllib.parse
from pathlib import Path
from typing import Any, Iterable, Iterator, Sequence

from load_scifact import (
    DatasetSpec,
    LoaderError,
    StellarClient,
    download_dataset,
    insert_edges,
    insert_rows,
    read_jsonl,
    stable_id,
    verify_server_compatibility,
)


NFCORPUS = DatasetSpec("nfcorpus", "a89dba18a62ef92f7d323ec890a0d38d")


def metadata_url(source: dict[str, Any]) -> str:
    metadata = source.get("metadata") or {}
    return str(metadata.get("url") or "") if isinstance(metadata, dict) else ""


def url_domain(url: str) -> str:
    return (urllib.parse.urlparse(url).hostname or "").lower()


def pubmed_id(url: str) -> str:
    parts = [part for part in urllib.parse.urlparse(url).path.split("/") if part]
    if "pubmed" not in parts:
        return ""
    position = parts.index("pubmed") + 1
    return parts[position] if position < len(parts) else ""


def article_rows(root: Path) -> Iterator[dict[str, Any]]:
    for source in read_jsonl(root / "corpus.jsonl"):
        source_id = str(source.get("_id") or "")
        if not source_id:
            raise LoaderError(f"A medical article in {root / 'corpus.jsonl'} has no _id")
        url = metadata_url(source)
        yield {
            "id": stable_id("nfcorpus", "article", source_id),
            "source_id": source_id,
            "pubmed_id": pubmed_id(url),
            "title": str(source.get("title") or ""),
            "abstract": str(source.get("text") or ""),
            "url": url,
            "source_domain": url_domain(url),
        }


def topic_rows(root: Path) -> Iterator[dict[str, Any]]:
    for source in read_jsonl(root / "queries.jsonl"):
        source_id = str(source.get("_id") or "")
        if not source_id:
            raise LoaderError(f"A health topic in {root / 'queries.jsonl'} has no _id")
        url = metadata_url(source)
        yield {
            "id": stable_id("nfcorpus", "topic", source_id),
            "source_id": source_id,
            "text": str(source.get("text") or ""),
            "url": url,
            "source_domain": url_domain(url),
        }


def assessment_rows(root: Path, splits: set[str] | None) -> Iterator[dict[str, Any]]:
    qrels_dir = root / "qrels"
    if not qrels_dir.is_dir():
        raise LoaderError(f"No qrels directory found under {root}")

    found_splits: set[str] = set()
    for path in sorted(qrels_dir.glob("*.tsv")):
        split = path.stem
        if splits is not None and split not in splits:
            continue
        found_splits.add(split)
        with path.open("r", encoding="utf-8", newline="") as source:
            reader = csv.DictReader(source, delimiter="\t")
            if reader.fieldnames != ["query-id", "corpus-id", "score"]:
                raise LoaderError(f"Unexpected NFCorpus qrels header in {path}: {reader.fieldnames}")
            for line_number, row in enumerate(reader, 2):
                topic_id = str(row["query-id"])
                article_id = str(row["corpus-id"])
                try:
                    grade = int(row["score"])
                except ValueError as error:
                    raise LoaderError(
                        f"Invalid relevance grade in {path}:{line_number}: {row['score']!r}"
                    ) from error
                identity = f"{split}\0{topic_id}\0{article_id}"
                yield {
                    "id": stable_id("nfcorpus", "assessment", identity),
                    "split": split,
                    "topic_id": topic_id,
                    "topic_record_id": stable_id("nfcorpus", "topic", topic_id),
                    "article_id": article_id,
                    "article_record_id": stable_id("nfcorpus", "article", article_id),
                    "grade": grade,
                }

    if splits is not None:
        missing = splits - found_splits
        if missing:
            raise LoaderError(
                f"NFCorpus does not contain qrels split(s): {', '.join(sorted(missing))}"
            )


def grade_rows(root: Path, splits: set[str] | None) -> Iterator[dict[str, Any]]:
    grades = sorted({int(row["grade"]) for row in assessment_rows(root, splits)})
    for grade in grades:
        yield {"id": f"grade_{grade}", "value": grade}


def split_rows(root: Path, splits: set[str] | None) -> Iterator[dict[str, Any]]:
    for path in sorted((root / "qrels").glob("*.tsv")):
        split = path.stem
        if splits is None or split in splits:
            yield {"id": split, "name": split}


def source_site_rows(root: Path) -> Iterator[dict[str, Any]]:
    domains = {
        row["source_domain"]
        for row in [*article_rows(root), *topic_rows(root)]
        if row["source_domain"]
    }
    for domain in sorted(domains):
        yield {
            "id": stable_id("nfcorpus", "site", domain),
            "domain": domain,
        }


def collection_names() -> tuple[str, str, str, str, str, str]:
    return (
        "medical_articles",
        "health_topics",
        "relevance_assessments",
        "relevance_grades",
        "source_sites",
        "benchmark_splits",
    )


def create_collections(
    client: StellarClient, replace: bool
) -> tuple[str, str, str, str, str, str]:
    articles, topics, assessments, grades, sites, splits = collection_names()
    if replace:
        for collection in (assessments, grades, splits, topics, articles, sites):
            try:
                client.execute(f"DROP COLLECTION {collection} CASCADE")
            except LoaderError as error:
                if "not found" not in str(error).lower():
                    raise

    client.execute(
        f"""DEFINE COLLECTION {articles} (
            SCHEMA FLEXIBLE,
            source_id string REQUIRED,
            pubmed_id string,
            title string,
            abstract string REQUIRED,
            url string REQUIRED,
            source_domain string REQUIRED
        )"""
    )
    client.execute(
        f"""DEFINE COLLECTION {topics} (
            SCHEMA FLEXIBLE,
            source_id string REQUIRED,
            text string REQUIRED,
            url string REQUIRED,
            source_domain string REQUIRED
        )"""
    )
    client.execute(
        f"""DEFINE COLLECTION {assessments} (
            SCHEMA FLEXIBLE,
            split string REQUIRED,
            topic_id string REQUIRED,
            topic_record_id string REQUIRED,
            article_id string REQUIRED,
            article_record_id string REQUIRED,
            grade int REQUIRED
        )"""
    )
    client.execute(
        f"""DEFINE COLLECTION {grades} (
            SCHEMA FLEXIBLE,
            value int REQUIRED
        )"""
    )
    client.execute(
        f"""DEFINE COLLECTION {sites} (
            SCHEMA FLEXIBLE,
            domain string REQUIRED
        )"""
    )
    client.execute(
        f"""DEFINE COLLECTION {splits} (
            SCHEMA FLEXIBLE,
            name string REQUIRED
        )"""
    )
    return articles, topics, assessments, grades, sites, splits


def site_edge_statements(root: Path) -> Iterator[tuple[str, str]]:
    articles, topics, _, _, sites, _ = collection_names()
    for collection, rows in ((articles, article_rows(root)), (topics, topic_rows(root))):
        for row in rows:
            domain = str(row["source_domain"])
            if domain:
                site_id = stable_id("nfcorpus", "site", domain)
                yield (
                    f"RELATE {collection}:{row['id']}->hosted_by->{sites}:{site_id} RETURN NONE",
                    str(row["id"]),
                )


def assessment_edge_statements(
    root: Path, splits_filter: set[str] | None
) -> Iterator[tuple[str, str]]:
    articles, topics, assessments, grades, _, splits = collection_names()
    for row in assessment_rows(root, splits_filter):
        topic = f"{topics}:{row['topic_record_id']}"
        assessment = f"{assessments}:{row['id']}"
        article = f"{articles}:{row['article_record_id']}"
        grade = f"{grades}:grade_{row['grade']}"
        split = f"{splits}:{row['split']}"
        identity = str(row["id"])
        yield f"RELATE {topic}->has_assessment->{assessment} RETURN NONE", identity
        yield f"RELATE {assessment}->evaluates->{article} RETURN NONE", identity
        yield f"RELATE {assessment}->graded_as->{grade} RETURN NONE", identity
        yield f"RELATE {assessment}->in_split->{split} RETURN NONE", identity


def create_indexes(client: StellarClient, analyzer: str | None) -> None:
    articles, topics, assessments, grades, sites, splits = collection_names()
    analyzer_clause = f" ANALYZER {analyzer}" if analyzer else ""
    client.execute(f"CREATE INDEX ON {articles}(source_id) UNIQUE")
    client.execute(f"CREATE INDEX ON {articles}(pubmed_id)")
    client.execute(f"CREATE INDEX ON {articles}(title, abstract) FULLTEXT{analyzer_clause}")
    client.execute(f"CREATE INDEX ON {topics}(source_id) UNIQUE")
    client.execute(f"CREATE INDEX ON {topics}(text) FULLTEXT{analyzer_clause}")
    client.execute(f"CREATE INDEX ON {assessments}(topic_id, article_id)")
    client.execute(f"CREATE INDEX ON {assessments}(split, grade)")
    client.execute(f"CREATE INDEX ON {grades}(value) UNIQUE")
    client.execute(f"CREATE INDEX ON {sites}(domain) UNIQUE")
    client.execute(f"CREATE INDEX ON {splits}(name) UNIQUE")


def count_rows(rows: Iterable[dict[str, Any]]) -> int:
    return sum(1 for _ in rows)


def load_nfcorpus(
    client: StellarClient | None,
    root: Path,
    splits_filter: set[str] | None,
    replace: bool,
    analyzer: str | None,
    batch_size: int,
    max_batch_bytes: int,
) -> dict[str, int]:
    print(f"\n[nfcorpus] source: {root}")
    assessment_count = count_rows(assessment_rows(root, splits_filter))
    article_count = count_rows(article_rows(root))
    topic_count = count_rows(topic_rows(root))
    counts = {
        "medical_articles": article_count,
        "health_topics": topic_count,
        "relevance_assessments": assessment_count,
        "relevance_grades": count_rows(grade_rows(root, splits_filter)),
        "source_sites": count_rows(source_site_rows(root)),
        "benchmark_splits": count_rows(split_rows(root, splits_filter)),
        "edges": article_count + topic_count + assessment_count * 4,
    }
    if client is None:
        print_summary(counts, prefix="  dry run: ")
        return counts

    articles, topics, assessments, grades, sites, splits = create_collections(client, replace)
    counts["medical_articles"] = insert_rows(
        client, articles, article_rows(root), batch_size, max_batch_bytes
    )
    counts["health_topics"] = insert_rows(
        client, topics, topic_rows(root), batch_size, max_batch_bytes
    )
    counts["relevance_assessments"] = insert_rows(
        client, assessments, assessment_rows(root, splits_filter), batch_size, max_batch_bytes
    )
    counts["relevance_grades"] = insert_rows(
        client, grades, grade_rows(root, splits_filter), batch_size, max_batch_bytes
    )
    counts["source_sites"] = insert_rows(
        client, sites, source_site_rows(root), batch_size, max_batch_bytes
    )
    counts["benchmark_splits"] = insert_rows(
        client, splits, split_rows(root, splits_filter), batch_size, max_batch_bytes
    )

    site_edges = insert_edges(
        client, site_edge_statements(root), batch_size, max_batch_bytes
    )
    assessment_edges = insert_edges(
        client,
        assessment_edge_statements(root, splits_filter),
        batch_size,
        max_batch_bytes,
    )
    counts["edges"] = site_edges + assessment_edges
    print("  nfcorpus: creating indexes")
    create_indexes(client, analyzer)
    return counts


def print_summary(counts: dict[str, int], prefix: str = "  nfcorpus: ") -> None:
    print(
        f"{prefix}{counts['medical_articles']:,} medical articles, "
        f"{counts['health_topics']:,} health topics, "
        f"{counts['relevance_assessments']:,} relevance assessments, "
        f"{counts['relevance_grades']:,} relevance grades, "
        f"{counts['source_sites']:,} source sites, "
        f"{counts['benchmark_splits']:,} splits, {counts['edges']:,} graph edges"
    )


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Download NFCorpus from BEIR and load its health-topic-to-medical-literature "
            "graph into StellarDB."
        )
    )
    parser.add_argument("--url", default="http://127.0.0.1:3000", help="StellarDB base URL")
    parser.add_argument("--database", required=True, help="Target StellarDB database name")
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
        "--analyzer", help="Optional existing StellarDB analyzer for the full-text indexes"
    )
    parser.add_argument(
        "--batch-size",
        type=int,
        default=250,
        help="Maximum rows or edge statements per request (default: 250)",
    )
    parser.add_argument(
        "--max-batch-bytes",
        type=int,
        default=512 * 1024,
        help="Approximate maximum serialized bytes per request",
    )
    parser.add_argument("--timeout", type=float, default=900, help="HTTP timeout in seconds")
    parser.add_argument(
        "--query-timeout",
        help="Optional X-Query-Timeout value sent to StellarDB, for example 2m",
    )
    parser.add_argument("--dry-run", action="store_true", help="Download, validate, and count only")
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

    client = None
    if not args.dry_run:
        client = StellarClient(
            args.url, args.database, args.token, args.timeout, args.query_timeout
        )
        client.ensure_database()
        print(f"Target: {args.url.rstrip('/')} database={args.database}")
        verify_server_compatibility(client)

    root = download_dataset(NFCORPUS, args.cache_dir, args.redownload)
    counts = load_nfcorpus(
        client=client,
        root=root,
        splits_filter=set(args.splits) if args.splits else None,
        replace=args.replace,
        analyzer=args.analyzer,
        batch_size=args.batch_size,
        max_batch_bytes=args.max_batch_bytes,
    )
    print("\nCompleted:")
    print_summary(counts)
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
