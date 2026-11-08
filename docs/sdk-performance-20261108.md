# Offline SDK performance sample

Measured 8 November 2026 on aarch64 macOS, rustc 1.99.0 (b940084d7 2026-09-28). CPU model was not captured. No optional SDK features; optimized bench profile. This is a local development-tree sample based on `9c88a49064229dde0844e47a1822ea5656723f7e`, with the new benchmark code and lockfile changes present.

The [JSON evidence](sdk-performance-20261108.json) records benchmark/lockfile hashes, compiler, sample configuration and 95% confidence intervals. It does not identify a clean published release.

| Case | Mean local time |
|---|---|
| arguments/quoted | 122.4 ns |
| event/decode | 383.4 ns |
| event/typed_payload | 104.9 ns |
| message/serialize | 29.8 ns |
| capability/validate | 1.529 µs |
| cache/channel_hit | 77.3 ns |
| cache/channel_replace | 90.0 ns |
| dispatch/native_no_effects | 2.918 µs |

20 samples per case, one-second warmup and one-second measurement. Treat the figures as an initial reproducible sample, not a cross-machine comparison or a CI speed requirement. Other development work may influence them. No prior SDK baseline exists to establish a performance improvement.

Dispatch uses a cached identity and a handler with no remote effects. Cache measurements include Tokio `block_on`; replacement and dispatch include their input clones. These costs do not include authentication, network transit, API processing, replies, worker IPC, or durable checkpoint storage.

See [development and measurement](development.md) for commands, input definitions and isolated build-time/executable-size measurements. Live Yapin API benchmarking remains separate.

An offline release build of the checked-in `ping_bot` example took 29.35 seconds with a fresh Cargo target and 0.17 seconds reusing it; executable size was 8,756,848 bytes. This includes repository development dependencies and warm registry/OS caches. [Build evidence](sdk-build-20261108.json) records this dirty-tree sample; it is not a standalone-consumer minimum or an isolated gateway/worker overhead comparison. The resulting executable was never run.
