# actr-version

[![CI](https://github.com/actor-rtc/actr-version/workflows/CI/badge.svg)](https://github.com/actor-rtc/actr-version/actions)
[![codecov](https://codecov.io/gh/actor-rtc/actr-version/branch/main/graph/badge.svg)](https://codecov.io/gh/actor-rtc/actr-version)
[![Crates.io](https://img.shields.io/crates/v/actr-version.svg)](https://crates.io/crates/actr-version)
[![Documentation](https://docs.rs/actr-version/badge.svg)](https://docs.rs/actr-version)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)

🔒 **Semantic Protocol Compatibility Analysis Library**

`actr-version` provides professional-grade protobuf compatibility analysis through `proto-sign` semantic breaking change detection and `actr-protocol` service structures, enabling comprehensive service version management.

## 🎯 Design Philosophy

This library solves the **real problem** of protocol compatibility analysis - not just comparing hashes, but understanding the **semantic meaning** of protobuf schema changes.

```rust
// Use proto-sign for true semantic analysis
let result = ServiceCompatibility::analyze_compatibility(&base_service, &candidate_service)?;
// Detects: field removal, type changes, backward compatibility, etc.
```

## 📦 Core Features

- **Semantic Proto Analysis**: Professional breaking change detection using `proto-sign`
- **Service-Level Compatibility**: Analyze complete `ServiceSpec` structures from `actr-protocol`
- **Breaking Change Detection**: Identify specific breaking changes with detailed explanations
- **Efficient Fingerprint Comparison**: Direct use of `ServiceSpec` built-in semantic fingerprints
- **Comprehensive Results**: Detailed compatibility analysis with actionable insights

## 🚀 Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
actr-version = { git = "https://github.com/actor-rtc/actr-version" }
actr-protocol = { git = "https://github.com/actor-rtc/actr-protocol" }
```

```rust
use actr_version::{ServiceCompatibility, CompatibilityLevel, Fingerprint, ProtoFile};
use actr_protocol::ServiceSpec;

// Create base service with proto content
let proto_files = vec![ProtoFile {
    name: "user.proto".to_string(),
    content: r#"
        syntax = "proto3";
        message User {
            string name = 1;
            string email = 2;
        }
    "#.to_string(),
    path: None,
}];

let base_fingerprint = Fingerprint::calculate_service_semantic_fingerprint(&proto_files)?;

let base_service = ServiceSpec {
    version: "1.0.0".to_string(),
    description: Some("User management service".to_string()),
    fingerprint: base_fingerprint,
    protobufs: proto_files.into_iter().map(|pf| {
        actr_protocol::service_spec::Protobuf {
            uri: format!("actr://user-service/{}", pf.name),
            content: pf.content,
            fingerprint: "file_fp".to_string(),
        }
    }).collect(),
};

// Create candidate service with breaking change
let candidate_proto = vec![ProtoFile {
    name: "user.proto".to_string(),
    content: r#"
        syntax = "proto3";
        message User {
            string name = 1;
            // email field removed - this is a breaking change!
            int32 age = 3;  // new field added
        }
    "#.to_string(),
    path: None,
}];

let candidate_fingerprint = Fingerprint::calculate_service_semantic_fingerprint(&candidate_proto)?;

let candidate_service = ServiceSpec {
    version: "1.1.0".to_string(),
    description: Some("User management service".to_string()),
    fingerprint: candidate_fingerprint,
    protobufs: candidate_proto.into_iter().map(|pf| {
        actr_protocol::service_spec::Protobuf {
            uri: format!("actr://user-service/{}", pf.name),
            content: pf.content,
            fingerprint: "file_fp".to_string(),
        }
    }).collect(),
};

// Execute semantic compatibility analysis
let result = ServiceCompatibility::analyze_compatibility(&base_service, &candidate_service)?;

match result.level {
    CompatibilityLevel::FullyCompatible => {
        println!("✅ No changes detected - safe to deploy");
    },
    CompatibilityLevel::BackwardCompatible => {
        println!("⚠️ Backward compatible changes - safe to upgrade");
        for change in &result.changes {
            println!("  - {}: {}", change.change_type, change.description);
        }
    },
    CompatibilityLevel::BreakingChanges => {
        println!("❌ Breaking changes detected - coordinated upgrade required!");
        for breaking_change in &result.breaking_changes {
            println!("  - {}: {}", breaking_change.rule, breaking_change.message);
        }
    }
}
```

## 🎯 Core API

```rust
// Fingerprint calculation
Fingerprint::calculate_proto_semantic_fingerprint(content: &str) -> Result<String>
Fingerprint::calculate_service_semantic_fingerprint(files: &[ProtoFile]) -> Result<String>

// Compatibility analysis
ServiceCompatibility::analyze_compatibility(base: &ServiceSpec, candidate: &ServiceSpec)
    -> Result<CompatibilityAnalysisResult>
ServiceCompatibility::has_breaking_changes(base: &ServiceSpec, candidate: &ServiceSpec)
    -> Result<bool>
ServiceCompatibility::get_breaking_changes(base: &ServiceSpec, candidate: &ServiceSpec)
    -> Result<Vec<BreakingChange>>
ServiceCompatibility::are_semantically_identical(base: &ServiceSpec, candidate: &ServiceSpec)
    -> Result<bool>
```

## 🧪 Testing

The library has **95%+ test coverage** with comprehensive tests for:

- ✅ Error handling and edge cases
- ✅ Breaking change detection
- ✅ File addition/removal scenarios
- ✅ Semantic fingerprint calculation
- ✅ Input validation

```bash
cargo test
```

## 📄 License

Apache-2.0 License - see [LICENSE](LICENSE) file
