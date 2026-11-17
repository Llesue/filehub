#![deny(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::dbg_macro,
    clippy::print_stdout,
    clippy::print_stderr
)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::print_stdout,
        clippy::print_stderr
    )
)]

pub mod admin_fs;
pub mod api;
pub mod app;
pub mod auth;
pub mod authz;
pub mod branding;
pub mod cmdrunner;
pub mod config;
pub mod db;
pub mod editor;
pub mod error;
pub mod events;
pub mod files;
pub mod fsops;
pub mod indexer;
pub mod jobs;
pub mod pathresolver;
pub mod platform;
pub mod preferences;
pub mod presence;
pub mod properties;
pub mod search;
#[cfg(windows)]
#[allow(unsafe_code)]
pub mod service;
pub mod shares;
pub mod state;
pub mod terminal_ws;
pub mod thumbs;
pub mod usage;
pub mod web;
#[cfg(windows)]
#[allow(unsafe_code)]
pub mod winindex;
