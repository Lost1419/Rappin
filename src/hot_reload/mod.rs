//! True Rust cog code reloading through separately compiled worker executables.
//! The host owns the API token and event loop. Workers speak a versioned local JSON protocol.
//! Worker source, binaries, build scripts and filesystem access are trusted; this is not a sandbox.
#![doc = include_str!("../../docs/hot-reloading.md")]
mod host;
mod protocol;
mod worker;
pub use host::{HotReload, WatchGuard, WorkerProject};
pub use protocol::{CogEvent, Listener};
pub use worker::{WorkerCog, WorkerContext};
