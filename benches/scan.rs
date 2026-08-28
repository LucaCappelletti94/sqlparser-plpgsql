//! Benchmarks for the scanner, the preprocessor, and the body parser.
//!
//! Every fixture is a real PL/pgSQL trigger body except the two adversarial
//! ones, which cover the paths that used to panic.

// criterion_group! expands to an undocumented public function, and a bench
// binary has no API for missing_docs to protect.
#![allow(missing_docs)]

use std::{hint::black_box, time::Duration};

use criterion::{
    BenchmarkGroup, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main,
    measurement::WallTime,
};
use sqlparser_plpgsql::{PlPgSqlPreprocessor, Scanner};

/// Inputs for the two scanner groups, smallest first.
const LADDER: &[(&str, &str)] = &[
    ("tiny", include_str!("fixtures/tiny.sql")),
    ("elsif_else", include_str!("fixtures/log_event_trigger.sql")),
    ("large", include_str!("fixtures/large_concatenated.sql")),
    (
        "adversarial_unicode",
        include_str!("fixtures/adversarial_unicode.sql"),
    ),
    (
        "adversarial_nested",
        include_str!("fixtures/adversarial_nested.sql"),
    ),
];

/// Real trigger bodies for the preprocessor and body parser groups.
const BODIES: &[(&str, &str)] = &[
    ("tiny", include_str!("fixtures/tiny.sql")),
    (
        "trigger_issue_f1",
        include_str!("fixtures/ensure_parent_procedure_templates.sql"),
    ),
    (
        "trigger_issue_f2",
        include_str!("fixtures/inherit_procedure_template_asset_models.sql"),
    ),
    (
        "groups_f1",
        include_str!("fixtures/create_owner_for_group.sql"),
    ),
    ("elsif_else", include_str!("fixtures/log_event_trigger.sql")),
];

/// A body holding three dollar-quoted literals, one of them with quotes inside.
const DOLLAR_LITERALS: &str = include_str!("fixtures/dollar_literals.sql");

fn configure(group: &mut BenchmarkGroup<'_, WallTime>) {
    group.warm_up_time(Duration::from_secs(5));
    group.measurement_time(Duration::from_secs(30));
    group.sample_size(100);
}

fn bytes(text: &str) -> Throughput {
    Throughput::Bytes(u64::try_from(text.len()).expect("a fixture is far below u64::MAX"))
}

fn scanner_regions(c: &mut Criterion) {
    let mut group = c.benchmark_group("scanner/regions");
    configure(&mut group);
    for (name, body) in LADDER {
        group.throughput(bytes(body));
        group.bench_with_input(BenchmarkId::from_parameter(name), body, |b, body| {
            b.iter(|| Scanner::new(black_box(body)).regions());
        });
    }
    group.finish();
}

fn scanner_find_keyword(c: &mut Criterion) {
    let mut group = c.benchmark_group("scanner/find_keyword");
    configure(&mut group);
    for (name, body) in LADDER {
        group.throughput(bytes(body));
        group.bench_with_input(BenchmarkId::from_parameter(name), body, |b, body| {
            b.iter(|| Scanner::new(black_box(body)).find_keyword("ELSIF", 0));
        });
    }
    group.finish();
}

fn scanner_requote(c: &mut Criterion) {
    let mut group = c.benchmark_group("scanner/requote_dollar_literals");
    configure(&mut group);
    group.throughput(bytes(DOLLAR_LITERALS));
    group.bench_function("three_literals", |b| {
        b.iter(|| Scanner::new(black_box(DOLLAR_LITERALS)).requote_dollar_literals());
    });
    group.finish();
}

fn preprocessor_preprocess(c: &mut Criterion) {
    let mut group = c.benchmark_group("preprocessor/preprocess");
    configure(&mut group);
    for (name, body) in BODIES {
        assert!(
            PlPgSqlPreprocessor::preprocess(body).is_ok(),
            "fixture {name} must preprocess, or the group times the error path"
        );
        group.throughput(bytes(body));
        group.bench_with_input(BenchmarkId::from_parameter(name), body, |b, body| {
            b.iter(|| PlPgSqlPreprocessor::preprocess(black_box(body)));
        });
    }
    group.finish();
}

#[cfg(feature = "body-parse")]
fn body_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("body_parse/parse_body");
    configure(&mut group);
    for (name, body) in BODIES {
        assert!(
            sqlparser_plpgsql::parse_body(name, body).is_ok(),
            "fixture {name} must parse, or the group times the error path"
        );
        group.throughput(bytes(body));
        group.bench_with_input(BenchmarkId::from_parameter(name), body, |b, body| {
            b.iter(|| sqlparser_plpgsql::parse_body(black_box(name), black_box(body)));
        });
    }
    group.finish();
}

/// Without the `body-parse` feature there is nothing to measure here.
#[cfg(not(feature = "body-parse"))]
fn body_parse(_: &mut Criterion) {}

criterion_group!(
    benches,
    scanner_regions,
    scanner_find_keyword,
    scanner_requote,
    preprocessor_preprocess,
    body_parse
);
criterion_main!(benches);
