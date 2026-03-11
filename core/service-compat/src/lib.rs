//! # Actor-RTC Protocol Compatibility Analysis Library
//!
//! A library providing semantic protocol compatibility analysis based on protobuf
//! schema evolution rules, using proto-sign for professional breaking change detection.
//!
//! ## Core Features
//!
//! - **Semantic Compatibility Analysis**: Deep protobuf schema compatibility checking
//! - **Breaking Change Detection**: Identify specific breaking changes between versions
//! - **Service-Level Comparison**: Compare complete ServiceSpec structures from actr-protocol
//! - **Stable Fingerprinting**: Semantic fingerprints that ignore formatting
//!
//! ## Usage
//!
//! ```rust,ignore
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use actr_version::{ServiceCompatibility, CompatibilityLevel, Fingerprint, ProtoFile};
//! use actr_protocol::ServiceSpec;
//!
//! // Example: Create service with proto content
//! let proto_files = vec![ProtoFile {
//!     name: "user.proto".to_string(),
//!     content: r#"
//!         syntax = "proto3";
//!         message User { string name = 1; string email = 2; }
//!     "#.to_string(),
//!     path: None,
//! }];
//!
//! let fingerprint = Fingerprint::calculate_service_semantic_fingerprint(&proto_files)?;
//!
//! let base_service = ServiceSpec {
//!     version: "1.0.0".to_string(),
//!     description: Some("User service".to_string()),
//!     fingerprint,
//!     protobufs: proto_files.into_iter().map(|pf| actr_protocol::service_spec::Protobuf {
//!         uri: format!("actr://user-service/{}", pf.name),
//!         content: pf.content,
//!         fingerprint: "file_fp".to_string(),
//!     }).collect(),
//! };
//!
//! # let candidate_service = base_service.clone();
//!
//! // Analyze compatibility between versions
//! let result = ServiceCompatibility::analyze_compatibility(&base_service, &candidate_service)?;
//!
//! match result.level {
//!     CompatibilityLevel::FullyCompatible => println!("✅ No changes"),
//!     CompatibilityLevel::BackwardCompatible => println!("⚠️ Backward compatible changes"),
//!     CompatibilityLevel::BreakingChanges => println!("❌ Breaking changes detected"),
//! }
//! # Ok(())
//! # }
//! ```

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod compatibility;
pub mod fingerprint;
pub mod types;

// Re-export actr-protocol types
pub use actr_protocol::{ServiceSpec, service_spec::Protobuf as ProtoFileSpec};

// Re-export our specific types
pub use compatibility::{BreakingChange, CompatibilityAnalysis, ServiceCompatibility};
pub use fingerprint::Fingerprint;
pub use types::{CompatibilityLevel, ProtoFile};

/// Detailed compatibility analysis result (Rust-specific, extends proto version)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatibilityAnalysisResult {
    /// Overall compatibility level
    pub level: CompatibilityLevel,
    /// Detailed list of changes detected
    pub changes: Vec<ProtocolChange>,
    /// Breaking changes (subset of changes)
    pub breaking_changes: Vec<BreakingChange>,
    /// Semantic fingerprint of base version
    pub base_semantic_fingerprint: String,
    /// Semantic fingerprint of candidate version
    pub candidate_semantic_fingerprint: String,
    /// Analysis timestamp
    pub analyzed_at: chrono::DateTime<chrono::Utc>,
}

impl CompatibilityAnalysisResult {
    /// Check if services are compatible (not breaking)
    pub fn is_compatible(&self) -> bool {
        !matches!(self.level, CompatibilityLevel::BreakingChanges)
    }

    /// Get a summary string of the analysis
    pub fn summary(&self) -> String {
        match self.level {
            CompatibilityLevel::FullyCompatible => "No changes detected".to_string(),
            CompatibilityLevel::BackwardCompatible => {
                format!("{} backward compatible changes", self.changes.len())
            }
            CompatibilityLevel::BreakingChanges => {
                format!("{} breaking changes detected", self.breaking_changes.len())
            }
        }
    }
}

/// Individual protocol change detected
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolChange {
    /// Type of change (e.g., "FIELD_REMOVED", "TYPE_CHANGED")
    pub change_type: String,
    /// File where change occurred
    pub file_name: String,
    /// Location within the file
    pub location: String,
    /// Human-readable description
    pub description: String,
    /// Whether this change breaks compatibility
    pub is_breaking: bool,
}

