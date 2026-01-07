//! Performance benchmarks for IvoryValley proxy.
//!
//! Run with: cargo bench
//! View HTML reports in: target/criterion/

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use ivoryvalley::db::{extract_dedup_uri, SeenUriStore};
use serde_json::{json, Value};
use std::time::Duration;

/// Generate a mock status with a unique URI.
fn generate_status(id: u64) -> Value {
    json!({
        "id": id.to_string(),
        "uri": format!("https://mastodon.social/users/testuser/statuses/{}", id),
        "content": "<p>Test content for status</p>",
        "created_at": "2024-01-01T12:00:00.000Z",
        "account": {
            "id": "1",
            "username": "testuser",
            "acct": "testuser",
            "display_name": "Test User"
        },
        "reblog": null,
        "replies_count": 0,
        "reblogs_count": 0,
        "favourites_count": 0
    })
}

/// Generate a mock reblog status.
fn generate_reblog(id: u64, original_id: u64) -> Value {
    json!({
        "id": id.to_string(),
        "uri": format!("https://mastodon.social/users/booster/statuses/{}", id),
        "content": "",
        "created_at": "2024-01-01T12:00:00.000Z",
        "account": {
            "id": "2",
            "username": "booster",
            "acct": "booster",
            "display_name": "Booster"
        },
        "reblog": {
            "id": original_id.to_string(),
            "uri": format!("https://fosstodon.org/users/original/statuses/{}", original_id),
            "content": "<p>Original content</p>",
            "account": {
                "id": "3",
                "username": "original",
                "acct": "original@fosstodon.org"
            }
        },
        "replies_count": 0,
        "reblogs_count": 0,
        "favourites_count": 0
    })
}

/// Benchmark database check_and_mark operations.
fn bench_db_check_and_mark(c: &mut Criterion) {
    let mut group = c.benchmark_group("db_check_and_mark");

    // Benchmark single check_and_mark on empty database
    group.bench_function("empty_db", |b| {
        let store = SeenUriStore::open(":memory:").unwrap();
        let mut counter = 0u64;
        b.iter(|| {
            counter += 1;
            let uri = format!("https://example.com/status/{}", counter);
            black_box(store.check_and_mark(&uri).unwrap())
        });
    });

    // Benchmark check_and_mark with existing URIs (hit rate test)
    group.bench_function("existing_uri", |b| {
        let store = SeenUriStore::open(":memory:").unwrap();
        let uri = "https://example.com/status/1";
        store.mark_seen(uri).unwrap();
        b.iter(|| black_box(store.check_and_mark(uri).unwrap()));
    });

    group.finish();
}

/// Benchmark database with varying sizes.
fn bench_db_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("db_scaling");
    group.measurement_time(Duration::from_secs(10));

    for size in [1_000, 10_000, 100_000].iter() {
        group.bench_with_input(BenchmarkId::new("is_seen", size), size, |b, &size| {
            let store = SeenUriStore::open(":memory:").unwrap();

            // Pre-populate the database
            for i in 0..size {
                let uri = format!("https://example.com/status/{}", i);
                store.mark_seen(&uri).unwrap();
            }

            // Benchmark lookup in the middle of the dataset
            let test_uri = format!("https://example.com/status/{}", size / 2);
            b.iter(|| black_box(store.is_seen(&test_uri).unwrap()));
        });

        group.bench_with_input(
            BenchmarkId::new("check_and_mark_new", size),
            size,
            |b, &size| {
                let store = SeenUriStore::open(":memory:").unwrap();

                // Pre-populate the database
                for i in 0..size {
                    let uri = format!("https://example.com/status/{}", i);
                    store.mark_seen(&uri).unwrap();
                }

                // Benchmark inserting new URIs
                let mut counter = size;
                b.iter(|| {
                    counter += 1;
                    let uri = format!("https://example.com/status/{}", counter);
                    black_box(store.check_and_mark(&uri).unwrap())
                });
            },
        );
    }

    group.finish();
}

/// Benchmark extract_dedup_uri function.
fn bench_extract_uri(c: &mut Criterion) {
    let mut group = c.benchmark_group("extract_uri");

    let regular_status = generate_status(1);
    let reblog_status = generate_reblog(2, 1);

    group.bench_function("regular_status", |b| {
        b.iter(|| black_box(extract_dedup_uri(&regular_status)));
    });

    group.bench_function("reblog_status", |b| {
        b.iter(|| black_box(extract_dedup_uri(&reblog_status)));
    });

    group.finish();
}

