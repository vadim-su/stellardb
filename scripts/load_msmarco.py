#!/usr/bin/env python3
"""Load MS MARCO passages and query-to-passage judgments into StellarDB."""

from __future__ import annotations

import argparse
import csv
import json
import os
import re
import sys
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


MSMARCO = DatasetSpec("msmarco", "444067daf65d982533ea17ebd59501e4")


def text_terms(text: str) -> list[str]:
    return re.findall(r"[a-z0-9]+(?:'[a-z0-9]+)?", text.lower())


def sentence_segments(text: str) -> list[str]:
    stripped = text.strip()
    if not stripped:
        return []
    return [part for part in re.split(r"(?<=[.!?])\s+", stripped) if part]


def passage_rows(root: Path, max_passages: int | None) -> Iterator[dict[str, Any]]:
    for ordinal, source in enumerate(read_jsonl(root / "corpus.jsonl")):
        if max_passages is not None and ordinal >= max_passages:
            break
        source_id = str(source.get("_id") or "")
        if not source_id:
            raise LoaderError(f"A passage in {root / 'corpus.jsonl'} has no _id")
        text = str(source.get("text") or "")
        sentences = sentence_segments(text)
        yield {
            "id": stable_id("msmarco", "passage", source_id),
            "source_id": source_id,
            "text": text,
            "sentences": sentences,
            "text_stats": {
                "characters": len(text),
                "words": len(text_terms(text)),
                "sentences": len(sentences),
            },
        }


def selected_passage_ids(root: Path, max_passages: int | None) -> set[str] | None:
    if max_passages is None:
        return None
    return {str(row["source_id"]) for row in passage_rows(root, max_passages)}


def qrel_files(root: Path, splits: set[str] | None) -> list[Path]:
    qrels_dir = root / "qrels"
    if not qrels_dir.is_dir():
        raise LoaderError(f"No qrels directory found under {root}")
    available = {path.stem: path for path in qrels_dir.glob("*.tsv")}
    if not available:
        raise LoaderError(f"No qrels TSV files found under {qrels_dir}")
    if splits is not None:
        missing = splits - set(available)
        if missing:
            raise LoaderError(
                f"MS MARCO does not contain qrels split(s): {', '.join(sorted(missing))}"
            )
        return [available[name] for name in sorted(splits)]
    return [available[name] for name in sorted(available)]


def judgment_rows(
    root: Path,
    splits: set[str] | None,
    allowed_passage_ids: set[str] | None,
) -> Iterator[dict[str, Any]]:
    for path in qrel_files(root, splits):
        split = path.stem
        with path.open("r", encoding="utf-8", newline="") as source:
            reader = csv.DictReader(source, delimiter="\t")
            if reader.fieldnames != ["query-id", "corpus-id", "score"]:
                raise LoaderError(f"Unexpected MS MARCO qrels header in {path}: {reader.fieldnames}")
            for line_number, row in enumerate(reader, 2):
                query_id = str(row["query-id"])
                passage_id = str(row["corpus-id"])
                if allowed_passage_ids is not None and passage_id not in allowed_passage_ids:
                    continue
                try:
                    grade = int(row["score"])
                except ValueError as error:
                    raise LoaderError(
                        f"Invalid relevance grade in {path}:{line_number}: {row['score']!r}"
                    ) from error
                identity = f"{split}\0{query_id}\0{passage_id}"
                yield {
                    "id": stable_id("msmarco", "judgment", identity),
                    "split": split,
                    "query_id": query_id,
                    "query_record_id": stable_id("msmarco", "query", query_id),
                    "passage_id": passage_id,
                    "passage_record_id": stable_id("msmarco", "passage", passage_id),
                    "grade": grade,
                    "benchmark": {"name": "msmarco", "split": split},
                }


def selected_query_ids(
    root: Path,
    splits: set[str] | None,
    allowed_passage_ids: set[str] | None,
) -> set[str] | None:
    if allowed_passage_ids is None and splits is None:
        return None
    return {
        str(row["query_id"])
        for row in judgment_rows(root, splits, allowed_passage_ids)
    }


def query_rows(root: Path, allowed_query_ids: set[str] | None) -> Iterator[dict[str, Any]]:
    for source in read_jsonl(root / "queries.jsonl"):
        source_id = str(source.get("_id") or "")
        if not source_id:
            raise LoaderError(f"A query in {root / 'queries.jsonl'} has no _id")
        if allowed_query_ids is not None and source_id not in allowed_query_ids:
            continue
        text = str(source.get("text") or "")
        terms = text_terms(text)
        yield {
            "id": stable_id("msmarco", "query", source_id),
            "source_id": source_id,
            "text": text,
            "terms": terms,
            "query_analysis": {
                "characters": len(text),
                "words": len(terms),
                "is_question": text.rstrip().endswith("?"),
            },
        }


