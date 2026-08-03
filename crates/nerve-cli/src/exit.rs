//! Exit codes.
//!
//! These are part of the CLI contract: scripts and CI depend on them, so they are named
//! constants rather than literals scattered through the command handlers.

/// The command did what was asked.
pub const SUCCESS: i32 = 0;

/// There is no index at the requested path, or it is not healthy enough to answer.
pub const NO_INDEX: i32 = 2;

/// The index was written, but some files could not be read or parsed.
pub const PARTIAL_INDEX: i32 = 3;

/// The index is internally sound but no longer describes the working tree.
///
/// Distinct from [`PARTIAL_INDEX`] on purpose: that one says the index Nerve holds is
/// incomplete, this one says the index is complete and describes a repository that has since
/// moved on. Only `nerve check` returns it, and only `nerve check` may: every other command
/// reports freshness alongside its answer rather than refusing to answer.
pub const STALE_INDEX: i32 = 4;

/// The command line was wrong.
pub const USAGE: i32 = 10;

/// Something failed that is our fault.
pub const INTERNAL: i32 = 70;
