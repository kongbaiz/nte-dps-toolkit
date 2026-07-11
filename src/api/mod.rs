//! Stable external protocol types for the headless sidecar. This layer owns
//! JSON-RPC envelopes and DTO mappings but never starts capture or performs I/O.

pub mod dto;
pub mod jsonrpc;
pub mod request;
pub mod response;

pub const PROTOCOL_VERSION: u32 = 1;
pub const DATA_VERSION: &str = "1";
