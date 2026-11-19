//! Shared native NAT classifier and selector.

pub use iroh::endpoint::nat_selector::*;

// The vendored iroh package is outside the root test workspace.
#[cfg(test)]
#[allow(dead_code)]
#[path = "../vendor/iroh/src/endpoint/nat_selector.rs"]
mod shared_tests;
