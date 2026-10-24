//! Networking for Burrow.
//!
//! Future responsibility: allocate stable local ports, rewrite generated
//! environment URLs, and run a loopback-only reverse proxy that routes
//! `*.localhost` hostnames, including WebSocket upgrades, to service ports.
//!
//! Implemented in milestone M4 (networking). The crate is intentionally empty
//! in M1.