def grade_rows(
    root: Path, splits: set[str] | None, allowed_passage_ids: set[str] | None
) -> Iterator[dict[str, Any]]:
    grades = sorted(
        {
            int(row["grade"])
            for row in judgment_rows(root, splits, allowed_passage_ids)
        }
    )
    for grade in grades:
        yield {"id": f"grade_{grade}" if grade >= 0 else f"grade_neg_{abs(grade)}", "value": grade}


def grade_record_id(grade: int) -> str:
    return f"grade_{grade}" if grade >= 0 else f"grade_neg_{abs(grade)}"


def split_rows(root: Path, splits: set[str] | None) -> Iterator[dict[str, Any]]:
    for path in qrel_files(root, splits):
        yield {"id": path.stem, "name": path.stem}


def collection_names() -> tuple[str, str, str, str, str]:
    return (
        "msmarco_passages",
        "msmarco_search_queries",
        "msmarco_passage_judgments",
        "msmarco_relevance_grades",
        "msmarco_benchmark_splits",
    )


def create_collections(
    client: StellarClient, replace: bool
) -> tuple[str, str, str, str, str]:
    passages, queries, judgments, grades, splits = collection_names()
    if replace:
        for collection in (judgments, grades, splits, queries, passages):
            try:
                client.execute(f"DROP COLLECTION {collection} CASCADE")
            except LoaderError as error:
                if "not found" not in str(error).lower():
                    raise
    client.execute(
        f"""DEFINE COLLECTION {passages} (
            SCHEMA FLEXIBLE,
            source_id string REQUIRED,
            text string REQUIRED,
            sentences array REQUIRED,
            text_stats object REQUIRED
        )"""
    )
    client.execute(
        f"""DEFINE COLLECTION {queries} (
            SCHEMA FLEXIBLE,
            source_id string REQUIRED,
            text string REQUIRED,
            terms array REQUIRED,
            query_analysis object REQUIRED
        )"""
    )
    client.execute(
        f"""DEFINE COLLECTION {judgments} (
            SCHEMA FLEXIBLE,
            split string REQUIRED,
            query_id string REQUIRED,
            query_record_id string REQUIRED,
            passage_id string REQUIRED,
            passage_record_id string REQUIRED,
            grade int REQUIRED,
            benchmark object REQUIRED
        )"""
    )
    client.execute(f"DEFINE COLLECTION {grades} (SCHEMA FLEXIBLE, value int REQUIRED)")
    client.execute(f"DEFINE COLLECTION {splits} (SCHEMA FLEXIBLE, name string REQUIRED)")
    return passages, queries, judgments, grades, splits


def judgment_edge_statements(
    root: Path,
    splits_filter: set[str] | None,
    allowed_passage_ids: set[str] | None,
) -> Iterator[tuple[str, str]]:
    passages, queries, judgments, grades, splits = collection_names()
    for row in judgment_rows(root, splits_filter, allowed_passage_ids):
        query = f"{queries}:{row['query_record_id']}"
        judgment = f"{judgments}:{row['id']}"
        passage = f"{passages}:{row['passage_record_id']}"
        grade = f"{grades}:{grade_record_id(int(row['grade']))}"
        split = f"{splits}:{row['split']}"
        identity = str(row["id"])
        edge_data = json.dumps(
            {"split": row["split"], "grade": row["grade"]},
            ensure_ascii=False,
            separators=(",", ":"),
        )
        yield f"RELATE {query}->has_passage_judgment->{judgment} RETURN NONE", identity
        yield f"RELATE {judgment}->selects->{passage} RETURN NONE", identity
        yield f"RELATE {judgment}->graded_as->{grade} RETURN NONE", identity
        yield f"RELATE {judgment}->in_split->{split} RETURN NONE", identity
        yield (
            f"RELATE {query}->answer_found_in->{passage} CONTENT {edge_data} RETURN NONE",
            identity,
        )


def create_secondary_indexes(client: StellarClient) -> None:
    passages, queries, judgments, grades, splits = collection_names()
    client.execute(f"CREATE INDEX ON {passages}(source_id) UNIQUE")
    client.execute(f"CREATE INDEX ON {queries}(source_id) UNIQUE")
    client.execute(f"CREATE INDEX ON {judgments}(query_id, passage_id)")
    client.execute(f"CREATE INDEX ON {judgments}(split, grade)")
    client.execute(f"CREATE INDEX ON {grades}(value) UNIQUE")
    client.execute(f"CREATE INDEX ON {splits}(name) UNIQUE")


def create_fulltext_indexes(client: StellarClient, analyzer: str | None) -> None:
    passages, queries, _, _, _ = collection_names()
    analyzer_clause = f" ANALYZER {analyzer}" if analyzer else ""
    client.execute(f"CREATE INDEX ON {passages}(text) FULLTEXT{analyzer_clause}")
    client.execute(f"CREATE INDEX ON {queries}(text) FULLTEXT{analyzer_clause}")


