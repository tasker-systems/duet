//! duet's actions: every command's work, each a registered action with a typed input, run the
//! same way from every face (the bus design's §1, "The action registry", and decisions 10 and
//! 11).
//!
//! - **An action's input is one type.** It derives clap's `Args`, so the CLI's arguments come
//!   from it, and `JsonSchema`, so a later face (an MCP tool, the app's form) gets its schema
//!   from the same type ([`Action::input`]).
//! - **An action runs where it is invoked** (`runs: anywhere`, decision 10). Today every action
//!   does; [`Runs::Server`] is for the ones that will need what only a server owns.
//! - **An action writes through its [`Context`],** which records each document it writes as a
//!   `doc.written` event. Inside a project ([`Project::find`]) the face appends them to the
//!   journal; outside one there is no journal, and nothing is recorded.
//!
//! The registry is [`ACTIONS`]. `duet-cli` maps clap's parse onto each action's input, and its
//! registry witness checks that every action has a CLI path and every subcommand that writes a
//! file is an action.

#![forbid(unsafe_code)]

pub mod abc;
pub mod capture;
pub mod config;
mod context;
pub mod convert;
pub mod drums;
pub mod io;
pub mod part;
pub mod play;
pub mod record;
mod registry;
pub mod song;
pub mod view;

pub use context::{Context, PROJECT_VAR, Project, actor, fresh_id};
pub use registry::{ACTIONS, Action, Runs, Writes, action};
