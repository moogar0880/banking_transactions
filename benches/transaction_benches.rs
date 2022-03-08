use banking_transactions::engine::Ledger;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::path::PathBuf;

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("transactions_basic", |b| {
        b.iter(|| {
            Ledger::try_from(black_box(PathBuf::from(String::from(
                "data/transactions_basic",
            ))))
        })
    });

    c.bench_function("transactions_rounding", |b| {
        b.iter(|| {
            Ledger::try_from(black_box(PathBuf::from(String::from(
                "data/transactions_rounding",
            ))))
        })
    });

    c.bench_function("transactions_alot", |b| {
        b.iter(|| {
            Ledger::try_from(black_box(PathBuf::from(String::from(
                "data/transactions_alot",
            ))))
        })
    });

    c.bench_function("transactions_multiuser", |b| {
        b.iter(|| {
            Ledger::try_from(black_box(PathBuf::from(String::from(
                "data/transactions_multiuser",
            ))))
        })
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
