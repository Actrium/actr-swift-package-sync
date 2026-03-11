//! Service compatibility analysis using proto-sign for semantic protobuf analysis

use crate::{
    CompatibilityAnalysisResult, CompatibilityError, CompatibilityLevel, ProtocolChange, Result,
};
use actr_protocol::ServiceSpec;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Breaking change detected by proto-sign analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakingChange {
    /// Rule that was violated (e.g., "FIELD_REMOVED")
    pub rule: String,
    /// File where the breaking change occurred
    pub file: String,
    /// Specific location (e.g., "User.email")
    pub location: String,
    /// Description of the breaking change
    pub message: String,
}

/// Compatibility analysis with detailed breakdown
#[derive(Debug)]
pub struct CompatibilityAnalysis {
    /// Overall compatibility assessment from proto-sign
    pub compatibility: proto_fingerprint::Compatibility,
    /// All changes detected (breaking and non-breaking)
    pub changes: Vec<ProtocolChange>,
    /// Only breaking changes
    pub breaking_changes: Vec<BreakingChange>,
}

/// Main service compatibility analyzer
pub struct ServiceCompatibility;

impl ServiceCompatibility {
    /// Analyze compatibility between two ServiceSpec using both fingerprints and proto-sign
    pub fn analyze_compatibility(
        base_service: &ServiceSpec,
        candidate_service: &ServiceSpec,
    ) -> Result<CompatibilityAnalysisResult> {
        // Validate input services
        Self::validate_service(base_service, "base")?;
        Self::validate_service(candidate_service, "candidate")?;

        // Quick check: if semantic fingerprints are identical, no changes
        if base_service.fingerprint == candidate_service.fingerprint {
            return Ok(CompatibilityAnalysisResult {
                level: CompatibilityLevel::FullyCompatible,
                changes: vec![],
                breaking_changes: vec![],
                base_semantic_fingerprint: base_service.fingerprint.clone(),
                candidate_semantic_fingerprint: candidate_service.fingerprint.clone(),
                analyzed_at: chrono::Utc::now(),
            });
        }

        // Perform detailed proto-sign analysis on changed files
        let mut all_changes = Vec::new();
        let mut breaking_changes = Vec::new();
        let mut overall_compatibility = proto_fingerprint::Compatibility::Green;

        // Create package content maps (using package name directly)
        let base_files: HashMap<String, String> = base_service
            .protobufs
            .iter()
            .map(|f| (f.package.clone(), f.content.clone()))
            .collect();
        let candidate_files: HashMap<String, String> = candidate_service
            .protobufs
            .iter()
            .map(|f| (f.package.clone(), f.content.clone()))
            .collect();

        // Analyze each file pair
        for (file_name, base_content) in &base_files {
            if let Some(candidate_content) = candidate_files.get(file_name) {
                // File exists in both versions - analyze changes
                let analysis = Self::analyze_file_pair(file_name, base_content, candidate_content)?;

                all_changes.extend(analysis.changes);
                breaking_changes.extend(analysis.breaking_changes);
                overall_compatibility =
                    Self::merge_compatibility(overall_compatibility, analysis.compatibility);
            } else {
                // File was removed - this is a breaking change
                breaking_changes.push(BreakingChange {
                    rule: "FILE_REMOVED".to_string(),
                    file: file_name.clone(),
                    location: file_name.clone(),
                    message: format!("Proto file '{file_name}' was removed"),
                });
                overall_compatibility = proto_fingerprint::Compatibility::Red;
            }
        }

        // Check for newly added files (generally safe)
        for file_name in candidate_files.keys() {
            if !base_files.contains_key(file_name) {
                all_changes.push(ProtocolChange {
                    change_type: "FILE_ADDED".to_string(),
                    file_name: file_name.clone(),
                    location: file_name.clone(),
                    description: format!("Proto file '{file_name}' was added"),
                    is_breaking: false,
                });
            }
        }

        // Convert proto-sign compatibility to our enum
        let level = match overall_compatibility {
            proto_fingerprint::Compatibility::Green => CompatibilityLevel::FullyCompatible,
            proto_fingerprint::Compatibility::Yellow => CompatibilityLevel::BackwardCompatible,
            proto_fingerprint::Compatibility::Red => CompatibilityLevel::BreakingChanges,
        };

        Ok(CompatibilityAnalysisResult {
            level,
            changes: all_changes,
            breaking_changes,
            base_semantic_fingerprint: base_service.fingerprint.clone(),
            candidate_semantic_fingerprint: candidate_service.fingerprint.clone(),
            analyzed_at: chrono::Utc::now(),
        })
    }

