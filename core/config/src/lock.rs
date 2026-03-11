//! Lock file management for actr.lock.toml
//!
//! This module provides lock file structures for dependency locking.
//! Proto content is cached to the project's `proto/` folder, not in the lock file.
//! The lock file only records metadata (fingerprints, paths) for version locking.

use crate::error::{ConfigError, Result};
use actr_protocol::ServiceSpec;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::str::FromStr;

/// Lock file structure for actr.lock.toml
///
/// Format matches documentation spec:
/// ```toml
/// [metadata]
/// version = 1
/// generated_at = "2024-01-15T10:30:00Z"
///
/// [[dependency]]
/// name = "user-service"
/// actr_type = "acme+user-service"
/// description = "User management service"
/// fingerprint = "service_semantic:a1b2c3d4e5f6..."
/// published_at = 1705315800
/// tags = ["latest", "stable"]
/// cached_at = "2024-01-15T10:30:00Z"
///
///   [[dependency.files]]
///   path = "user-service/user.v1.proto"
///   fingerprint = "semantic:abc123..."
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LockFile {
    /// Lock file metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<LockMetadata>,

    /// Locked dependencies (ordered array for deterministic output)
    #[serde(rename = "dependency", default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<LockedDependency>,
}

/// Lock file metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockMetadata {
    /// Lock file format version
    pub version: u32,
    /// Generation timestamp (ISO 8601)
    pub generated_at: String,
}

/// A locked dependency entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockedDependency {
    /// Service name (identifier in lock file, matches actr.toml dependency name property)
    pub name: String,

    /// Actor type (e.g., "acme+user-service")
    pub actr_type: String,

    /// Service description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Service-level semantic fingerprint (format: "service_semantic:hash")
    pub fingerprint: String,

    /// Publication timestamp (Unix epoch seconds)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<i64>,

    /// Tags like "latest", "stable"
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// When this dependency was cached (ISO 8601)
    pub cached_at: String,

    /// Proto file references (path + fingerprint only, no content)
    #[serde(rename = "files")]
    pub files: Vec<LockedProtoFile>,
}

/// Proto file reference in lock file (NO content, just path and fingerprint)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockedProtoFile {
    /// Relative path from project's proto/ folder (e.g., "user-service/user.v1.proto")
    pub path: String,

    /// Semantic fingerprint of the file (format: "semantic:hash")
    pub fingerprint: String,
}

/// Service specification metadata (for backward compatibility and conversions)
/// Holds proto file references for conversions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceSpecMeta {
    pub name: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Service-level semantic fingerprint
    pub fingerprint: String,

    /// Proto files referenced by path
    #[serde(rename = "files")]
    pub protobufs: Vec<ProtoFileMeta>,

    /// Publication timestamp (Unix epoch seconds)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<i64>,

    /// Tags like "latest", "stable"
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// Package-level protobuf entry in lock file
/// Note: References a local file instead of embedding content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtoFileMeta {
    /// Relative path to the proto file (e.g., "remote/user-service/user.v1.proto")
    pub path: String,

    /// Semantic fingerprint of the package content
    pub fingerprint: String,
}

// ============================================================================
// Bidirectional Conversion: ServiceSpec ↔ ServiceSpecMeta
// ============================================================================

impl From<ServiceSpec> for ServiceSpecMeta {
    fn from(spec: ServiceSpec) -> Self {
        Self {
            name: spec.name,
            description: spec.description,
            fingerprint: spec.fingerprint,
            protobufs: spec
                .protobufs
                .into_iter()
                .map(|proto| ProtoFileMeta {
                    path: format!("{}.proto", proto.package),
                    fingerprint: proto.fingerprint,
                })
                .collect(),
            published_at: spec.published_at,
            tags: spec.tags,
        }
    }
}

impl From<ServiceSpecMeta> for ServiceSpec {
    fn from(meta: ServiceSpecMeta) -> Self {
        Self {
            name: meta.name,
            description: meta.description,
            fingerprint: meta.fingerprint,
            protobufs: meta
                .protobufs
                .into_iter()
                .map(|proto| actr_protocol::service_spec::Protobuf {
                    package: package_from_path(&proto.path),
                    content: String::new(), // Content is no longer in lock file
                    fingerprint: proto.fingerprint,
                })
                .collect(),
            published_at: meta.published_at,
            tags: meta.tags,
        }
    }
}

// ============================================================================
// LockFile Operations
// ============================================================================

impl LockFile {
    /// Create a new empty lock file with current timestamp
    pub fn new() -> Self {
        Self {
            metadata: Some(LockMetadata {
                version: 1,
                generated_at: chrono::Utc::now().to_rfc3339(),
            }),
            dependencies: Vec::new(),
        }
    }

    /// Load lock file from disk
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        content.parse()
    }

    /// Save lock file to disk
    pub fn save_to_file(&self, path: impl AsRef<Path>) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Add or update a dependency
    pub fn add_dependency(&mut self, dep: LockedDependency) {
        // Remove existing entry with same name if exists
        self.dependencies.retain(|d| d.name != dep.name);

        // Add new entry
        self.dependencies.push(dep);

        // Sort by name for deterministic output
        self.dependencies.sort_by(|a, b| a.name.cmp(&b.name));
    }

    /// Get a dependency by name
    pub fn get_dependency(&self, name: &str) -> Option<&LockedDependency> {
        self.dependencies.iter().find(|d| d.name == name)
    }

    /// Remove a dependency by name
    pub fn remove_dependency(&mut self, name: &str) -> bool {
        let before = self.dependencies.len();
        self.dependencies.retain(|d| d.name != name);
        self.dependencies.len() != before
    }

    /// Update generation timestamp
    pub fn update_timestamp(&mut self) {
        if let Some(ref mut metadata) = self.metadata {
            metadata.generated_at = chrono::Utc::now().to_rfc3339();
        }
    }
}

