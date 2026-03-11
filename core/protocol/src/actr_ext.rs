//! Actor identity formatting and parsing utilities
//!
//! This module provides string formatting and parsing helpers for `ActrType` and `ActrId`.
//! String forms are stable for logging, configuration, and CLI interactions.
//!
//! ## String formats
//!
//! | Type     | Format                                        | Example                      |
//! |----------|-----------------------------------------------|------------------------------|
//! | ActrType | `manufacturer:name` or `manufacturer:name:version` | `acme:echo-service:1.0.0` |
//! | ActrId   | `<serial_hex>@<realm_id>/<actr_type>`         | `1a2b3c@101/acme:echo:1.0.0` |

use crate::{ActrId, ActrType, Realm, name::Name};
use std::str::FromStr;
use thiserror::Error;

/// Errors for actor identity parsing and formatting.
///
/// Covers only syntactic/structural validity of `ActrId` and `ActrType` strings.
/// Runtime and RPC errors belong to `ActrIdError` in the `error` module.
#[derive(Error, Debug, PartialEq, Eq)]
pub enum ActrIdError {
    #[error(
        "Invalid Actor ID format: '{0}'. Expected: <serial_hex>@<realm_id>/<manufacturer>:<name>[:<version>]"
    )]
    InvalidFormat(String),

    #[error("Invalid component in actor identity: {0}")]
    InvalidComponent(String),

    #[error("Invalid actor type format: '{0}'. Expected: <manufacturer>:<name>[:<version>]")]
    InvalidTypeFormat(String),
}

/// Helpers for `ActrType` string conversions.
pub trait ActrTypeExt: Sized {
    /// Convert to stable string representation.
    ///
    /// With non-empty version: `"manufacturer:name:version"`
    /// With empty version:     `"manufacturer:name"`
    fn to_string_repr(&self) -> String;

    /// Parse from string representation.
    ///
    /// Accepts:
    /// - `"manufacturer:name:version"` — with version (required)
    ///
    /// Returns `Err(ActrIdError::InvalidTypeFormat)` if version is absent,
    /// as `ActrType.version` is now a required field.
    fn from_string_repr(s: &str) -> Result<Self, ActrIdError>;
}

impl ActrTypeExt for ActrType {
    fn to_string_repr(&self) -> String {
        if self.version.is_empty() {
            format!("{}:{}", self.manufacturer, self.name)
        } else {
            format!("{}:{}:{}", self.manufacturer, self.name, self.version)
        }
    }

    fn from_string_repr(s: &str) -> Result<Self, ActrIdError> {
        // Require exactly 3 segments: manufacturer, name, version
        let parts: Vec<&str> = s.splitn(4, ':').collect();
        let (manufacturer, name, version) = match parts.as_slice() {
            [_, _] => {
                return Err(ActrIdError::InvalidTypeFormat(format!(
                    "{s} (version is required, expected <manufacturer>:<name>:<version>)"
                )));
            }
            [m, n, v] => (*m, *n, *v),
            _ => return Err(ActrIdError::InvalidTypeFormat(s.to_string())),
        };

        Name::new(manufacturer.to_string())
            .map_err(|e| ActrIdError::InvalidComponent(format!("Invalid manufacturer: {e}")))?;
        Name::new(name.to_string())
            .map_err(|e| ActrIdError::InvalidComponent(format!("Invalid type name: {e}")))?;

        Ok(ActrType {
            manufacturer: manufacturer.to_string(),
            name: name.to_string(),
            version: version.to_string(),
        })
    }
}

/// Helpers for `ActrId` string conversions.
pub trait ActrIdExt: Sized {
    /// Convert to `"<serial_hex>@<realm_id>/<actr_type>"`.
    ///
    /// The `/` separates the realm ID from the ActrType string, avoiding
    /// ambiguity with the `:` separators used inside ActrType.
    fn to_string_repr(&self) -> String;

    /// Parse from string representation.
    fn from_string_repr(s: &str) -> Result<Self, ActrIdError>;
}

impl ActrIdExt for ActrId {
    fn to_string_repr(&self) -> String {
        format!(
            "{:x}@{}/{}",
            self.serial_number,
            self.realm.realm_id,
            self.r#type.to_string_repr()
        )
    }

