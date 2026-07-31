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

/// The command line was wrong.
pub const USAGE: i32 = 10;

/// Something failed that is our fault.
pub const INTERNAL: i32 = 70;