/// Errors that can occur during compatibility analysis
#[derive(Error, Debug)]
pub enum CompatibilityError {
    #[error("Failed to parse proto file: {file_name}: {source}")]
    ProtoParseError {
        file_name: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("Proto-sign analysis failed: {0}")]
    ProtoSignError(#[from] anyhow::Error),

    #[error("Service has no proto files: {service_name}")]
    NoProtoFiles { service_name: String },

    #[error("Invalid service structure: {0}")]
    InvalidService(String),
}

pub type Result<T> = std::result::Result<T, CompatibilityError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compatibility_levels() {
        // Test that compatibility levels work as expected
        assert_eq!(
            CompatibilityLevel::FullyCompatible,
            CompatibilityLevel::FullyCompatible
        );
        assert_ne!(
            CompatibilityLevel::FullyCompatible,
            CompatibilityLevel::BackwardCompatible
        );
    }

    #[test]
    fn test_protocol_change_creation() {
        let change = ProtocolChange {
            change_type: "FIELD_REMOVED".to_string(),
            file_name: "user.proto".to_string(),
            location: "User.email".to_string(),
            description: "Field 'email' was removed from message 'User'".to_string(),
            is_breaking: true,
        };

        assert_eq!(change.change_type, "FIELD_REMOVED");
        assert!(change.is_breaking);
    }

    #[test]
    fn test_compatibility_result_structure() {
        let result = CompatibilityAnalysisResult {
            level: CompatibilityLevel::BackwardCompatible,
            changes: vec![],
            breaking_changes: vec![],
            base_semantic_fingerprint: "abc123".to_string(),
            candidate_semantic_fingerprint: "def456".to_string(),
            analyzed_at: chrono::Utc::now(),
        };

        assert_eq!(result.level, CompatibilityLevel::BackwardCompatible);
        assert_eq!(result.changes.len(), 0);
    }

    #[test]
    fn test_compatibility_result_methods() {
        let breaking_result = CompatibilityAnalysisResult {
            level: CompatibilityLevel::BreakingChanges,
            changes: vec![ProtocolChange {
                change_type: "FIELD_REMOVED".to_string(),
                file_name: "user.proto".to_string(),
                location: "User.email".to_string(),
                description: "Field removed".to_string(),
                is_breaking: true,
            }],
            breaking_changes: vec![BreakingChange {
                rule: "FIELD_REMOVED".to_string(),
                file: "user.proto".to_string(),
                location: "User.email".to_string(),
                message: "Field email was removed".to_string(),
            }],
            base_semantic_fingerprint: "base_fp".to_string(),
            candidate_semantic_fingerprint: "candidate_fp".to_string(),
            analyzed_at: chrono::Utc::now(),
        };

        assert!(!breaking_result.is_compatible());
        assert!(breaking_result.summary().contains("breaking changes"));

        let compatible_result = CompatibilityAnalysisResult {
            level: CompatibilityLevel::FullyCompatible,
            changes: vec![],
            breaking_changes: vec![],
            base_semantic_fingerprint: "fp".to_string(),
            candidate_semantic_fingerprint: "fp".to_string(),
            analyzed_at: chrono::Utc::now(),
        };

        assert!(compatible_result.is_compatible());
        assert!(compatible_result.summary().contains("No changes"));
    }

    #[test]
    fn test_compatibility_error_display() {
        // Test ProtoParseError
        let parse_error = CompatibilityError::ProtoParseError {
            file_name: "test.proto".to_string(),
            source: anyhow::anyhow!("syntax error"),
        };
        let error_msg = format!("{parse_error}");
        assert!(error_msg.contains("test.proto"));
        assert!(error_msg.contains("syntax error"));

        // Test ProtoSignError
        let sign_error = CompatibilityError::ProtoSignError(anyhow::anyhow!("proto-sign failed"));
        let error_msg = format!("{sign_error}");
        assert!(error_msg.contains("proto-sign failed"));

        // Test NoProtoFiles
        let no_files_error = CompatibilityError::NoProtoFiles {
            service_name: "empty-service".to_string(),
        };
        let error_msg = format!("{no_files_error}");
        assert!(error_msg.contains("empty-service"));

        // Test InvalidService
        let invalid_error = CompatibilityError::InvalidService("missing fields".to_string());
        let error_msg = format!("{invalid_error}");
        assert!(error_msg.contains("missing fields"));
    }

    #[test]
    fn test_error_conversion() {
        // Test that anyhow::Error converts to ProtoSignError
        let anyhow_error = anyhow::anyhow!("some error");
        let compat_error: CompatibilityError = anyhow_error.into();
        assert!(matches!(
            compat_error,
            CompatibilityError::ProtoSignError(_)
        ));
    }
}