    fn from_string_repr(s: &str) -> Result<Self, ActrIdError> {
        // Format: "<serial_hex>@<realm_id>/<actr_type>"
        let (serial_part, rest) = s
            .split_once('@')
            .ok_or_else(|| ActrIdError::InvalidFormat("Missing '@' separator".to_string()))?;

        let serial_number = u64::from_str_radix(serial_part, 16).map_err(|_| {
            ActrIdError::InvalidComponent(format!("Invalid serial number hex: {serial_part}"))
        })?;

        let (realm_part, type_part) = rest
            .split_once('/')
            .ok_or_else(|| ActrIdError::InvalidFormat("Missing '/' separator".to_string()))?;

        let realm_id = u32::from_str(realm_part).map_err(|_| {
            ActrIdError::InvalidComponent(format!("Invalid realm ID: {realm_part}"))
        })?;

        let actr_type = ActrType::from_string_repr(type_part)?;

        Ok(ActrId {
            realm: Realm { realm_id },
            serial_number,
            r#type: actr_type,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_actor_id_roundtrip_with_version() {
        let original = ActrId {
            realm: Realm { realm_id: 101 },
            serial_number: 0x1a2b3c,
            r#type: ActrType {
                manufacturer: "acme".to_string(),
                name: "echo-service".to_string(),
                version: "1.0.0".to_string(),
            },
        };

        let s = original.to_string_repr();
        assert_eq!(s, "1a2b3c@101/acme:echo-service:1.0.0");

        let parsed = ActrId::from_string_repr(&s).unwrap();
        assert_eq!(parsed.realm.realm_id, original.realm.realm_id);
        assert_eq!(parsed.serial_number, original.serial_number);
        assert_eq!(parsed.r#type.manufacturer, original.r#type.manufacturer);
        assert_eq!(parsed.r#type.name, original.r#type.name);
        assert_eq!(parsed.r#type.version, original.r#type.version);
    }

    #[test]
    fn test_actor_id_roundtrip_without_version_errors() {
        // Parsing a string without version should now return an error
        let result = ActrId::from_string_repr("1a2b3c@101/acme:echo-service");
        assert!(
            matches!(result, Err(ActrIdError::InvalidTypeFormat(_))),
            "Expected InvalidTypeFormat error, got: {:?}",
            result
        );
    }

    #[test]
    fn test_invalid_actor_id_format() {
        assert!(matches!(
            ActrId::from_string_repr("invalid-string"),
            Err(ActrIdError::InvalidFormat(_))
        ));
        // Missing '/' between realm and type
        assert!(matches!(
            ActrId::from_string_repr("123@101"),
            Err(ActrIdError::InvalidFormat(_))
        ));
        // Invalid hex serial
        assert!(matches!(
            ActrId::from_string_repr("xyz@101/acme:echo"),
            Err(ActrIdError::InvalidComponent(_))
        ));
    }

    #[test]
    fn test_actr_type_roundtrip_with_version() {
        let s = "acme:echo:1.2.3";
        let ty = ActrType::from_string_repr(s).unwrap();
        assert_eq!(ty.manufacturer, "acme");
        assert_eq!(ty.name, "echo");
        assert_eq!(ty.version.as_str(), "1.2.3");
        assert_eq!(ty.to_string_repr(), s);
    }

    #[test]
    fn test_actr_type_without_version_is_error() {
        // Version is now required; two-segment strings must fail
        let s = "acme:echo-service";
        let result = ActrType::from_string_repr(s);
        assert!(
            matches!(result, Err(ActrIdError::InvalidTypeFormat(_))),
            "Expected InvalidTypeFormat, got: {:?}",
            result
        );
    }

    #[test]
    fn test_actr_type_invalid_format() {
        // Single segment (no colon) — invalid
        assert!(matches!(
            ActrType::from_string_repr("acme-echo"),
            Err(ActrIdError::InvalidTypeFormat(_))
        ));
        // Too many segments (4+)
        assert!(matches!(
            ActrType::from_string_repr("a:b:c:d"),
            Err(ActrIdError::InvalidTypeFormat(_))
        ));
        // Invalid manufacturer
        assert!(matches!(
            ActrType::from_string_repr("1acme:echo:v1"),
            Err(ActrIdError::InvalidComponent(_))
        ));
        // Invalid name
        assert!(matches!(
            ActrType::from_string_repr("acme:echo!:v1"),
            Err(ActrIdError::InvalidComponent(_))
        ));
    }
}
