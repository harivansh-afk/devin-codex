mod adapter;
pub mod compat;
pub mod config;
mod content;
pub mod gateway;
pub mod models;
pub mod proto;
pub mod request;
pub mod responses;
pub mod server;
pub mod session;
pub mod smoke;
pub mod tools;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Strict,
    Codex,
}