def count_rows(rows: Iterable[dict[str, Any]]) -> int:
    return sum(1 for _ in rows)


def load_msmarco(
    client: StellarClient | None,
    root: Path,
    splits_filter: set[str] | None,
    max_passages: int | None,
    replace: bool,
    analyzer: str | None,
    batch_size: int,
    max_batch_bytes: int,
) -> dict[str, int]:
    print(f"\n[msmarco] source: {root}")
    allowed_passages = selected_passage_ids(root, max_passages)
    allowed_queries = selected_query_ids(root, splits_filter, allowed_passages)
    if max_passages is not None:
        print(
            f"  consistent prefix subset: {len(allowed_passages or ()):,} passages; "
            "qrels and queries without a retained passage are omitted"
        )

    if client is None:
        passage_count = count_rows(passage_rows(root, max_passages))
        judgment_count = count_rows(
            judgment_rows(root, splits_filter, allowed_passages)
        )
        counts = {
            "passages": passage_count,
            "queries": count_rows(query_rows(root, allowed_queries)),
            "judgments": judgment_count,
            "grades": count_rows(grade_rows(root, splits_filter, allowed_passages)),
            "splits": count_rows(split_rows(root, splits_filter)),
            "edges": judgment_count * 5,
        }
        print_summary(counts, "  dry run: ")
        return counts

    passages, queries, judgments, grades, splits = create_collections(client, replace)
    # MS MARCO is too large for any single index backfill operation. Define all
    # indexes while the collections are empty so each bounded INSERT updates
    # them incrementally and the finalization step cannot hit the server's hard
    # database-operation deadline.
    create_secondary_indexes(client)
    create_fulltext_indexes(client, analyzer)
    counts = {
        "passages": insert_rows(
            client,
            passages,
            passage_rows(root, max_passages),
            batch_size,
            max_batch_bytes,
        ),
        "queries": insert_rows(
            client, queries, query_rows(root, allowed_queries), batch_size, max_batch_bytes
        ),
        "judgments": insert_rows(
            client,
            judgments,
            judgment_rows(root, splits_filter, allowed_passages),
            batch_size,
            max_batch_bytes,
        ),
        "grades": insert_rows(
            client,
            grades,
            grade_rows(root, splits_filter, allowed_passages),
            batch_size,
            max_batch_bytes,
        ),
        "splits": insert_rows(
            client, splits, split_rows(root, splits_filter), batch_size, max_batch_bytes
        ),
    }
    counts["edges"] = insert_edges(
        client,
        judgment_edge_statements(root, splits_filter, allowed_passages),
        batch_size,
        max_batch_bytes,
    )
    return counts


def print_summary(counts: dict[str, int], prefix: str = "  msmarco: ") -> None:
    print(
        f"{prefix}{counts['passages']:,} passages, {counts['queries']:,} queries, "
        f"{counts['judgments']:,} judgments, {counts['grades']:,} grades, "
        f"{counts['splits']:,} splits, {counts['edges']:,} graph edges"
    )


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Download BEIR MS MARCO and load its passage-ranking graph into StellarDB."
    )
    parser.add_argument("--url", default="http://127.0.0.1:3000", help="StellarDB base URL")
    parser.add_argument("--database", required=True, help="Target StellarDB database name")
    parser.add_argument("--splits", nargs="+", help="Selected qrels splits (default: all)")
    parser.add_argument(
        "--max-passages",
        type=int,
        help=(
            "Load only the first N corpus passages and retain only qrels/queries connected "
            "to them; useful for a smaller but graph-consistent demo"
        ),
    )
    parser.add_argument("--cache-dir", type=Path, default=Path(".cache/beir"))
    parser.add_argument("--token", help="Bearer token; defaults to STELLARDB_TOKEN")
    parser.add_argument("--replace", action="store_true", help="Drop and recreate collections")
    parser.add_argument("--redownload", action="store_true")
    parser.add_argument("--analyzer", help="Existing analyzer for full-text indexes")
    parser.add_argument("--batch-size", type=int, default=500)
    parser.add_argument("--max-batch-bytes", type=int, default=1024 * 1024)
    parser.add_argument("--timeout", type=float, default=900)
    parser.add_argument("--query-timeout", help="Optional X-Query-Timeout, for example 10m")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args(argv)
    if args.batch_size < 1:
        parser.error("--batch-size must be at least 1")
    if args.max_batch_bytes < 1024:
        parser.error("--max-batch-bytes must be at least 1024")
    if args.max_passages is not None and args.max_passages < 1:
        parser.error("--max-passages must be at least 1")
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
    root = download_dataset(MSMARCO, args.cache_dir, args.redownload)
    counts = load_msmarco(
        client,
        root,
        set(args.splits) if args.splits else None,
        args.max_passages,
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
