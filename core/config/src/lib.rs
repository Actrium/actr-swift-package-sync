//! # actr-config
//!
//! Configuration file parser and project manifest support for Actor-RTC framework.
//!
//! This crate provides a two-layer configuration system:
//! - `RawConfig`: Direct TOML mapping (no processing)
//! - `Config`: Fully parsed and validated final configuration
//!
//! The parser uses an edition-based system, allowing the configuration format
//! to evolve over time while maintaining backward compatibility.
//!
//! # Example
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use actr_config::ConfigParser;
//!
//! // Parse configuration from file
//! let config = ConfigParser::from_file("actr.toml")?;
//!
//! // Access parsed values
//! println!("Package: {}", config.package.name);
//! println!("Realm: {}", config.realm.realm_id);
//! # Ok(())
//! # }
//! ```

// Core modules
pub mod config;
pub mod error;
pub mod lock;
pub mod parser;
pub mod raw;

// Re-exports for convenience
pub use config::*;
pub use error::*;
pub use lock::*;
pub use parser::*;
pub use raw::*;

/// Re-export commonly used types
pub use serde::{Deserialize, Serialize};
pub use url::Url;
