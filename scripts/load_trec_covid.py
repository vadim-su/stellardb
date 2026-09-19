#!/usr/bin/env python3
"""Load TREC-COVID as a research-topic-to-publication graph in StellarDB."""

from __future__ import annotations

import argparse
import csv
import json
import os
import re
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


TREC_COVID = DatasetSpec("trec-covid", "ce62140cb23feb9becf6270d0d1fe6d1")


def text_terms(text: str) -> list[str]:
    return re.findall(r"[a-z0-9]+(?:-[a-z0-9]+)?", text.lower())


def publication_rows(root: Path) -> Iterator[dict[str, Any]]:
    for source in read_jsonl(root / "corpus.jsonl"):
        source_id = str(source.get("_id") or "")
        if not source_id:
            raise LoaderError(f"A publication in {root / 'corpus.jsonl'} has no _id")
        metadata = source.get("metadata") or {}
        if not isinstance(metadata, dict):
            metadata = {}
        url = str(metadata.get("url") or "")
        domain = (urllib.parse.urlparse(url).hostname or "").lower()
        pubmed_id = str(metadata.get("pubmed_id") or "")
        abstract = str(source.get("text") or "")
        identifiers = [{"scheme": "cord_uid", "value": source_id}]
        if pubmed_id:
            identifiers.append({"scheme": "pubmed", "value": pubmed_id})
        yield {
            "id": stable_id("trec_covid", "publication", source_id),
            "source_id": source_id,
            "pubmed_id": pubmed_id,
            "title": str(source.get("title") or ""),
            "abstract": abstract,
            "url": url,
            "source_domain": domain,
            "identifiers": identifiers,
            "source": {"url": url, "domain": domain},
            "content_stats": {
                "characters": len(abstract),
                "words": len(text_terms(abstract)),
            },
        }


