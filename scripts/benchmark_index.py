#!/usr/bin/env python3
"""Comprehensive benchmark for secondary index performance."""

import argparse
import random
import string
import time
from dataclasses import dataclass
from typing import List, Tuple

import requests

BASE_URL = "http://127.0.0.1:3000"
DATABASE = "benchmark"
DOCS = 100_000
BATCH_SIZE = 100
QUERIES_PER_TEST = 50


@dataclass
class BenchmarkResult:
    name: str
    description: str
    before_ms: float
    after_ms: float
    uses_index: bool
    status: str  # "PASS", "FAIL", "TODO"

    @property
    def speedup(self) -> str:
        if self.after_ms > 0:
            ratio = self.before_ms / self.after_ms
            if ratio >= 1.5:
                return f"{ratio:.1f}x faster"
            elif ratio <= 0.67:
                return f"{1/ratio:.1f}x SLOWER"
            else:
                return "~same"
        return "N/A"


def random_string(length=10):
    return "".join(random.choices(string.ascii_lowercase, k=length))


def ensure_database():
    """Create the benchmark database if it does not exist."""
    response = requests.post(
        f"{BASE_URL}/databases",
        json={"name": DATABASE},
        timeout=60,
    )
    if response.status_code not in (201, 409):
        raise Exception(f"HTTP {response.status_code}: {response.text[:200]}")


def sql(query):
    """Execute SQL and return (result, time_ms)."""
    start = time.perf_counter()
    r = requests.post(
        f"{BASE_URL}/sql",
        headers={"X-Database": DATABASE},
        json={"query": query},
        timeout=60,
    )
    elapsed_ms = (time.perf_counter() - start) * 1000

    if r.status_code != 200:
        raise Exception(f"HTTP {r.status_code}: {r.text[:200]}")
    body = r.json()
    if body.get("error"):
        raise Exception(f"SQL error: {body['error']}")
    return body.get("data", body), elapsed_ms


