//! Example showing how to parse an actr.toml configuration file

use actr_config::ConfigParser;
use actr_protocol::ActrTypeExt;
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Get config path from command line or use default
    let config_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "../actr.toml".to_string());

    println!("🔧 Loading configuration from: {config_path}");

    // Load and parse the configuration
    let config = ConfigParser::from_file(&config_path)?;

    println!("\n📦 Package Information:");
    println!("  Name: {}", config.package.name);
    println!("  Type: {}", config.package.actr_type.to_string_repr());
    if let Some(ref description) = config.package.description {
        println!("  Description: {description}");
    }
    if !config.package.authors.is_empty() {
        println!("  Authors: {}", config.package.authors.join(", "));
    }
    if let Some(ref license) = config.package.license {
        println!("  License: {license}");
    }

    println!("\n📦 Exports:");
    if config.exports.is_empty() {
        println!("  (none)");
    } else {
        for proto in &config.exports {
            println!("  {}", proto.path.display());
        }
    }

    println!("\n📚 Dependencies:");
    if config.dependencies.is_empty() {
        println!("  (none)");
    } else {
        for dep in &config.dependencies {
            let service_str = if let Some(ref s) = dep.service {
                format!("{}:{}", s.name, s.fingerprint)
            } else {
                "*".to_string()
            };
            let type_str = if let Some(ref t) = dep.actr_type {
                t.to_string_repr()
            } else {
                "*".to_string()
            };
            println!("  {} ({}) service={}", dep.alias, type_str, service_str);
        }
    }

    println!("\n📜 Scripts:");
    if config.scripts.is_empty() {
        println!("  (none)");
    } else {
        for (name, command) in &config.scripts {
            println!("  {name}: {command}");
        }
    }

    println!("\n🌐 System Configuration:");
    println!("  Signaling URL: {}", config.signaling_url);
    println!("  Realm ID: {}", config.realm.realm_id);
    println!("  Visible in discovery: {}", config.visible_in_discovery);

    println!("\n✅ Configuration successfully parsed and validated!");

    Ok(())
}
