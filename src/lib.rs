//! Huda's single-profile application core. Adapters never own semantic storage.
pub mod acquisition;
pub mod answer;
pub mod api;
pub mod bridge;
pub mod error;
pub mod mcp;
pub mod relation;
pub mod runtime;
pub mod storage;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const MCP_VERSION: &str = "2026-07-28";
pub const IPC_VERSION: u32 = 1;
pub const MAX_MESSAGE_BYTES: usize = 1_048_576;
pub const MAX_SOURCE_BYTES: usize = 4_194_304;

/// OS/application identity, distinct from the stable existing data location.
pub const APP_NAME: &str = "Huda";
pub const APP_ID: &str = "com.devsolux.huda";
