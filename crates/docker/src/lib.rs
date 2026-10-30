//! Halfnine's Docker adapter: it finds the Docker engine on this computer and runs Postgres containers in it. Only an
//! engine on this computer is used, through its socket or named pipe; this crate makes no other connection.

pub mod discover;
mod engine;
pub mod postgres;

pub use discover::{Endpoint, Found};
pub use engine::{Container, Engine, Layers, NewContainer, Ran, port_taken};
pub use postgres::{Credentials, VERSIONS, data_path, is_postgres, label, major, pick_port, valid_name};
