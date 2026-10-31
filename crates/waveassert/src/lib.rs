//! Snapshot regression testing for rendered audio.
//!
//! WaveAssert checks that rendered audio still matches a stored reference
//! within explicit, documented tolerances, and explains why it does not when a
//! check fails. The design is in `docs/architecture.md`.
//!
//! This crate is a placeholder: it compiles and documents itself, but the
//! buffer, WAV, policy, comparison, report and snapshot modules are added by
//! later milestones. No API is stable yet.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
// The library never panics on untrusted input (§11). Test code is exempt.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