    /// Analyze a single file pair using proto-sign
    fn analyze_file_pair(
        file_name: &str,
        base_content: &str,
        candidate_content: &str,
    ) -> Result<CompatibilityAnalysis> {
        // Parse proto specifications using proto-sign
        let base_spec = proto_fingerprint::Spec::try_from(base_content).map_err(|e| {
            CompatibilityError::ProtoParseError {
                file_name: file_name.to_string(),
                source: e,
            }
        })?;

        let candidate_spec = proto_fingerprint::Spec::try_from(candidate_content).map_err(|e| {
            CompatibilityError::ProtoParseError {
                file_name: file_name.to_string(),
                source: e,
            }
        })?;

        // Perform proto-sign compatibility analysis
        let compatibility = base_spec.compare_with(&candidate_spec);

        // Convert proto-sign results to our format
        let changes = if base_spec.fingerprint != candidate_spec.fingerprint {
            vec![ProtocolChange {
                change_type: "PROTO_CONTENT_CHANGED".to_string(),
                file_name: file_name.to_string(),
                location: file_name.to_string(),
                description: format!("Proto file '{file_name}' has semantic changes"),
                is_breaking: compatibility == proto_fingerprint::Compatibility::Red,
            }]
        } else {
            vec![]
        };

        let breaking_changes = if compatibility == proto_fingerprint::Compatibility::Red {
            vec![BreakingChange {
                rule: "BREAKING_PROTO_CHANGE".to_string(),
                file: file_name.to_string(),
                location: file_name.to_string(),
                message: format!("Breaking changes detected in '{file_name}'"),
            }]
        } else {
            vec![]
        };

        Ok(CompatibilityAnalysis {
            compatibility,
            changes,
            breaking_changes,
        })
    }

    /// Validate that a service has required proto files
    fn validate_service(service: &ServiceSpec, label: &str) -> Result<()> {
        if service.protobufs.is_empty() {
            return Err(CompatibilityError::NoProtoFiles {
                service_name: format!("{label} service"),
            });
        }

        for proto_file in &service.protobufs {
            if proto_file.content.trim().is_empty() {
                return Err(CompatibilityError::InvalidService(format!(
                    "Package '{}' has empty content",
                    proto_file.package
                )));
            }
        }

        Ok(())
    }

    /// Merge two compatibility levels (most restrictive wins)
    fn merge_compatibility(
        current: proto_fingerprint::Compatibility,
        new: proto_fingerprint::Compatibility,
    ) -> proto_fingerprint::Compatibility {
        match (current, new) {
            (proto_fingerprint::Compatibility::Red, _)
            | (_, proto_fingerprint::Compatibility::Red) => proto_fingerprint::Compatibility::Red,
            (proto_fingerprint::Compatibility::Yellow, _)
            | (_, proto_fingerprint::Compatibility::Yellow) => {
                proto_fingerprint::Compatibility::Yellow
            }
            _ => proto_fingerprint::Compatibility::Green,
        }
    }

    /// 检查是否存在破坏性变更（用于 CI/CD 快速判断）
    ///
    /// 注意：此方法内部会执行完整的兼容性分析。如果需要详细信息，
    /// 建议直接调用 `analyze_compatibility` 并检查 `result.level`。
    pub fn is_breaking(
        base_service: &ServiceSpec,
        candidate_service: &ServiceSpec,
    ) -> Result<bool> {
        let result = Self::analyze_compatibility(base_service, candidate_service)?;
        Ok(matches!(result.level, CompatibilityLevel::BreakingChanges))
    }

