# Developing and measuring Rappin

The library keeps optional features off by default. CI checks the minimal build,
each optional feature separately, and the combined build on Rust 1.88. The checked-in
lockfile selects compatible dependencies; downstream applications control their own
lockfiles and Cargo resolver. If a newer transitive dependency raises its minimum
Rust version, select a compatible version or use Cargo's MSRV-aware resolution.

Stable Linux, macOS and Windows jobs run tests, examples, Clippy and documentation
examples. Nightly rustdoc measures item and eligible-example coverage; both must be
100%. `scripts/check-doc-coverage.py` fails on missing/malformed coverage as well
as uncovered items. Rustdoc's eligible-example denominator excludes some item kinds;
it is not a claim that every field has a separate executable example.

From a checkout:

```sh
cargo test --locked --all-features --all-targets
cargo clippy --locked --all-features --all-targets -- -D warnings
cargo test --locked --all-features --doc
cargo test --locked --no-default-features --doc
python3 scripts/check-guides.py
cargo +nightly rustdoc --locked --all-features --lib -- -Z unstable-options --show-coverage --output-format json
python3 scripts/check-doc-coverage.py target/doc/rappin.json
```

Coverage JSON and normal rustdoc JSON both use `target/doc/rappin.json`; run the
checker immediately after the coverage command. Nightly is a maintenance tool,
not a requirement for applications using the SDK.

## Offline randomized checks

`tests/properties_test.rs` checks quoted argument round trips and UTF-8 offsets,
malformed event/payload input, and ordered polling checkpoints with randomized
failures, duplicates and reversed batches. It runs actual SDK intake against local
HTTP fixtures, with no mutation routes. The first incomplete handler blocks the
checkpoint and subsequent handlers. Private protocol tests cover arbitrary,
truncated and oversized frames and preservation of the next frame.

```sh
cargo test --locked --all-features --test properties_test
cargo test --locked --features hot-reload --lib frame_properties
```

The separate `fuzz/` development workspace uses cargo-fuzz/libFuzzer for public
event/worker DTO decoding and argument/payload validation. Each input is bounded
to 8 KiB. It does not invoke a bot handler, load credentials, or access Yapin.
The actual private IPC framing reader is covered by in-crate property tests.

```sh
cargo install cargo-fuzz --locked --version 0.13.2
mkdir -p fuzz/corpus/events fuzz/corpus/arguments
cargo +nightly fuzz run events fuzz/corpus/events fuzz/seeds/events -- -max_total_time=60 -max_len=8192 -rss_limit_mb=1024
cargo +nightly fuzz run arguments fuzz/corpus/arguments fuzz/seeds/arguments -- -max_total_time=60 -max_len=8192 -rss_limit_mb=1024
```

CI runs 15 seconds per target; longer local runs provide more exploration. Preserve
crashing inputs as regression cases before changing behavior. Generated corpora,
artifacts and coverage stay outside releases. Neither a smoke run nor randomized
coverage proves absence of every parser or concurrency bug.

## SDK microbenchmarks

```sh
cargo bench --locked --bench sdk
# Short repeatable local sample:
cargo bench --locked --bench sdk -- --sample-size 20 --measurement-time 1 --warm-up-time 1
```

Criterion measures quoted parsing, raw/typed event decoding, message serialization,
capability validation, channel cache hit/replacement and native no-effect dispatch.
Dispatch preloads the bot identity to avoid authentication reads; its handler does
not reply or mutate remote state. Cache cases include Tokio `block_on` overhead;
replacement includes cloning the channel, and dispatch includes cloning the event.
These figures measure local SDK work, not network/API latency or end-to-end replies.

An [initial local sample](sdk-performance-20261108.md) includes JSON provenance
and measured confidence intervals.

Criterion stores estimates and confidence intervals under `target/criterion`.
Compare the same benchmark, compiler, feature selection, machine and power state.
Record the Git revision and whether the tree was dirty. Shared CI performs no
hardware-sensitive timing threshold; semantic tests remain independent of speed.

For build time and executable size:

```sh
python3 scripts/measure-build.py --template basic --out target/build-basic.json
python3 scripts/measure-build.py --template gateway --out target/build-gateway.json
python3 scripts/measure-build.py --template worker --out target/build-worker.json
```

The script builds the corresponding checked-in example in a new isolated target
directory, then measures a reused-target build. It records compiler/host, lockfile
hash, features, revision/dirty state, durations and executable bytes. It never runs
the bot or deletes existing build artifacts. "Fresh target" means no compiled
Cargo artifacts there; registry and operating-system caches can still be warm.
Cargo build scripts and procedural macros execute trusted code. Pre-fetch dependencies
before this offline measurement; the script reports build failures rather than
silently fetching or changing the lockfile. The examples differ in functionality,
so their sizes are deployment samples, not isolated feature overhead comparisons.
Example builds also compile this repository's development dependencies; the recorded
build time is not the minimum build time of a standalone consumer.

Live API latency and throughput benchmarking remains a separate project. SDK
microbenchmarks must not be presented as a Yapin service benchmark.
