//! Configuration for Burrow.
//!
//! Future responsibility: parse, validate, and template `burrow.toml`, merge
//! `burrow.local.toml` overrides, detect dependency cycles, and report
//! actionable errors that point at the offending key or line.
//!
//! Implemented in milestone M2 (config and service graph). The crate is
//! intentionally empty in M1.
