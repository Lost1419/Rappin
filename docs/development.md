# Developing and measuring Rappin

Use a tagged GitHub checkout for the full integration, fuzz and benchmark suites
and their maintenance scripts. The crates.io package includes the SDK, public
examples, unit-test support, runtime setup script and build-required guides/schema.
Historical reports and agent skills are distributed through GitHub.

The library keeps optional features off by default. CI checks the minimal build,
each optional feature separately, and the combined build on Rust 1.89. The checked-in
lockfile selects compatible dependencies; downstream applications control their own
lockfiles and Cargo resolver. If a newer transitive dependency raises its minimum
Rust version, select a compatible version or use Cargo's MSRV-aware resolution.

Stable Linux, macOS and Windows jobs run Clippy, every test target (including
examples and benches) and the all-feature documentation examples. Checks that don't
depend on the platform run once, on Linux: formatting, rustdoc warnings, minimal
documentation examples, the guide checks and the package check. The Rust 1.89 jobs
cover the minimal build. Nightly rustdoc measures item and eligible-example coverage;
both must be 100%. `scripts/check-doc-coverage.py` fails on missing/malformed coverage as well
as uncovered items. Rustdoc's eligible-example denominator excludes some item kinds;
it is not a claim that every field has a separate executable example.

From a checkout:

```sh
cargo test --locked --all-features --all-targets
cargo clippy --locked --all-features --all-targets -- -D warnings
cargo test --locked --all-features --doc
cargo test --locked --no-default-features --doc
python3 scripts/check-guides.py
cargo +nightly-2026-11-19 rustdoc --locked --all-features --lib -- -Z unstable-options --coverage --output-format json
python3 scripts/check-doc-coverage.py target/doc/rappin.json
```

Coverage JSON and normal rustdoc JSON both use `target/doc/rappin.json`; run the
checker immediately after the coverage command. Nightly is a maintenance tool,
not a requirement for applications using the SDK.

CI pins nightly through `PINNED_NIGHTLY` in `.github/workflows/ci.yml`, because rustdoc's
coverage flags have changed between nightlies before (`--show-coverage` became
`--coverage`). To move it, install the new date locally, run the coverage command and
the fuzz smoke with it, then update the variable and the commands in this guide.

## Dependency audits

CI pins third-party actions to full commit SHAs and grants read-only repository
contents access. The weekly scheduled security job audits both lockfiles without
starting the platform test matrix. GitHub loads schedules from the default
branch, so the schedule takes effect when this workflow reaches that branch.
The audit also runs on ordinary code changes to `main`.

Markdown-only pushes and pull requests use the documentation checks when their
diff is available. A manual CI dispatch also defaults to documentation checks.
Enable the `full_suite` input to run the platform test matrix manually.

```sh
cargo audit
cargo audit --no-fetch --file fuzz/Cargo.lock
```

## Daily Yapin API watch

`.github/workflows/yapin-api-watch.yml` runs every day at 19:00 UTC (12:00 MST) and on
manual dispatch. It snapshots Yapin's public contract with `skills/yapin-api-audit`,
then compares it with the previous day's snapshot (kept in the Actions cache) and with
the bundled `docs/openapi.json`. The run summary lists both results. A live change, such
as new spec entries, changed guide text, a new changelog entry, or a new API or SDK
version, opens an issue titled "Yapin API contract changed", or comments on that issue
if it's still open. A new build hash on its own doesn't count. The snapshot and the JSON
reports are kept as a run artifact for 30 days.

The new snapshot becomes the cached baseline only after any change notification has
succeeded. If opening or commenting on the issue fails, the next run (or a re-run)
reports the same change again instead of losing it. When the cache holds no previous
snapshot (on the first run, or after GitHub evicts it), the run becomes the new baseline
and only reports drift from the bundled spec. Specification extensions (`x-*`) are
reported as structural changes but are never listed as operations.
Pull requests that touch the watch run its offline tests and a live dry run. They never
open issues or replace the cached baseline. The classification logic lives in
`.github/scripts/yapin_api_watch.py`:

```sh
python3 -m unittest discover -s .github/scripts -p 'test_*.py'
```

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
cargo +nightly-2026-11-19 fuzz run events fuzz/corpus/events fuzz/seeds/events -- -max_total_time=60 -max_len=8192 -rss_limit_mb=1024
cargo +nightly-2026-11-19 fuzz run arguments fuzz/corpus/arguments fuzz/seeds/arguments -- -max_total_time=60 -max_len=8192 -rss_limit_mb=1024
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

Criterion measures quoted parsing, raw/typed event decoding, timestamped event
encoding and fingerprinting, message serialization,
capability validation, channel cache hit/replacement and native no-effect dispatch.
Dispatch preloads the bot identity to avoid authentication reads; its handler does
not reply or mutate remote state. Cache cases include Tokio `block_on` overhead;
replacement includes cloning the channel, and dispatch includes cloning the event.
These figures measure local SDK work, not network/API latency or end-to-end replies.

An [initial local sample](https://github.com/Lost1419/Rappin/blob/main/docs/sdk-performance-20261108.md) includes JSON provenance
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

## Smaller executables

Install the CLI from crates.io with `cargo install rappin-rs --features cli --profile small`.
Select any additional CLI features needed for your deployment, such as `voice`.
Build a checkout with these commands.

```sh
cargo build --locked --profile small --features cli --bin rappin
cargo build --locked --profile small --example ping_bot
```

The `small` profile inherits `release`, uses `opt-level = "s"`, thin LTO, one
codegen unit and symbol stripping. It explicitly keeps `panic = "unwind"` so the
SDK can retain its supported panic-to-failure handling. Symbol stripping reduces
backtrace detail. LTO and fewer codegen units can increase build time, and size
optimization can affect throughput. Measure your own deployment workload.

Cargo uses profiles from the application's root manifest. To apply the same
settings to a consumer, add a profile there:

```toml
[profile.small]
inherits = "release"
opt-level = "s"
lto = "thin"
codegen-units = 1
strip = "symbols"
panic = "unwind"
```

For repeated local development, `cargo check --profile compact` and
`cargo test --profile compact` use the existing debug-free profile with incremental
compilation disabled. Normal `cargo check` and `cargo test` use Cargo's standard
profiles and can accumulate substantial debug/incremental artifacts over time.
Choose development profiles in your own application or workspace.

The published schema is compact JSON containing the same API fields and values.
Select `metrics-http` for the Prometheus HTTP listener and its smaller dependency
graph. The `metrics` feature enables HTTP and push-gateway exporter support.
Applications can configure an exporter through their own dependency.
