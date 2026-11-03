//! Rustcraft launcher core: everything that is not UI or Android glue. Pure Rust, unit-testable on a desktop.
pub mod accounts;
pub mod assets;
pub mod controls;
pub mod download;
pub mod error;
pub mod jar_inspect;
pub mod launch;
pub mod logs;
pub mod manifest;
pub mod mods;
pub mod packs;
pub mod paths;
pub mod prepare;
pub mod profiles;
pub mod runtime;
pub mod servers;
pub mod store;
pub mod theme;

pub use error::{Error, Result};
pub use paths::Dirs;