def topic_rows(root: Path) -> Iterator[dict[str, Any]]:
    for source in read_jsonl(root / "queries.jsonl"):
        source_id = str(source.get("_id") or "")
        if not source_id:
            raise LoaderError(f"A topic in {root / 'queries.jsonl'} has no _id")
        metadata = source.get("metadata") or {}
        if not isinstance(metadata, dict):
            metadata = {}
        question = str(source.get("text") or "")
        keywords = str(metadata.get("query") or "")
        narrative = str(metadata.get("narrative") or "")
        yield {
            "id": stable_id("trec_covid", "topic", source_id),
            "source_id": source_id,
            "question": question,
            "keywords": keywords,
            "narrative": narrative,
            "keyword_terms": text_terms(keywords),
            "formulations": [
                {"kind": "question", "text": question},
                {"kind": "keywords", "text": keywords},
                {"kind": "narrative", "text": narrative},
            ],
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
                raise LoaderError(f"Unexpected TREC-COVID qrels header in {path}: {reader.fieldnames}")
            for line_number, row in enumerate(reader, 2):
                topic_id = str(row["query-id"])
                publication_id = str(row["corpus-id"])
                try:
                    grade = int(row["score"])
                except ValueError as error:
                    raise LoaderError(
                        f"Invalid relevance grade in {path}:{line_number}: {row['score']!r}"
                    ) from error
                identity = f"{split}\0{topic_id}\0{publication_id}"
                yield {
                    "id": stable_id("trec_covid", "assessment", identity),
                    "split": split,
                    "topic_id": topic_id,
                    "topic_record_id": stable_id("trec_covid", "topic", topic_id),
                    "publication_id": publication_id,
                    "publication_record_id": stable_id(
                        "trec_covid", "publication", publication_id
                    ),
                    "grade": grade,
                    "benchmark": {"name": "trec-covid", "split": split},
                }
    if splits is not None:
        missing = splits - found_splits
        if missing:
            raise LoaderError(
                f"TREC-COVID does not contain qrels split(s): {', '.join(sorted(missing))}"
            )


def grade_rows(root: Path, splits: set[str] | None) -> Iterator[dict[str, Any]]:
    for grade in sorted({int(row["grade"]) for row in assessment_rows(root, splits)}):
        yield {"id": f"grade_{grade}" if grade >= 0 else f"grade_neg_{abs(grade)}", "value": grade}


def grade_record_id(grade: int) -> str:
    return f"grade_{grade}" if grade >= 0 else f"grade_neg_{abs(grade)}"


def split_rows(root: Path, splits: set[str] | None) -> Iterator[dict[str, Any]]:
    for path in sorted((root / "qrels").glob("*.tsv")):
        if splits is None or path.stem in splits:
            yield {"id": path.stem, "name": path.stem}


def source_site_rows(root: Path) -> Iterator[dict[str, Any]]:
    domains = {row["source_domain"] for row in publication_rows(root) if row["source_domain"]}
    for domain in sorted(domains):
        yield {
            "id": stable_id("trec_covid", "site", str(domain)),
            "domain": domain,
        }


def collection_names() -> tuple[str, str, str, str, str, str]:
    return (
        "covid_publications",
        "covid_research_topics",
        "covid_relevance_assessments",
        "covid_relevance_grades",
        "covid_source_sites",
        "covid_benchmark_splits",
    )


def create_collections(
    client: StellarClient, replace: bool
) -> tuple[str, str, str, str, str, str]:
    publications, topics, assessments, grades, sites, splits = collection_names()
    if replace:
        for collection in (assessments, grades, splits, topics, publications, sites):
            try:
                client.execute(f"DROP COLLECTION {collection} CASCADE")
            except LoaderError as error:
                if "not found" not in str(error).lower():
                    raise

    client.execute(
        f"""DEFINE COLLECTION {publications} (
            SCHEMA FLEXIBLE,
            source_id string REQUIRED,
            pubmed_id string,
            title string,
            abstract string REQUIRED,
            url string REQUIRED,
            source_domain string REQUIRED,
            identifiers array REQUIRED,
            source object REQUIRED,
            content_stats object REQUIRED
        )"""
    )
    client.execute(
        f"""DEFINE COLLECTION {topics} (
            SCHEMA FLEXIBLE,
            source_id string REQUIRED,
            question string REQUIRED,
            keywords string REQUIRED,
            narrative string REQUIRED,
            keyword_terms array REQUIRED,
            formulations array REQUIRED
        )"""
    )
    client.execute(
        f"""DEFINE COLLECTION {assessments} (
            SCHEMA FLEXIBLE,
            split string REQUIRED,
            topic_id string REQUIRED,
            topic_record_id string REQUIRED,
            publication_id string REQUIRED,
            publication_record_id string REQUIRED,
            grade int REQUIRED,
            benchmark object REQUIRED
        )"""
    )
    client.execute(f"DEFINE COLLECTION {grades} (SCHEMA FLEXIBLE, value int REQUIRED)")
    client.execute(f"DEFINE COLLECTION {sites} (SCHEMA FLEXIBLE, domain string REQUIRED)")
    client.execute(f"DEFINE COLLECTION {splits} (SCHEMA FLEXIBLE, name string REQUIRED)")
    return publications, topics, assessments, grades, sites, splits


def source_edge_statements(root: Path) -> Iterator[tuple[str, str]]:
    publications, _, _, _, sites, _ = collection_names()
    for row in publication_rows(root):
        domain = str(row["source_domain"])
        if domain:
            site_id = stable_id("trec_covid", "site", domain)
            yield (
                f"RELATE {publications}:{row['id']}->hosted_by->{sites}:{site_id} RETURN NONE",
                str(row["id"]),
            )


def assessment_edge_statements(
    root: Path, splits_filter: set[str] | None
) -> Iterator[tuple[str, str]]:
    publications, topics, assessments, grades, _, splits = collection_names()
    for row in assessment_rows(root, splits_filter):
        topic = f"{topics}:{row['topic_record_id']}"
        assessment = f"{assessments}:{row['id']}"
        publication = f"{publications}:{row['publication_record_id']}"
        grade = f"{grades}:{grade_record_id(int(row['grade']))}"
        split = f"{splits}:{row['split']}"
        identity = str(row["id"])
        yield f"RELATE {topic}->has_assessment->{assessment} RETURN NONE", identity
        yield f"RELATE {assessment}->judges->{publication} RETURN NONE", identity
        yield f"RELATE {assessment}->graded_as->{grade} RETURN NONE", identity
        yield f"RELATE {assessment}->in_split->{split} RETURN NONE", identity


def create_secondary_indexes(client: StellarClient) -> None:
    publications, topics, assessments, grades, sites, splits = collection_names()
    client.execute(f"CREATE INDEX ON {publications}(source_id) UNIQUE")
    client.execute(f"CREATE INDEX ON {publications}(pubmed_id)")
    client.execute(f"CREATE INDEX ON {topics}(source_id) UNIQUE")
    client.execute(f"CREATE INDEX ON {assessments}(topic_id, publication_id)")
    client.execute(f"CREATE INDEX ON {assessments}(split, grade)")
    client.execute(f"CREATE INDEX ON {grades}(value) UNIQUE")
    client.execute(f"CREATE INDEX ON {sites}(domain) UNIQUE")
    client.execute(f"CREATE INDEX ON {splits}(name) UNIQUE")


def create_fulltext_indexes(client: StellarClient, analyzer: str | None) -> None:
    publications, topics, _, _, _, _ = collection_names()
    analyzer_clause = f" ANALYZER {analyzer}" if analyzer else ""
    client.execute(
        f"CREATE INDEX ON {publications}(title, abstract) FULLTEXT{analyzer_clause}"
    )
    client.execute(
        f"CREATE INDEX ON {topics}(question, keywords, narrative) FULLTEXT{analyzer_clause}"
    )


def count_rows(rows: Iterable[dict[str, Any]]) -> int:
    return sum(1 for _ in rows)


def load_trec_covid(
    client: StellarClient | None,
    root: Path,
    splits_filter: set[str] | None,
    replace: bool,
    analyzer: str | None,
    batch_size: int,
    max_batch_bytes: int,
) -> dict[str, int]:
    print(f"\n[trec-covid] source: {root}")
    if client is None:
        publication_count = count_rows(publication_rows(root))
        assessment_count = count_rows(assessment_rows(root, splits_filter))
        source_edge_count = count_rows(source_edge_statements(root))
        counts = {
            "publications": publication_count,
            "topics": count_rows(topic_rows(root)),
            "assessments": assessment_count,
            "grades": count_rows(grade_rows(root, splits_filter)),
            "sites": count_rows(source_site_rows(root)),
            "splits": count_rows(split_rows(root, splits_filter)),
            "edges": source_edge_count + assessment_count * 4,
        }
        print_summary(counts, "  dry run: ")
        return counts

    publications, topics, assessments, grades, sites, splits = create_collections(client, replace)
    # Build every index while collections are empty. Backfilling a large corpus
    # in one CREATE INDEX operation can exceed the server hard deadline;
    # incremental batched inserts stay bounded.
    create_secondary_indexes(client)
    create_fulltext_indexes(client, analyzer)
    counts = {
        "publications": insert_rows(
            client, publications, publication_rows(root), batch_size, max_batch_bytes
        ),
        "topics": insert_rows(client, topics, topic_rows(root), batch_size, max_batch_bytes),
        "assessments": insert_rows(
            client,
            assessments,
            assessment_rows(root, splits_filter),
            batch_size,
            max_batch_bytes,
        ),
        "grades": insert_rows(
            client, grades, grade_rows(root, splits_filter), batch_size, max_batch_bytes
        ),
        "sites": insert_rows(client, sites, source_site_rows(root), batch_size, max_batch_bytes),
        "splits": insert_rows(
            client, splits, split_rows(root, splits_filter), batch_size, max_batch_bytes
        ),
    }
    counts["edges"] = insert_edges(
        client, source_edge_statements(root), batch_size, max_batch_bytes
    ) + insert_edges(
        client,
        assessment_edge_statements(root, splits_filter),
        batch_size,
        max_batch_bytes,
    )
    return counts


def print_summary(counts: dict[str, int], prefix: str = "  trec-covid: ") -> None:
    print(
        f"{prefix}{counts['publications']:,} publications, {counts['topics']:,} topics, "
        f"{counts['assessments']:,} assessments, {counts['grades']:,} grades, "
        f"{counts['sites']:,} sites, {counts['splits']:,} splits, "
        f"{counts['edges']:,} graph edges"
    )


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Download BEIR TREC-COVID and load its research graph into StellarDB."
    )
    parser.add_argument("--url", default="http://127.0.0.1:3000", help="StellarDB base URL")
    parser.add_argument("--database", required=True, help="Target StellarDB database name")
    parser.add_argument("--splits", nargs="+", help="Selected qrels splits (default: all)")
    parser.add_argument("--cache-dir", type=Path, default=Path(".cache/beir"))
    parser.add_argument("--token", help="Bearer token; defaults to STELLARDB_TOKEN")
    parser.add_argument("--replace", action="store_true", help="Drop and recreate collections")
    parser.add_argument("--redownload", action="store_true")
    parser.add_argument("--analyzer", help="Existing analyzer for full-text indexes")
    parser.add_argument("--batch-size", type=int, default=250)
    parser.add_argument("--max-batch-bytes", type=int, default=512 * 1024)
    parser.add_argument("--timeout", type=float, default=900)
    parser.add_argument("--query-timeout", help="Optional X-Query-Timeout, for example 2m")
    parser.add_argument("--dry-run", action="store_true")
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
    root = download_dataset(TREC_COVID, args.cache_dir, args.redownload)
    counts = load_trec_covid(
        client,
        root,
        set(args.splits) if args.splits else None,
        args.replace,
        args.analyzer,
        args.batch_size,
        args.max_batch_bytes,
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
