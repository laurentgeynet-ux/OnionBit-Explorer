// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Error type for `explorer-jscore`.

use thiserror::Error;

/// Host-facing error: either a parse failure or an uncaught script
/// exception (described as a string, the heap being gone by then).
#[derive(Debug, Error)]
pub enum Error {
    /// Source text could not be parsed (or uses an unsupported construct).
    #[error("syntax error at {line}:{col}: {message}")]
    Syntax {
        /// 1-based line of the offending token.
        line: usize,
        /// 1-based column of the offending token.
        col: usize,
        /// What went wrong.
        message: String,
    },
    /// The script threw a value that nothing caught.
    #[error("uncaught script exception: {0}")]
    Uncaught(String),
    /// The wall-clock eval deadline fired inside the engine
    /// (`EXPLORER_JS_EVAL_TIMEOUT_MS` — a burst of timers/
    /// microtasks saturating a core is killed in milliseconds,
    /// not at the 30 s channel timeout). Hosts map this to
    /// `JsError::Timeout` for the frozen-engine breaker.
    #[error("eval deadline exceeded")]
    Deadline,
}

/// Convenience alias used across the crate.
pub type Result<T> = std::result::Result<T, Error>;