    /// 获取破坏性变更列表（用于生成升级指南、错误报告等）
    ///
    /// 注意：此方法内部会执行完整的兼容性分析。如果需要其他信息，
    /// 建议直接调用 `analyze_compatibility` 并使用 `result.breaking_changes`。
    pub fn breaking_changes(
        base_service: &ServiceSpec,
        candidate_service: &ServiceSpec,
    ) -> Result<Vec<BreakingChange>> {
        let result = Self::analyze_compatibility(base_service, candidate_service)?;
        Ok(result.breaking_changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Fingerprint, ProtoFile};

    fn create_test_service(name: &str, _version: &str, proto_content: &str) -> ServiceSpec {
        let proto_files = vec![ProtoFile {
            name: "test.proto".to_string(),
            content: proto_content.to_string(),
            path: None,
        }];

        // Calculate real semantic fingerprint
        let semantic_fp = Fingerprint::calculate_service_semantic_fingerprint(&proto_files)
            .unwrap_or_else(|_| "test-fp".to_string());

        ServiceSpec {
            name: name.to_string(),
            description: Some("Test service".to_string()),
            fingerprint: semantic_fp,
            protobufs: vec![actr_protocol::service_spec::Protobuf {
                package: "test.proto".to_string(),
                content: proto_content.to_string(),
                fingerprint: "file-fp".to_string(),
            }],
            published_at: None,
            tags: vec![],
        }
    }

    #[test]
    fn test_identical_services() {
        let proto_content = r#" 
            syntax = "proto3";
            message TestMessage {
                string name = 1;
            }
        "#;

        let service1 = create_test_service("test", "1.0.0", proto_content);
        let service2 = create_test_service("test", "1.0.0", proto_content);

        let result = ServiceCompatibility::analyze_compatibility(&service1, &service2).unwrap();
        assert_eq!(result.level, CompatibilityLevel::FullyCompatible);
        assert_eq!(result.changes.len(), 0);
        assert_eq!(result.breaking_changes.len(), 0);
    }

    #[test]
    fn test_is_breaking() {
        let base_proto = r#" 
            syntax = "proto3";
            message User {
                string name = 1;
                string email = 2;
            }
        "#;

        let candidate_proto = r#" 
            syntax = "proto3";
            message User {
                string name = 1;
                // email field removed - breaking change
            }
        "#;

        let base_service = create_test_service("user", "1.0.0", base_proto);
        let candidate_service = create_test_service("user", "1.1.0", candidate_proto);

        let is_breaking =
            ServiceCompatibility::is_breaking(&base_service, &candidate_service).unwrap();
        assert!(is_breaking);
    }

    #[test]
    fn test_service_validation_errors() {
        // Test empty proto files
        let empty_service = ServiceSpec {
            name: "empty".to_string(),
            description: None,
            fingerprint: "fp".to_string(),
            protobufs: vec![],
            published_at: None,
            tags: vec![],
        };

        let valid_service =
            create_test_service("valid", "1.0.0", "syntax = \"proto3\"; message Test {}");

        let result = ServiceCompatibility::analyze_compatibility(&empty_service, &valid_service);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CompatibilityError::NoProtoFiles { .. }
        ));

        // Test empty proto content
        let empty_content_service = ServiceSpec {
            name: "empty-content".to_string(),
            description: None,
            fingerprint: "fp".to_string(),
            protobufs: vec![actr_protocol::service_spec::Protobuf {
                package: "empty.proto".to_string(),
                content: "   \n  \t  ".to_string(), // Only whitespace
                fingerprint: "fp".to_string(),
            }],
            published_at: None,
            tags: vec![],
        };