/// Benchmark timeline filtering with various feed sizes.
fn bench_timeline_filtering(c: &mut Criterion) {
    let mut group = c.benchmark_group("timeline_filtering");
    group.measurement_time(Duration::from_secs(10));

    for size in [20, 100, 500].iter() {
        group.throughput(Throughput::Elements(*size as u64));

        group.bench_with_input(BenchmarkId::new("all_new", size), size, |b, &size| {
            b.iter_batched(
                || {
                    let store = SeenUriStore::open(":memory:").unwrap();
                    let statuses: Vec<Value> =
                        (0..size).map(|i| generate_status(i as u64)).collect();
                    (store, statuses)
                },
                |(store, statuses)| {
                    for status in &statuses {
                        if let Some(uri) = extract_dedup_uri(status) {
                            black_box(store.check_and_mark(uri).unwrap());
                        }
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });

        group.bench_with_input(BenchmarkId::new("all_seen", size), size, |b, &size| {
            b.iter_batched(
                || {
                    let store = SeenUriStore::open(":memory:").unwrap();
                    let statuses: Vec<Value> =
                        (0..size).map(|i| generate_status(i as u64)).collect();
                    // Pre-mark all as seen
                    for status in &statuses {
                        if let Some(uri) = extract_dedup_uri(status) {
                            store.mark_seen(uri).unwrap();
                        }
                    }
                    (store, statuses)
                },
                |(store, statuses)| {
                    for status in &statuses {
                        if let Some(uri) = extract_dedup_uri(status) {
                            black_box(store.check_and_mark(uri).unwrap());
                        }
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });

        // Mixed: 50% new, 50% seen
        group.bench_with_input(BenchmarkId::new("mixed_50_50", size), size, |b, &size| {
            b.iter_batched(
                || {
                    let store = SeenUriStore::open(":memory:").unwrap();
                    let statuses: Vec<Value> =
                        (0..size).map(|i| generate_status(i as u64)).collect();
                    // Mark first half as seen
                    for status in statuses.iter().take(size / 2) {
                        if let Some(uri) = extract_dedup_uri(status) {
                            store.mark_seen(uri).unwrap();
                        }
                    }
                    (store, statuses)
                },
                |(store, statuses)| {
                    for status in &statuses {
                        if let Some(uri) = extract_dedup_uri(status) {
                            black_box(store.check_and_mark(uri).unwrap());
                        }
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

/// Benchmark JSON parsing and serialization (important for timeline processing).
fn bench_json_processing(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_processing");

    for size in [20, 100, 500].iter() {
        let statuses: Vec<Value> = (0..*size).map(|i| generate_status(i as u64)).collect();
        let json_bytes = serde_json::to_vec(&statuses).unwrap();

        group.throughput(Throughput::Bytes(json_bytes.len() as u64));

        group.bench_with_input(BenchmarkId::new("parse", size), &json_bytes, |b, bytes| {
            b.iter(|| {
                let parsed: Vec<Value> = serde_json::from_slice(bytes).unwrap();
                black_box(parsed)
            });
        });

        group.bench_with_input(
            BenchmarkId::new("serialize", size),
            &statuses,
            |b, statuses| {
                b.iter(|| {
                    let bytes = serde_json::to_vec(statuses).unwrap();
                    black_box(bytes)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark full pipeline: parse, filter, serialize.
fn bench_full_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_pipeline");
    group.measurement_time(Duration::from_secs(15));

    for size in [20, 100, 500].iter() {
        let statuses: Vec<Value> = (0..*size).map(|i| generate_status(i as u64)).collect();
        let json_bytes = serde_json::to_vec(&statuses).unwrap();

        group.throughput(Throughput::Elements(*size as u64));

        // Full pipeline with fresh database (all new)
        group.bench_with_input(
            BenchmarkId::new("all_new", size),
            &json_bytes,
            |b, bytes| {
                b.iter_batched(
                    || SeenUriStore::open(":memory:").unwrap(),
                    |store| {
                        // Parse
                        let statuses: Vec<Value> = serde_json::from_slice(bytes).unwrap();

                        // Filter
                        let filtered: Vec<Value> = statuses
                            .into_iter()
                            .filter(|status| {
                                if let Some(uri) = extract_dedup_uri(status) {
                                    !store.check_and_mark(uri).unwrap()
                                } else {
                                    true
                                }
                            })
                            .collect();

                        // Serialize
                        let result = serde_json::to_vec(&filtered).unwrap();
                        black_box(result)
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );

        // Full pipeline with pre-populated database (50% duplicates)
        group.bench_with_input(
            BenchmarkId::new("half_duplicates", size),
            &json_bytes,
            |b, bytes| {
                b.iter_batched(
                    || {
                        let store = SeenUriStore::open(":memory:").unwrap();
                        // Pre-mark half as seen
                        for i in 0..(*size / 2) {
                            let uri =
                                format!("https://mastodon.social/users/testuser/statuses/{}", i);
                            store.mark_seen(&uri).unwrap();
                        }
                        store
                    },
                    |store| {
                        // Parse
                        let statuses: Vec<Value> = serde_json::from_slice(bytes).unwrap();

                        // Filter
                        let filtered: Vec<Value> = statuses
                            .into_iter()
                            .filter(|status| {
                                if let Some(uri) = extract_dedup_uri(status) {
                                    !store.check_and_mark(uri).unwrap()
                                } else {
                                    true
                                }
                            })
                            .collect();

                        // Serialize
                        let result = serde_json::to_vec(&filtered).unwrap();
                        black_box(result)
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

/// Benchmark cleanup operation with varying database sizes.
fn bench_cleanup(c: &mut Criterion) {
    let mut group = c.benchmark_group("cleanup");
    group.measurement_time(Duration::from_secs(10));

    for size in [1_000, 10_000, 50_000].iter() {
        group.bench_with_input(BenchmarkId::new("full_cleanup", size), size, |b, &size| {
            b.iter_batched(
                || {
                    let store = SeenUriStore::open(":memory:").unwrap();
                    for i in 0..size {
                        let uri = format!("https://example.com/status/{}", i);
                        store.mark_seen(&uri).unwrap();
                    }
                    store
                },
                |store| {
                    // Remove all entries
                    black_box(store.cleanup(0).unwrap())
                },
                criterion::BatchSize::LargeInput,
            );
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_db_check_and_mark,
    bench_db_scaling,
    bench_extract_uri,
    bench_timeline_filtering,
    bench_json_processing,
    bench_full_pipeline,
    bench_cleanup,
);

criterion_main!(benches);