impl FromStr for LockFile {
    type Err = ConfigError;

    fn from_str(s: &str) -> Result<Self> {
        toml::from_str(s).map_err(ConfigError::from)
    }
}

impl LockedDependency {
    /// Create a new locked dependency entry
    pub fn new(actr_type: String, spec: ServiceSpecMeta) -> Self {
        Self {
            name: spec.name,
            actr_type,
            description: spec.description,
            fingerprint: spec.fingerprint,
            published_at: spec.published_at,
            tags: spec.tags,
            cached_at: chrono::Utc::now().to_rfc3339(),
            files: spec
                .protobufs
                .iter()
                .map(|p| LockedProtoFile {
                    path: p.path.clone(),
                    fingerprint: p.fingerprint.clone(),
                })
                .collect(),
        }
    }

    /// Get service-level fingerprint
    pub fn service_fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Get file fingerprints
    pub fn file_fingerprints(&self) -> &[LockedProtoFile] {
        &self.files
    }
}

fn package_from_path(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_else(|| path.trim_end_matches(".proto").to_string())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_spec_conversion() {
        let spec = ServiceSpec {
            name: "test-service".to_string(),
            description: Some("Test service".to_string()),
            fingerprint: "service_semantic:abc123".to_string(),
            protobufs: vec![actr_protocol::service_spec::Protobuf {
                package: "user.v1".to_string(),
                content: "syntax = \"proto3\";".to_string(),
                fingerprint: "semantic:xyz".to_string(),
            }],
            published_at: Some(1705315800),
            tags: vec!["latest".to_string(), "stable".to_string()],
        };

        // Convert to meta
        let meta: ServiceSpecMeta = spec.clone().into();
        assert_eq!(meta.protobufs.len(), 1);
        assert_eq!(meta.protobufs[0].path, "user.v1.proto");
        assert_eq!(meta.protobufs[0].fingerprint, "semantic:xyz");
        assert_eq!(meta.published_at, Some(1705315800));
        assert_eq!(meta.tags.len(), 2);

        // Convert back to ServiceSpec
        let restored: ServiceSpec = meta.into();
        assert_eq!(restored.fingerprint, spec.fingerprint);
        assert_eq!(restored.protobufs.len(), 1);
        assert_eq!(restored.protobufs[0].package, spec.protobufs[0].package);
        // Note: content is lost during conversion back to ServiceSpec
        assert_eq!(restored.protobufs[0].content, "");
    }

    #[test]
    fn test_lock_file_operations() {
        let mut lock_file = LockFile::new();
        assert!(lock_file.dependencies.is_empty());

        let spec_meta = ServiceSpecMeta {
            name: "test-service".to_string(),
            description: None,
            fingerprint: "service_semantic:test".to_string(),
            protobufs: vec![],
            published_at: None,
            tags: vec![],
        };

        let dep = LockedDependency::new("acme+test-service".to_string(), spec_meta);

        lock_file.add_dependency(dep);
        assert_eq!(lock_file.dependencies.len(), 1);

        let found = lock_file.get_dependency("test-service");
        assert!(found.is_some());
        assert_eq!(found.unwrap().actr_type, "acme+test-service");

        let removed = lock_file.remove_dependency("test-service");
        assert!(removed);
        assert!(lock_file.dependencies.is_empty());
    }

    #[test]
    fn test_lock_file_serialization() {
        let mut lock_file = LockFile::new();

        let spec_meta = ServiceSpecMeta {
            name: "user-service".to_string(),
            description: Some("User service".to_string()),
            fingerprint: "service_semantic:abc123".to_string(),
            protobufs: vec![ProtoFileMeta {
                path: "user-service/user.v1.proto".to_string(),
                fingerprint: "semantic:xyz".to_string(),
            }],
            published_at: Some(1705315800),
            tags: vec!["latest".to_string()],
        };

        let dep = LockedDependency::new("acme+user-service".to_string(), spec_meta);

        lock_file.add_dependency(dep);

        // Serialize to TOML
        let toml_str = toml::to_string_pretty(&lock_file).unwrap();
        assert!(toml_str.contains("user-service"));
        assert!(toml_str.contains("user-service/user.v1.proto"));
        // Lock file should NOT contain proto content anymore
        assert!(!toml_str.contains("syntax = \"proto3\""));
        assert!(toml_str.contains("service_semantic:abc123"));

        // Deserialize back
        let restored: LockFile = toml::from_str(&toml_str).unwrap();
        assert_eq!(restored.dependencies.len(), 1);
        assert_eq!(restored.dependencies[0].name, "user-service");
        assert_eq!(
            restored.dependencies[0].files[0].path,
            "user-service/user.v1.proto"
        );
        assert_eq!(
            restored.dependencies[0].files[0].fingerprint,
            "semantic:xyz"
        );
    }

    #[test]
    fn test_path_serialization() {
        let spec_meta = ServiceSpecMeta {
            name: "user-service".to_string(),
            description: None,
            fingerprint: "service_semantic:test".to_string(),
            protobufs: vec![ProtoFileMeta {
                path: "user-service/user.v1.proto".to_string(),
                fingerprint: "semantic:abc".to_string(),
            }],
            published_at: None,
            tags: vec![],
        };

        // Serialize
        let toml_str = toml::to_string_pretty(&spec_meta).unwrap();

        // Should contain path
        assert!(toml_str.contains("path = \"user-service/user.v1.proto\""));

        // Deserialize back
        let restored: ServiceSpecMeta = toml::from_str(&toml_str).unwrap();
        assert_eq!(restored.protobufs[0].path, "user-service/user.v1.proto");
    }
}