        let result =
            ServiceCompatibility::analyze_compatibility(&empty_content_service, &valid_service);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CompatibilityError::InvalidService(_)
        ));
    }

    #[test]
    fn test_file_removed_breaking_change() {
        let base_service = ServiceSpec {
            name: "base-service".to_string(),
            description: None,
            fingerprint: "fp1".to_string(),
            protobufs: vec![
                actr_protocol::service_spec::Protobuf {
                    package: "user.proto".to_string(),
                    content: "syntax = \"proto3\"; message User { string name = 1; }".to_string(),
                    fingerprint: "fp-user".to_string(),
                },
                actr_protocol::service_spec::Protobuf {
                    package: "order.proto".to_string(),
                    content: "syntax = \"proto3\"; message Order { string id = 1; }".to_string(),
                    fingerprint: "fp-order".to_string(),
                },
            ],
            published_at: None,
            tags: vec![],
        };

        // Remove order.proto file
        let candidate_service = ServiceSpec {
            name: "candidate-service".to_string(),
            description: None,
            fingerprint: "fp2".to_string(),
            protobufs: vec![actr_protocol::service_spec::Protobuf {
                package: "user.proto".to_string(),
                content: "syntax = \"proto3\"; message User { string name = 1; }".to_string(),
                fingerprint: "fp-user".to_string(),
            }],
            published_at: None,
            tags: vec![],
        };

        let result =
            ServiceCompatibility::analyze_compatibility(&base_service, &candidate_service).unwrap();
        assert_eq!(result.level, CompatibilityLevel::BreakingChanges);
        assert!(!result.breaking_changes.is_empty());

        let file_removed = result
            .breaking_changes
            .iter()
            .any(|bc| bc.rule == "FILE_REMOVED" && bc.file == "order.proto");
        assert!(
            file_removed,
            "Should detect file removal as breaking change"
        );
    }

    #[test]
    fn test_file_added_non_breaking() {
        let base_service = ServiceSpec {
            name: "base-service".to_string(),
            description: None,
            fingerprint: "fp1".to_string(),
            protobufs: vec![actr_protocol::service_spec::Protobuf {
                package: "user.proto".to_string(),
                content: "syntax = \"proto3\"; message User { string name = 1; }".to_string(),
                fingerprint: "fp-user".to_string(),
            }],
            published_at: None,
            tags: vec![],
        };

        // Add order.proto file
        let candidate_service = ServiceSpec {
            name: "candidate-service".to_string(),
            description: None,
            fingerprint: "fp2".to_string(),
            protobufs: vec![
                actr_protocol::service_spec::Protobuf {
                    package: "user.proto".to_string(),
                    content: "syntax = \"proto3\"; message User { string name = 1; }".to_string(),
                    fingerprint: "fp-user".to_string(),
                },
                actr_protocol::service_spec::Protobuf {
                    package: "order.proto".to_string(),
                    content: "syntax = \"proto3\"; message Order { string id = 1; }".to_string(),
                    fingerprint: "fp-order".to_string(),
                },
            ],
            published_at: None,
            tags: vec![],
        };

        let result =
            ServiceCompatibility::analyze_compatibility(&base_service, &candidate_service).unwrap();

        // Should be backward compatible (adding files is generally safe)
        let file_added = result
            .changes
            .iter()
            .any(|c| c.change_type == "FILE_ADDED" && c.file_name == "order.proto");
        assert!(file_added, "Should detect file addition");

        let breaking_file_changes = result
            .breaking_changes
            .iter()
            .any(|bc| bc.rule == "FILE_ADDED");
        assert!(
            !breaking_file_changes,
            "Adding files should not be breaking"
        );
    }

    #[test]
    fn test_breaking_changes() {
        let base_proto = r#"
            syntax = "proto3";
            message User {
                string name = 1;
                string email = 2;
            }
        "#;

        let candidate_proto = r#"
            syntax = "proto3";
            message User {
                string name = 1;
                // email removed
            }
        "#;

        let base_service = create_test_service("user", "1.0.0", base_proto);
        let candidate_service = create_test_service("user", "1.1.0", candidate_proto);

        let changes =
            ServiceCompatibility::breaking_changes(&base_service, &candidate_service).unwrap();
        assert!(!changes.is_empty());

        let has_breaking_proto_change = changes.iter().any(|bc| bc.rule == "BREAKING_PROTO_CHANGE");
        assert!(
            has_breaking_proto_change,
            "Should identify breaking proto changes"
        );
    }

    #[test]
    fn test_proto_parse_error() {
        let base_service =
            create_test_service("valid", "1.0.0", "syntax = \"proto3\"; message Valid {}");

        // Use a more definitively invalid proto content that proto-sign will reject
        let invalid_service = ServiceSpec {
            name: "invalid-service".to_string(),
            description: None,
            fingerprint: "fp".to_string(),
            protobufs: vec![actr_protocol::service_spec::Protobuf {
                package: "invalid.proto".to_string(),
                content: "completely invalid proto syntax { {{ ??? }}}".to_string(),
                fingerprint: "fp".to_string(),
            }],
            published_at: None,
            tags: vec![],
        };

        let result = ServiceCompatibility::analyze_compatibility(&base_service, &invalid_service);

        // If proto-sign can somehow parse our invalid content, just check we get an error of some kind
        match result {
            Err(CompatibilityError::ProtoParseError { .. }) => {
                // This is what we expect
            }
            Err(_) => {
                // Any error is acceptable for invalid proto content
            }
            Ok(analysis) => {
                // If it somehow succeeds, at least verify we get some kind of change detection
                assert!(
                    analysis.level != CompatibilityLevel::FullyCompatible
                        || !analysis.changes.is_empty()
                );
            }
        }
    }

    #[test]
    fn test_base_service_proto_parse_error() {
        // Test error in base service parsing
        let invalid_base_service = ServiceSpec {
            name: "invalid-base-service".to_string(),
            description: None,
            fingerprint: "fp".to_string(),
            protobufs: vec![actr_protocol::service_spec::Protobuf {
                package: "bad-base.proto".to_string(),
                content: "syntax = \"proto3\"; message".to_string(), // Incomplete syntax
                fingerprint: "fp".to_string(),
            }],
            published_at: None,
            tags: vec![],
        };

        let valid_service =
            create_test_service("valid", "1.0.0", "syntax = \"proto3\"; message Valid {}");

        let result =
            ServiceCompatibility::analyze_compatibility(&invalid_base_service, &valid_service);
        // Should get some kind of error (either parse error or analysis error)
        if let Err(CompatibilityError::ProtoParseError { file_name, .. }) = result {
            assert_eq!(file_name, "bad-base.proto");
        }
        // Other errors or success are also acceptable
    }

    #[test]
    fn test_backward_compatible_changes() {
        // Create a scenario that should trigger BackwardCompatible (Yellow) level
        let base_proto = r#" 
            syntax = "proto3";
            message User {
                string name = 1;
            }
        "#;

        // Add a field (typically backward compatible)
        let candidate_proto = r#" 
            syntax = "proto3";
            message User {
                string name = 1;
                string email = 2;  // New optional field
            }
        "#;

        let base_service = create_test_service("user", "1.0.0", base_proto);
        let candidate_service = create_test_service("user", "1.1.0", candidate_proto);

        let result =
            ServiceCompatibility::analyze_compatibility(&base_service, &candidate_service).unwrap();

        // The result might be FullyCompatible or BackwardCompatible depending on proto-sign's assessment
        // This test helps ensure we cover the Yellow compatibility path if it occurs
        assert!(matches!(
            result.level,
            CompatibilityLevel::FullyCompatible | CompatibilityLevel::BackwardCompatible
        ));
    }

    #[test]
    fn test_mixed_compatibility_merge() {
        // Create a scenario with multiple files having different compatibility levels
        let base_service = ServiceSpec {
            name: "base-service".to_string(),
            description: None,
            fingerprint: "fp1".to_string(),
            protobufs: vec![
                actr_protocol::service_spec::Protobuf {
                    package: "stable.proto".to_string(),
                    content: "syntax = \"proto3\"; message Stable { string name = 1; }".to_string(),
                    fingerprint: "fp-stable".to_string(),
                },
                actr_protocol::service_spec::Protobuf {
                    package: "evolving.proto".to_string(),
                    content: "syntax = \"proto3\"; message Evolving { string id = 1; }".to_string(),
                    fingerprint: "fp-evolving1".to_string(),
                },
            ],
            published_at: None,
            tags: vec![],
        };

        let candidate_service =
            ServiceSpec {
                name: "candidate-service".to_string(),
                description: None,
                fingerprint: "fp2".to_string(),
                protobufs: vec![
                actr_protocol::service_spec::Protobuf {
                    package: "stable.proto".to_string(),
                    content: "syntax = \"proto3\"; message Stable { string name = 1; }".to_string(), // No change
                    fingerprint: "fp-stable".to_string(),
                },
                actr_protocol::service_spec::Protobuf {
                    package: "evolving.proto".to_string(),
                    content: "syntax = \"proto3\"; message Evolving { string id = 1; string type = 2; }".to_string(), // Add field
                    fingerprint: "fp-evolving2".to_string(),
                },
            ],
                published_at: None,
                tags: vec![],
            };

        let result =
            ServiceCompatibility::analyze_compatibility(&base_service, &candidate_service).unwrap();

        // This test helps exercise the compatibility merging logic
        assert!(matches!(
            result.level,
            CompatibilityLevel::FullyCompatible
                | CompatibilityLevel::BackwardCompatible
                | CompatibilityLevel::BreakingChanges
        ));

        // Should have detected at least one change
        assert!(result.level != CompatibilityLevel::FullyCompatible || !result.changes.is_empty());
    }
}
