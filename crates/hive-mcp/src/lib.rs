//! MCP for Hive: the registry of servers agents may call (`hive mcp`), a
//! minimal Model Context Protocol client over stdio, Streamable HTTP and the
//! older HTTP+SSE transport, OAuth 2.1 sign-in for http and sse servers
//! (`hive mcp login`), and the shared skills (`hive skill`).

pub mod browser;
pub mod client;
pub mod commands;
pub mod oauth;

pub use browser::Workspace;
pub use client::*;
pub use commands::{
    Checked, RESOLVER_TIMEOUT, connect_and_record, connect_error, register, resolver,
};
pub use oauth::LoginNeeded;
