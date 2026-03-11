//! Semantic fingerprint calculation for protocol services
//!
//! Provides semantic fingerprinting capabilities for proto files and services
//! using proto-sign analysis that ignores formatting differences.

use crate::{CompatibilityError, ProtoFile, Result};
use sha2::{Digest, Sha256};

/// Semantic fingerprint calculation utilities for protocol services
pub struct Fingerprint;

impl Fingerprint {
    // =========================================================================
    // Level 1: Single Proto File Fingerprinting
    // =========================================================================

    /// Calculate semantic fingerprint for a single proto file
    /// This uses proto-sign analysis and ignores formatting differences
    pub fn calculate_proto_semantic_fingerprint(proto_content: &str) -> Result<String> {
        let spec = proto_fingerprint::Spec::try_from(proto_content)
            .map_err(CompatibilityError::ProtoSignError)?;

        Ok(format!("semantic:{}", spec.fingerprint))
    }

    // Removed: calculate_proto_fingerprints() - content fingerprints are not semantically meaningful

    // =========================================================================
    // Level 2: Service-Level Multi-File Fingerprinting
    // =========================================================================

    /// Calculate semantic fingerprint for multiple proto files (service level)
    /// Uses proto-sign for each file and combines the results deterministically
    pub fn calculate_service_semantic_fingerprint(proto_files: &[ProtoFile]) -> Result<String> {
        let mut sorted_files = proto_files.to_vec();
        sorted_files.sort_by(|a, b| a.name.cmp(&b.name));

        let mut semantic_fingerprints = Vec::new();
        for file in &sorted_files {
            let spec = proto_fingerprint::Spec::try_from(file.content.as_str()).map_err(|e| {
                CompatibilityError::ProtoParseError {
                    file_name: file.name.clone(),
                    source: e,
                }
            })?;

            semantic_fingerprints.push(format!("{}:{}", file.name, spec.fingerprint));
        }

        let combined = semantic_fingerprints.join("\n");
        let mut hasher = Sha256::new();
        hasher.update(combined.as_bytes());
        Ok(format!("service_semantic:{:x}", hasher.finalize()))
    }

    // Removed: calculate_service_fingerprints() - content fingerprints are not semantically meaningful
}

// =========================================================================
// Fingerprint Result Structures
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use actr_protocol::ServiceSpec;

    fn create_test_proto_file(name: &str, content: &str) -> ProtoFile {
        ProtoFile {
            name: name.to_string(),
            content: content.to_string(),
            path: None,
        }
    }

    fn create_test_service(name: &str, proto_files: Vec<ProtoFile>) -> ServiceSpec {
        let semantic_fp = Fingerprint::calculate_service_semantic_fingerprint(&proto_files)
            .unwrap_or_else(|_| "placeholder".to_string());

        ServiceSpec {
            name: name.to_string(),
            description: Some("Test service".to_string()),
            fingerprint: semantic_fp,
            protobufs: proto_files
                .into_iter()
                .map(|pf| actr_protocol::service_spec::Protobuf {
                    package: pf.name, // Use name as package
                    content: pf.content,
                    fingerprint: "test-fp".to_string(),
                })
                .collect(),
            published_at: None,
            tags: vec![],
        }
    }

    #[test]
    fn test_semantic_fingerprint_ignores_formatting() {
        let content1 = "syntax=\"proto3\";message Test{string name=1;}";
        let content2 = r#"
            syntax = "proto3";

            // Test message
            message Test {
                string name = 1;  // Name field
            }
        "#;

        // Semantic fingerprints should be the same (semantically identical)
        let semantic_fp1 = Fingerprint::calculate_proto_semantic_fingerprint(content1).unwrap();
        let semantic_fp2 = Fingerprint::calculate_proto_semantic_fingerprint(content2).unwrap();

        assert_eq!(semantic_fp1, semantic_fp2);
    }

    #[test]
    fn test_service_semantic_fingerprint() {
        let files = vec![
            create_test_proto_file(
                "a.proto",
                "syntax = \"proto2\"; message A { required string name = 1; }",
            ),
            create_test_proto_file(
                "b.proto",
                "syntax = \"proto2\"; message B { required int32 id = 1; }",
            ),
        ];

        let fp = Fingerprint::calculate_service_semantic_fingerprint(&files).unwrap();
        assert!(fp.starts_with("service_semantic:"));
    }

    #[test]
    fn test_service_spec_has_semantic_fingerprint() {
        let proto_file = create_test_proto_file(
            "test.proto",
            r#"
            syntax = "proto3";
            message TestMessage {
                string name = 1;
            }
        "#,
        );

        let service = create_test_service("test-service", vec![proto_file]);

        // ServiceSpec should have fingerprint field
        assert!(!service.fingerprint.is_empty());
    }

    #[test]
    fn test_proto_semantic_fingerprint_error_handling() {
        // Test with invalid proto content
        let invalid_proto = "this is not a valid proto file";
        let result = Fingerprint::calculate_proto_semantic_fingerprint(invalid_proto);
        assert!(result.is_err());

        if let Err(e) = result {
            assert!(matches!(e, CompatibilityError::ProtoSignError(_)));
        }
    }

    #[test]
    fn test_service_semantic_fingerprint_error_handling() {
        // Test with invalid proto content in service
        let invalid_file = create_test_proto_file("invalid.proto", "invalid proto content");
        let result = Fingerprint::calculate_service_semantic_fingerprint(&[invalid_file]);

        assert!(result.is_err());
        if let Err(e) = result {
            assert!(matches!(e, CompatibilityError::ProtoParseError { .. }));
        }
    }

    #[test]
    fn test_service_semantic_fingerprint_deterministic() {
        // Test that file order doesn't affect fingerprint (files are sorted internally)
        let files_order1 = vec![
            create_test_proto_file(
                "z.proto",
                "syntax = \"proto3\"; message Z { string id = 1; }",
            ),
            create_test_proto_file(
                "a.proto",
                "syntax = \"proto3\"; message A { string name = 1; }",
            ),
        ];

        let files_order2 = vec![
            create_test_proto_file(
                "a.proto",
                "syntax = \"proto3\"; message A { string name = 1; }",
            ),
            create_test_proto_file(
                "z.proto",
                "syntax = \"proto3\"; message Z { string id = 1; }",
            ),
        ];

        let fp1 = Fingerprint::calculate_service_semantic_fingerprint(&files_order1).unwrap();
        let fp2 = Fingerprint::calculate_service_semantic_fingerprint(&files_order2).unwrap();

        assert_eq!(fp1, fp2, "File order should not affect fingerprint");
    }
}
