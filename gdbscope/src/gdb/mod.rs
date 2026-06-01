//! GDB/MI subsystem.
//!
//! This module contains everything needed to drive a GDB process over the
//! Machine Interface (MI) protocol:
//!
//! * [`mi_types`] -- AST types for parsed MI output records and values.
//! * [`mi_parser`] -- Recursive-descent parser that turns raw MI text into
//!   [`mi_types::MiRecord`] values.
//! * [`mi_command`] -- Typed builder that produces token-tagged MI command
//!   strings.
//! * [`process`] -- Async child-process management (spawn, send, receive).
//! * [`controller`] -- High-level orchestrator that ties parsing, commands,
//!   and shared state together.

pub mod mi_types;
pub mod mi_parser;
pub mod mi_command;
pub mod process;
pub mod controller;

// ---- Re-exports for ergonomic access ----

pub use mi_command::MiCommandBuilder;
pub use mi_types::{MiBody, MiList, MiRecord, MiValue};
pub use process::GdbProcess;
