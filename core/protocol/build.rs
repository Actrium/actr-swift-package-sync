use std::path::PathBuf;

use glob::glob;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let generated_dir = PathBuf::from(std::env::var("OUT_DIR").map_err(
        |_| "OUT_DIR environment variable is not set. This script must be run by cargo.",
    )?);

    let has_uniffi = std::env::var_os("CARGO_FEATURE_UNIFFI").is_some();

    // Collect all .proto files using glob
    let proto_files: Vec<String> = glob("proto/**/*.proto")?
        .filter_map(Result::ok)
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    if proto_files.is_empty() {
        return Err("No proto files found".into());
    }

    // Configure prost-build for proto2 files
    let mut config = prost_build::Config::new();
    config
        .out_dir(&generated_dir)
        // Use bytes::Bytes instead of Vec<u8> for better zero-copy performance
        .bytes(["."])
        .type_attribute(
            "RegisterResponse.result",
            "#[allow(clippy::large_enum_variant)]",
        );

    // Add serde and Ord/PartialOrd for core identity types
    // (used in HashMaps, BTreeMaps, and serialized outside of protobuf)
    let base_attr = "#[derive(Ord, PartialOrd, serde::Serialize, serde::Deserialize";
    let attr_suffix = if has_uniffi {
        ", uniffi::Record)]"
    } else {
        ")]"
    };
    let full_attr = format!("{base_attr}{attr_suffix}");

    for type_name in ["ActrId", "ActrType", "Realm"] {
        config.type_attribute(type_name, &full_attr);
    }

    config.compile_protos(&proto_files, &["proto/"])?;

    // Tell cargo to rerun if proto files change
    println!("cargo:rerun-if-changed=proto/");

    Ok(())
}