def run_queries(queries: List[str]) -> Tuple[float, float, float]:
    """Run queries and return (avg_ms, p50_ms, p99_ms)."""
    times = []
    for q in queries:
        _, ms = sql(q)
        times.append(ms)

    times.sort()
    avg = sum(times) / len(times)
    p50 = times[len(times) // 2]
    p99 = times[int(len(times) * 0.99)]
    return avg, p50, p99


class Benchmark:
    def __init__(self):
        self.all_names: List[str] = []
        self.all_ages: List[int] = []
        self.all_scores: List[float] = []
        self.results: List[BenchmarkResult] = []

    def setup(self):
        """Create collection and load data."""
        print("=" * 70)
        print("SECONDARY INDEX BENCHMARK")
        print("=" * 70)
        print()
        print(f"Database: {DATABASE}")
        print()

        # Clean up
        try:
            sql("DROP COLLECTION benchmark CASCADE")
        except:
            pass

        print(f"Loading {DOCS:,} documents...")

        sql("""
            DEFINE COLLECTION benchmark (
                SCHEMA FLEXIBLE,
                name string,
                age int,
                city string,
                score float,
                status string
            )
        """)

        cities = ["Moscow", "Berlin", "Tokyo", "NYC", "London", "Paris", "Sydney", "Dubai"]
        statuses = ["active", "inactive", "pending"]

        inserted = 0
        start = time.time()

        for batch_start in range(0, DOCS, BATCH_SIZE):
            objects = []
            for i in range(min(BATCH_SIZE, DOCS - batch_start)):
                name = random_string(8)
                age = random.randint(18, 80)
                city = random.choice(cities)
                score = round(random.uniform(0, 100), 2)
                status = random.choice(statuses)

                self.all_names.append(name)
                self.all_ages.append(age)
                self.all_scores.append(score)

                objects.append(
                    f"{{name: '{name}', age: {age}, city: '{city}', score: {score}, status: '{status}'}}"
                )

            sql(f"INSERT INTO benchmark {', '.join(objects)}")
            inserted += len(objects)

            if inserted % 20000 == 0:
                elapsed = time.time() - start
                rate = inserted / elapsed
                print(f"  {inserted:,} docs ({rate:,.0f} docs/sec)")

        elapsed = time.time() - start
        print(f"Loaded {inserted:,} docs in {elapsed:.1f}s ({inserted/elapsed:,.0f} docs/sec)")

        # Insert documents with NULL fields for IS NULL / IS NOT NULL benchmarks
        null_count = DOCS // 10  # 10% with nulls
        print(f"Inserting {null_count:,} documents with NULL fields...")
        null_inserted = 0
        for batch_start in range(0, null_count, BATCH_SIZE):
            objects = []
            for i in range(min(BATCH_SIZE, null_count - batch_start)):
                age = random.randint(18, 80)
                # name is NULL, score is NULL
                objects.append(
                    f"{{name: NULL, age: {age}, city: 'Unknown', score: NULL, status: 'inactive'}}"
                )
            sql(f"INSERT INTO benchmark {', '.join(objects)}")
            null_inserted += len(objects)

        print(f"Inserted {null_inserted:,} docs with NULL name/score")
        print()

    def create_indexes(self):
        """Create all indexes."""
        print("Creating indexes...")

        indexes = [
            ("name", False),
            ("age", False),
            ("score", False),
            ("status", False),
            # Compound index
            ("status, age", False),
        ]

        for fields, unique in indexes:
            unique_str = " UNIQUE" if unique else ""
            try:
                _, ms = sql(f"CREATE INDEX ON benchmark({fields}){unique_str}")
                print(f"  - ({fields}){unique_str}: {ms:.0f}ms")
            except Exception as e:
                print(f"  - ({fields}): FAILED - {e}")

        print()

    def run_test(
        self,
        name: str,
        description: str,
        queries: List[str],
        uses_index: bool,
        status: str = "TODO",
    ):
        """Run a single benchmark test before and after index creation."""
        # This will be called twice - once before indexes, once after
        avg, p50, p99 = run_queries(queries)
        return avg

    def benchmark_equality_indexed_exists(self) -> BenchmarkResult:
        """Equality on indexed field, document exists."""
        test_names = random.sample(self.all_names, QUERIES_PER_TEST)
        queries = [f"SELECT * FROM benchmark WHERE name = '{n}'" for n in test_names]

        before = self.run_test("eq_indexed_exists", "", queries, True)
        self.create_indexes()
        after = self.run_test("eq_indexed_exists", "", queries, True)

        return BenchmarkResult(
            name="Equality (indexed, exists)",
            description="WHERE name = 'existing_value'",
            before_ms=before,
            after_ms=after,
            uses_index=True,
            status="PASS" if after < before * 0.5 else "FAIL",
        )

    def run_all(self):
        """Run all benchmark tests."""
        self.setup()

        # Store queries for before/after comparison
        test_cases = []

        # 1. Equality on indexed field (exists)
        test_names = random.sample(self.all_names, QUERIES_PER_TEST)
        test_cases.append({
            "name": "Equality (indexed, exists)",
            "desc": "WHERE name = 'existing'",
            "queries": [f"SELECT * FROM benchmark WHERE name = '{n}'" for n in test_names],
            "uses_index": True,
            "expected": "PASS",
        })

        # 2. Equality on indexed field (not exists)
        test_cases.append({
            "name": "Equality (indexed, not exists)",
            "desc": "WHERE name = 'nonexistent'",
            "queries": [f"SELECT * FROM benchmark WHERE name = 'NOTEXIST{i}'" for i in range(QUERIES_PER_TEST)],
            "uses_index": True,
            "expected": "PASS",
        })

        # 3. Equality on non-indexed field
        test_cases.append({
            "name": "Equality (no index)",
            "desc": "WHERE city = 'Moscow'",
            "queries": [f"SELECT * FROM benchmark WHERE city = '{c}'"
                       for c in ["Moscow", "Berlin", "Tokyo"] * (QUERIES_PER_TEST // 3 + 1)][:QUERIES_PER_TEST],
            "uses_index": False,
            "expected": "SAME",
        })

        # 4. Range > (greater than)
        test_ages = random.sample(self.all_ages, QUERIES_PER_TEST)
        test_cases.append({
            "name": "Range > (indexed)",
            "desc": "WHERE age > N",
            "queries": [f"SELECT * FROM benchmark WHERE age > {a}" for a in test_ages],
            "uses_index": True,
            "expected": "PASS",
        })

        # 5. Range >= (greater or equal)
        test_cases.append({
            "name": "Range >= (indexed)",
            "desc": "WHERE age >= N",
            "queries": [f"SELECT * FROM benchmark WHERE age >= {a}" for a in test_ages],
            "uses_index": True,
            "expected": "PASS",
        })

        # 6. Range < (less than)
        test_cases.append({
            "name": "Range < (indexed)",
            "desc": "WHERE score < N",
            "queries": [f"SELECT * FROM benchmark WHERE score < {s}" for s in random.sample(self.all_scores, QUERIES_PER_TEST)],
            "uses_index": True,
            "expected": "PASS",
        })

        # 7. Range <= (less or equal)
        test_cases.append({
            "name": "Range <= (indexed)",
            "desc": "WHERE score <= N",
            "queries": [f"SELECT * FROM benchmark WHERE score <= {s}" for s in random.sample(self.all_scores, QUERIES_PER_TEST)],
            "uses_index": True,
            "expected": "PASS",
        })

        # 8. Range BETWEEN (two conditions)
        test_cases.append({
            "name": "Range BETWEEN (indexed)",
            "desc": "WHERE age >= A AND age <= B",
            "queries": [f"SELECT * FROM benchmark WHERE age >= {a} AND age <= {a + 10}" for a in test_ages],
            "uses_index": True,
            "expected": "PASS",
        })

        # 9. AND with equality (indexed + indexed)
        test_cases.append({
            "name": "AND: eq + eq (both indexed)",
            "desc": "WHERE name = X AND status = Y",
            "queries": [f"SELECT * FROM benchmark WHERE name = '{n}' AND status = 'active'" for n in test_names],
            "uses_index": True,
            "expected": "PASS",  # Should use one index at least
        })

        # 10. AND with equality + range
        test_cases.append({
            "name": "AND: eq + range (indexed)",
            "desc": "WHERE status = X AND age > Y",
            "queries": [f"SELECT * FROM benchmark WHERE status = 'active' AND age > {a}" for a in test_ages],
            "uses_index": True,
            "expected": "PASS",
        })

        # 11. OR conditions - now uses index scans (union optimization)
        test_cases.append({
            "name": "OR conditions (indexed)",
            "desc": "WHERE name = X OR name = Y",
            "queries": [f"SELECT * FROM benchmark WHERE name = '{test_names[i]}' OR name = '{test_names[(i+1) % len(test_names)]}'"
                       for i in range(QUERIES_PER_TEST)],
            "uses_index": True,  # OR now uses index scans with union optimization
            "expected": "PASS",
        })

        # 12. High selectivity (few results)
        unique_names = random.sample(self.all_names, QUERIES_PER_TEST)
        test_cases.append({
            "name": "High selectivity (1 result)",
            "desc": "WHERE name = unique_value",
            "queries": [f"SELECT * FROM benchmark WHERE name = '{n}'" for n in unique_names],
            "uses_index": True,
            "expected": "PASS",
        })

        # 13. Low selectivity (many results)
        test_cases.append({
            "name": "Low selectivity (many results)",
            "desc": "WHERE status = 'active' (~33%)",
            "queries": [f"SELECT * FROM benchmark WHERE status = 'active'" for _ in range(QUERIES_PER_TEST)],
            "uses_index": True,
            "expected": "PASS",
        })

        # 14. != (not equal) - cannot use index
        test_cases.append({
            "name": "Not equal (!= indexed)",
            "desc": "WHERE status != 'active'",
            "queries": [f"SELECT * FROM benchmark WHERE status != 'active'" for _ in range(QUERIES_PER_TEST)],
            "uses_index": False,
            "expected": "SAME",
        })

        # 15. Compound index (status, age) - both equalities
        test_cases.append({
            "name": "Compound index (2 eq)",
            "desc": "WHERE status = X AND age = Y",
            "queries": [f"SELECT * FROM benchmark WHERE status = 'active' AND age = {a}" for a in test_ages],
            "uses_index": True,
            "expected": "PASS",  # Compound index should be much faster
        })

        # 16. IS NULL on indexed field (full scan — index doesn't cover NULLs)
        test_cases.append({
            "name": "IS NULL (indexed field)",
            "desc": "WHERE name IS NULL",
            "queries": [f"SELECT * FROM benchmark WHERE name IS NULL" for _ in range(QUERIES_PER_TEST)],
            "uses_index": False,
            "expected": "SAME",
        })

        # 17. IS NOT NULL on indexed field (full scan)
        test_cases.append({
            "name": "IS NOT NULL (indexed field)",
            "desc": "WHERE name IS NOT NULL",
            "queries": [f"SELECT * FROM benchmark WHERE name IS NOT NULL" for _ in range(QUERIES_PER_TEST)],
            "uses_index": False,
            "expected": "SAME",
        })

        # 18. IS NULL on non-indexed field
        test_cases.append({
            "name": "IS NULL (non-indexed field)",
            "desc": "WHERE score IS NULL",
            "queries": [f"SELECT * FROM benchmark WHERE score IS NULL" for _ in range(QUERIES_PER_TEST)],
            "uses_index": False,
            "expected": "SAME",
        })

        # 19. IS NOT NULL on non-indexed field
        test_cases.append({
            "name": "IS NOT NULL (non-indexed)",
            "desc": "WHERE score IS NOT NULL",
            "queries": [f"SELECT * FROM benchmark WHERE score IS NOT NULL" for _ in range(QUERIES_PER_TEST)],
            "uses_index": False,
            "expected": "SAME",
        })

        # 20. IS NULL combined with indexed equality
        test_cases.append({
            "name": "IS NULL + eq (indexed)",
            "desc": "WHERE status = X AND name IS NULL",
            "queries": [f"SELECT * FROM benchmark WHERE status = 'inactive' AND name IS NULL" for _ in range(QUERIES_PER_TEST)],
            "uses_index": True,
            "expected": "PASS",
        })

        # 21. OR on same indexed field (converts to IN, uses single index scan)
        #     (was #16 before IS NULL tests were added)
        #     Note: OR cross-field with range is not optimized yet (Union only supports Eq/In)
        test_cases.append({
            "name": "OR same-field (indexed)",
            "desc": "WHERE name = X OR name = Y (becomes IN)",
            "queries": [f"SELECT * FROM benchmark WHERE name = '{test_names[i]}' OR name = '{test_names[(i+1) % len(test_names)]}'"
                       for i in range(QUERIES_PER_TEST)],
            "uses_index": True,
            "expected": "PASS",
        })

        # 21b. OR cross-field with range - NOW SUPPORTED
        test_cases.append({
            "name": "OR cross-field+range (indexed)",
            "desc": "WHERE name = X OR age > 75",
            "queries": [f"SELECT * FROM benchmark WHERE name = '{test_names[i]}' OR age > 75"
                       for i in range(QUERIES_PER_TEST)],
            "uses_index": True,
            "expected": "PASS",
        })

        # ========== NEW: Smart Predicate Optimizer tests ==========

        # 22. IN clause (small list) - uses IndexLookup::In
        test_cases.append({
            "name": "IN clause (2 values)",
            "desc": "WHERE status IN ['active', 'pending']",
            "queries": [f"SELECT * FROM benchmark WHERE status IN ['active', 'pending']" for _ in range(QUERIES_PER_TEST)],
            "uses_index": True,
            "expected": "PASS",
        })

        # 23. IN clause (medium list) - uses IndexLookup::In
        test_cases.append({
            "name": "IN clause (5 values)",
            "desc": "WHERE age IN [25, 30, 35, 40, 45]",
            "queries": [f"SELECT * FROM benchmark WHERE age IN [25, 30, 35, 40, 45]" for _ in range(QUERIES_PER_TEST)],
            "uses_index": True,
            "expected": "PASS",
        })

        # 24. IN clause on names (high selectivity)
        sample_names = random.sample(self.all_names, min(5, len(self.all_names)))
        test_cases.append({
            "name": "IN clause (unique values)",
            "desc": "WHERE name IN [unique1, unique2, ...]",
            "queries": [f"SELECT * FROM benchmark WHERE name IN ['{sample_names[0]}', '{sample_names[1]}', '{sample_names[2]}']"
                       for _ in range(QUERIES_PER_TEST)],
            "uses_index": True,
            "expected": "PASS",
        })

        # 25. Complex: A AND (B OR C) - Smart predicate optimizer
        test_cases.append({
            "name": "AND + nested OR",
            "desc": "status = X AND (age > Y OR age < Z)",
            "queries": [f"SELECT * FROM benchmark WHERE status = 'active' AND (age > {a} OR age < {a - 20})"
                       for a in test_ages],
            "uses_index": True,
            "expected": "PASS",
        })

        # 26. Complex: A AND (B OR C) AND D - three conjuncts with OR
        test_cases.append({
            "name": "AND + OR + AND (3 parts)",
            "desc": "status = X AND (name = Y OR name = Z) AND age > W",
            "queries": [f"SELECT * FROM benchmark WHERE status = 'active' AND (name = '{test_names[i]}' OR name = '{test_names[(i+1) % len(test_names)]}') AND age > 20"
                       for i in range(QUERIES_PER_TEST)],
            "uses_index": True,
            "expected": "PASS",
        })

        # 27. OR same field multiple values (should become Union, no dedup needed)
        test_cases.append({
            "name": "OR same field (3 values)",
            "desc": "status = 'active' OR status = 'pending' OR status = 'inactive'",
            "queries": [f"SELECT * FROM benchmark WHERE status = 'active' OR status = 'pending' OR status = 'inactive'"
                       for _ in range(QUERIES_PER_TEST)],
            "uses_index": True,
            "expected": "PASS",
        })

        # 28. OR different fields (Union with potential dedup)
        test_cases.append({
            "name": "OR different fields (Union)",
            "desc": "name = X OR status = 'active'",
            "queries": [f"SELECT * FROM benchmark WHERE name = '{n}' OR status = 'active'"
                       for n in test_names],
            "uses_index": True,
            "expected": "PASS",
        })

        # 29. Best index selection test: Eq vs Range (Eq should win)
        test_cases.append({
            "name": "Index selection (Eq vs Range)",
            "desc": "status = X AND age > Y (should use status)",
            "queries": [f"SELECT * FROM benchmark WHERE status = 'active' AND age > {a}"
                       for a in test_ages],
            "uses_index": True,
            "expected": "PASS",
        })

        # 30. Compound vs single index comparison
        test_cases.append({
            "name": "Compound index prefix",
            "desc": "status = X (compound [status,age] prefix)",
            "queries": [f"SELECT * FROM benchmark WHERE status = 'active'" for _ in range(QUERIES_PER_TEST)],
            "uses_index": True,
            "expected": "PASS",
        })

        # Run BEFORE indexes
        print("-" * 70)
        print("BEFORE INDEXES (full table scan)")
        print("-" * 70)
        print()

        before_results = {}
        for tc in test_cases:
            avg, p50, p99 = run_queries(tc["queries"])
            before_results[tc["name"]] = avg
            print(f"  {tc['name']:<35} avg={avg:.2f}ms p50={p50:.2f}ms p99={p99:.2f}ms")

        print()

        # Create indexes
        print("-" * 70)
        print("CREATING INDEXES")
        print("-" * 70)
        print()
        self.create_indexes()

        # Run AFTER indexes
        print("-" * 70)
        print("AFTER INDEXES")
        print("-" * 70)
        print()

        after_results = {}
        for tc in test_cases:
            avg, p50, p99 = run_queries(tc["queries"])
            after_results[tc["name"]] = avg
            idx_str = "IDX" if tc["uses_index"] else "SCAN"
            print(f"  {tc['name']:<35} [{idx_str}] avg={avg:.2f}ms p50={p50:.2f}ms p99={p99:.2f}ms")

        print()

        # Build results
        for tc in test_cases:
            before = before_results[tc["name"]]
            after = after_results[tc["name"]]

            # Determine status
            if tc["expected"] == "SAME":
                # Allow up to 2x variance for "same" (network/disk noise)
                status = "PASS" if 0.5 <= after / before <= 2.0 else "FAIL"
            elif tc["expected"] == "PASS":
                # Should be at least 1.5x faster
                status = "PASS" if after < before * 0.67 else "FAIL"
            else:
                status = "TODO"

            self.results.append(BenchmarkResult(
                name=tc["name"],
                description=tc["desc"],
                before_ms=before,
                after_ms=after,
                uses_index=tc["uses_index"],
                status=status,
            ))

        # Print summary
        self.print_summary()

        # Cleanup
        print("Cleaning up...")
        sql("DROP COLLECTION benchmark CASCADE")
        print("Done!")

    def print_summary(self):
        print("=" * 70)
        print("SUMMARY")
        print("=" * 70)
        print()
        print(f"{'Test':<35} {'Before':>8} {'After':>8} {'Speedup':>14} {'Status':>8}")
        print("-" * 70)

        passed = 0
        failed = 0
        todo = 0

        for r in self.results:
            status_icon = {"PASS": "✓", "FAIL": "✗", "TODO": "○"}.get(r.status, "?")
            print(f"{r.name:<35} {r.before_ms:>7.2f}ms {r.after_ms:>7.2f}ms {r.speedup:>14} {status_icon} {r.status}")

            if r.status == "PASS":
                passed += 1
            elif r.status == "FAIL":
                failed += 1
            else:
                todo += 1

        print("-" * 70)
        print(f"Total: {len(self.results)} tests | ✓ PASS: {passed} | ✗ FAIL: {failed} | ○ TODO: {todo}")
        print()


def main():
    global BASE_URL, DATABASE

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default=BASE_URL, help="StellarDB server URL")
    parser.add_argument("--database", default=DATABASE, help="Benchmark database")
    args = parser.parse_args()

    BASE_URL = args.url.rstrip("/")
    DATABASE = args.database

    ensure_database()
    benchmark = Benchmark()
    benchmark.run_all()


if __name__ == "__main__":
    main()
