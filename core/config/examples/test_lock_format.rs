//! Example showing lock file format with proto file references

use actr_config::lock::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a lock file with proto file references
    let mut lock_file = LockFile::new();

    let spec_meta = ServiceSpecMeta {
        name: "user-service".to_string(),
        description: Some("User management service".to_string()),
        fingerprint: "service_semantic:a1b2c3d4e5f6".to_string(),
        protobufs: vec![
            ProtoFileMeta {
                path: "user-service/user.v1.proto".to_string(),
                fingerprint: "semantic:xyz123".to_string(),
            },
            ProtoFileMeta {
                path: "user-service/common.v1.proto".to_string(),
                fingerprint: "semantic:abc789".to_string(),
            },
        ],
        published_at: Some(1705315800),
        tags: vec!["latest".to_string(), "stable".to_string()],
    };

    let dep = LockedDependency::new("acme+user-service".to_string(), spec_meta);

    lock_file.add_dependency(dep);

    // Serialize to TOML
    println!("=== Generated actr.lock.toml ===\n");
    let toml_str = toml::to_string_pretty(&lock_file)?;
    println!("{toml_str}");

    // Verify round-trip
    println!("\n=== Verifying round-trip ===");
    let restored: LockFile = toml::from_str(&toml_str)?;
    println!("✓ Successfully parsed back");
    println!("✓ Dependencies: {}", restored.dependencies.len());
    println!("✓ Proto files: {}", restored.dependencies[0].files.len());

    // Check path
    let user_proto = &restored.dependencies[0].files[0];
    println!("✓ First proto path: {}", user_proto.path);
    println!("✓ Fingerprint: {}", user_proto.fingerprint);

    Ok(())
}
