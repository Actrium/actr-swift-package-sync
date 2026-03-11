//! Edition 1 configuration parser

use crate::config::ObservabilityConfig;
use crate::config::{
    Config, Dependency, IceServer, IceTransportPolicy, PackageInfo, ProtoFile, ServiceRef,
    WebRtcAdvancedConfig, WebRtcConfig,
};

use crate::error::{ConfigError, Result};
use crate::{RawConfig, RawDependency, RawPackageConfig, RawSystemConfig};
use actr_protocol::{Acl, ActrType, Name, Realm};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use url::Url;

const DEFAULT_TRACING_ENDPOINT: &str = "http://localhost:4317";

/// Edition 1 格式的解析器
pub struct ParserV1 {
    base_dir: PathBuf,
}

impl ParserV1 {
    pub fn new(config_path: impl AsRef<Path>) -> Self {
        let base_dir = config_path
            .as_ref()
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        Self { base_dir }
    }

    pub fn parse(&self, mut raw: RawConfig) -> Result<Config> {
        // 1. 处理继承
        let raw = if let Some(parent_path) = raw.inherit.take() {
            self.merge_inheritance(raw, parent_path)?
        } else {
            raw
        };

        // 2. 验证必需字段
        self.validate_required_fields(&raw)?;

        // 3. 解析 package
        let package = self.parse_package(&raw.package)?;

        // 4. 解析 exports
        let exports = self.parse_exports(&raw.exports)?;

        // 5. 获取 realm
        let self_realm = Realm {
            realm_id: raw
                .system
                .deployment
                .realm_id
                .ok_or(ConfigError::MissingField("system.deployment.realm"))?,
        };

        // 6. 解析 dependencies
        let dependencies = self.parse_dependencies(&raw.dependencies, &self_realm)?;

        // 7. 解析 signaling URL
        let signaling_url_str = raw
            .system
            .signaling
            .url
            .as_ref()
            .ok_or(ConfigError::MissingField("system.signaling.url"))?;

        let signaling_url = Url::parse(signaling_url_str).map_err(ConfigError::InvalidUrl)?;

        // 8. 解析 observability 配置
        let observability = self.parse_observability(&raw.system, &package);

        // 10. 解析 ACL (从顶级 acl 读取，放在最后以避免 partial move)
        let acl = if let Some(acl_value) = raw.acl {
            Some(self.parse_acl(acl_value, self_realm.realm_id)?)
        } else {
            None
        };

        // 11. 确定 config_dir
        // 如果 raw.config_dir 存在，则相对于当前 base_dir 解析
        let config_dir = if let Some(dir) = raw.config_dir {
            self.base_dir.join(dir)
        } else {
            self.base_dir.clone()
        };

        // 12. 构建最终配置
        Ok(Config {
            package,
            exports,
            dependencies,
            signaling_url,
            realm: self_realm,
            visible_in_discovery: raw.system.discovery.visible.unwrap_or(true),
            acl,
            mailbox_path: raw.system.storage.mailbox_path,
            tags: raw.package.tags,
            scripts: raw.scripts,
            webrtc: self.parse_webrtc(&raw.system.webrtc)?,
            websocket_listen_port: raw.system.websocket.listen_port,
            websocket_advertised_host: raw.system.websocket.advertised_host.clone(),
            observability,
            config_dir,
        })
    }

    fn parse_package(&self, raw: &RawPackageConfig) -> Result<PackageInfo> {
        Name::new(raw.actr_type.manufacturer.clone()).map_err(|e| {
            ConfigError::InvalidActrType(format!(
                "Invalid manufacturer name '{}': {}",
                raw.actr_type.manufacturer, e
            ))
        })?;

        Name::new(raw.actr_type.name.clone()).map_err(|e| {
            ConfigError::InvalidActrType(format!(
                "Invalid actor type name '{}': {}",
                raw.actr_type.name, e
            ))
        })?;

        let actr_type = ActrType {
            manufacturer: raw.actr_type.manufacturer.clone(),
            name: raw.actr_type.name.clone(),
            version: raw.actr_type.version.clone(),
        };

        Ok(PackageInfo {
            name: raw.name.clone(),
            actr_type,
            description: raw.description.clone(),
            authors: raw.authors.clone().unwrap_or_default(),
            license: raw.license.clone(),
        })
    }

    fn parse_exports(&self, paths: &[PathBuf]) -> Result<Vec<ProtoFile>> {
        paths
            .iter()
            .map(|path| {
                let full_path = self.base_dir.join(path);
                let content = std::fs::read_to_string(&full_path)
                    .map_err(|e| ConfigError::ProtoFileNotFound(full_path.clone(), e))?;
                Ok(ProtoFile {
                    path: full_path,
                    content,
                })
            })
            .collect()
    }

    fn parse_dependencies(
        &self,
        deps: &HashMap<String, RawDependency>,
        self_realm: &Realm,
    ) -> Result<Vec<Dependency>> {
        deps.iter()
            .map(|(alias, raw_dep)| match raw_dep {
                RawDependency::Empty {} => Ok(Dependency {
                    alias: alias.clone(),
                    realm: *self_realm,
                    actr_type: None,
                    service: None,
                }),
                RawDependency::Specified {
                    actr_type: actr_type_str,
                    service,
                    realm,
                } => {
                    let actr_type = self.parse_actr_type(actr_type_str)?;
                    let service_ref = service
                        .as_deref()
                        .map(|s| self.parse_service_ref(s))
                        .transpose()?;
                    let realm = Realm {
                        realm_id: realm.unwrap_or(self_realm.realm_id),
                    };
                    Ok(Dependency {
                        alias: alias.clone(),
                        realm,
                        actr_type: Some(actr_type),
                        service: service_ref,
                    })
                }
            })
            .collect()
    }

    /// Parse `"ServiceName:fingerprint"` into a `ServiceRef`.
    fn parse_service_ref(&self, s: &str) -> Result<ServiceRef> {
        let (name, fingerprint) = s.split_once(':').ok_or_else(|| {
            ConfigError::InvalidActrType(format!(
                "Invalid service reference '{}': expected 'ServiceName:fingerprint'",
                s
            ))
        })?;
        Ok(ServiceRef {
            name: name.to_string(),
            fingerprint: fingerprint.to_string(),
        })
    }

    /// Parse an ActrType string: `"manufacturer:name:version"`.
    ///
    /// The version segment is required. For backward compatibility with configs
    /// that omit it, the empty string is accepted and stored as-is.
    fn parse_actr_type(&self, s: &str) -> Result<ActrType> {
        let parts: Vec<&str> = s.splitn(4, ':').collect();
        let (manufacturer, name, version) = match parts.as_slice() {
            [m, n] => (*m, *n, ""),
            [m, n, v] => (*m, *n, *v),
            _ => {
                return Err(ConfigError::InvalidActrType(format!(
                    "Invalid actor type '{}': expected 'manufacturer:name:version'",
                    s
                )));
            }
        };

        Name::new(manufacturer.to_string()).map_err(|e| {
            ConfigError::InvalidActrType(format!(
                "Invalid manufacturer '{}' in '{}': {}",
                manufacturer, s, e
            ))
        })?;
        Name::new(name.to_string()).map_err(|e| {
            ConfigError::InvalidActrType(format!("Invalid type name '{}' in '{}': {}", name, s, e))
        })?;

        Ok(ActrType {
            manufacturer: manufacturer.to_string(),
            name: name.to_string(),
            version: version.to_string(),
        })
    }

    fn parse_acl(&self, value: toml::Value, self_realm_id: u32) -> Result<Acl> {
        use actr_protocol::AclRule;
        use actr_protocol::acl_rule::{Permission, SourceRealm};

        let rules_array = value
            .get("rules")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ConfigError::InvalidAcl("ACL must have 'rules' array".to_string()))?;

        let mut rules = Vec::new();

        for (idx, rule_value) in rules_array.iter().enumerate() {
            let rule_table = rule_value.as_table().ok_or_else(|| {
                ConfigError::InvalidAcl(format!("ACL rule {} must be a table", idx))
            })?;

            // permission (required)
            let permission_str = rule_table
                .get("permission")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    ConfigError::InvalidAcl(format!("ACL rule {} missing 'permission' field", idx))
                })?;
            let permission = match permission_str.to_uppercase().as_str() {
                "ALLOW" => Permission::Allow as i32,
                "DENY" => Permission::Deny as i32,
                _ => {
                    return Err(ConfigError::InvalidAcl(format!(
                        "ACL rule {}: invalid permission '{}', expected 'ALLOW' or 'DENY'",
                        idx, permission_str
                    )));
                }
            };

            // type (required): "manufacturer:name:version"
            let type_str = rule_table
                .get("type")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    ConfigError::InvalidAcl(format!("ACL rule {} missing 'type' field", idx))
                })?;
            let from_type = self.parse_actr_type(type_str)?;

            // realm (optional): omitted/"self" → self_realm_id, "*" → any, integer → specific
            let source_realm = match rule_table.get("realm") {
                None => Some(SourceRealm::RealmId(self_realm_id)),
                Some(v) => match v.as_str() {
                    Some("self") => Some(SourceRealm::RealmId(self_realm_id)),
                    Some("*") => Some(SourceRealm::AnyRealm(true)),
                    Some(other) => {
                        return Err(ConfigError::InvalidAcl(format!(
                            "ACL rule {}: invalid realm '{}', expected 'self', '*', or an integer",
                            idx, other
                        )));
                    }
                    None => match v.as_integer() {
                        Some(id) => Some(SourceRealm::RealmId(id as u32)),
                        None => {
                            return Err(ConfigError::InvalidAcl(format!(
                                "ACL rule {}: realm must be 'self', '*', or an integer",
                                idx
                            )));
                        }
                    },
                },
            };

            rules.push(AclRule {
                permission,
                from_type,
                source_realm,
            });
        }

        Ok(Acl { rules })
    }

    fn parse_webrtc(&self, raw: &crate::raw::RawWebRtcConfig) -> Result<WebRtcConfig> {
        let mut ice_servers = Vec::new();

        // 解析 STUN URLs
        if !raw.stun_urls.is_empty() {
            ice_servers.push(IceServer {
                urls: raw.stun_urls.clone(),
                username: None,
                credential: None,
            });
        }

        // 解析 TURN URLs（凭证在运行时动态生成）
        if !raw.turn_urls.is_empty() {
            ice_servers.push(IceServer {
                urls: raw.turn_urls.clone(),
                username: None,
                credential: None,
            });
        }

        // 解析 ICE 传输策略
        let ice_transport_policy = if raw.force_relay {
            IceTransportPolicy::Relay
        } else {
            IceTransportPolicy::All
        };

        // 解析端口配置
        let (udp_ports, public_ips) =
            if let (Some(start), Some(end)) = (raw.port_range_start, raw.port_range_end) {
                // 配置了端口范围，启用固定端口模式
                if start >= end {
                    // 端口范围无效，抛出错误
                    return Err(ConfigError::InvalidConfig(format!(
                        "Invalid port range: start ({}) must be less than end ({})",
                        start, end
                    )));
                } else {
                    // 端口范围模式
                    (Some((start, end)), raw.public_ips.clone())
                }
            } else {
                // 未配置端口范围，使用默认模式（随机端口）
                (None, Vec::new())
            };

        // 解析 ICE 等待时间
        let ice_host_acceptance_min_wait = raw.ice_host_acceptance_min_wait.unwrap_or(0);
        let ice_srflx_acceptance_min_wait = raw.ice_srflx_acceptance_min_wait.unwrap_or(20);
        let ice_prflx_acceptance_min_wait = raw.ice_prflx_acceptance_min_wait.unwrap_or(40);
        let ice_relay_acceptance_min_wait = raw.ice_relay_acceptance_min_wait.unwrap_or(100);

        let advanced = WebRtcAdvancedConfig {
            udp_ports,
            public_ips,
            ice_host_acceptance_min_wait,
            ice_srflx_acceptance_min_wait,
            ice_prflx_acceptance_min_wait,
            ice_relay_acceptance_min_wait,
        };

        Ok(WebRtcConfig {
            ice_servers,
            ice_transport_policy,
            advanced,
        })
    }

    fn parse_observability(
        &self,
        raw_system: &RawSystemConfig,
        package: &PackageInfo,
    ) -> ObservabilityConfig {
        ObservabilityConfig {
            filter_level: raw_system
                .observability
                .filter_level
                .clone()
                .unwrap_or_else(|| "info".to_string()),
            tracing_enabled: raw_system.observability.tracing_enabled.unwrap_or(false),
            tracing_endpoint: raw_system
                .observability
                .tracing_endpoint
                .clone()
                .unwrap_or_else(|| DEFAULT_TRACING_ENDPOINT.to_string()),
            tracing_service_name: raw_system
                .observability
                .tracing_service_name
                .clone()
                .unwrap_or_else(|| package.name.clone()),
        }
    }

    fn merge_inheritance(&self, child: RawConfig, parent_path: PathBuf) -> Result<RawConfig> {
        let parent_full_path = self.base_dir.join(&parent_path);
        let mut parent = RawConfig::from_file(&parent_full_path)?;

        // 检查 edition 一致性
        if parent.edition != child.edition {
            return Err(ConfigError::EditionMismatch {
                parent: parent.edition,
                child: child.edition,
            });
        }

        // 递归处理父配置的继承
        let parent = if let Some(grandparent) = parent.inherit.take() {
            self.merge_inheritance(parent, grandparent)?
        } else {
            parent
        };

        // 合并逻辑
        Ok(RawConfig {
            edition: child.edition, // 已验证一致
            inherit: None,
            config_dir: child.config_dir,
            package: child.package, // package 不继承
            exports: {
                let mut p = parent.exports;
                p.extend(child.exports);
                p
            },
            dependencies: {
                let mut d = parent.dependencies;
                d.extend(child.dependencies);
                d
            },
            system: self.merge_system_config(parent.system, child.system),
            acl: child.acl.or(parent.acl),
            scripts: {
                let mut s = parent.scripts;
                s.extend(child.scripts);
                s
            },
        })
    }

    fn merge_system_config(
        &self,
        parent: RawSystemConfig,
        child: RawSystemConfig,
    ) -> RawSystemConfig {
        RawSystemConfig {
            signaling: crate::raw::RawSignalingConfig {
                url: child.signaling.url.or(parent.signaling.url),
            },
            deployment: crate::raw::RawDeploymentConfig {
                realm_id: child.deployment.realm_id.or(parent.deployment.realm_id),
            },
            discovery: crate::raw::RawDiscoveryConfig {
                visible: child.discovery.visible.or(parent.discovery.visible),
            },
            storage: crate::raw::RawStorageConfig {
                mailbox_path: child.storage.mailbox_path.or(parent.storage.mailbox_path),
            },
            webrtc: crate::raw::RawWebRtcConfig {
                stun_urls: if child.webrtc.stun_urls.is_empty() {
                    parent.webrtc.stun_urls
                } else {
                    child.webrtc.stun_urls
                },
                turn_urls: if child.webrtc.turn_urls.is_empty() {
                    parent.webrtc.turn_urls
                } else {
                    child.webrtc.turn_urls
                },
                force_relay: child.webrtc.force_relay || parent.webrtc.force_relay,
                ice_host_acceptance_min_wait: child
                    .webrtc
                    .ice_host_acceptance_min_wait
                    .or(parent.webrtc.ice_host_acceptance_min_wait),
                ice_srflx_acceptance_min_wait: child
                    .webrtc
                    .ice_srflx_acceptance_min_wait
                    .or(parent.webrtc.ice_srflx_acceptance_min_wait),
                ice_prflx_acceptance_min_wait: child
                    .webrtc
                    .ice_prflx_acceptance_min_wait
                    .or(parent.webrtc.ice_prflx_acceptance_min_wait),
                ice_relay_acceptance_min_wait: child
                    .webrtc
                    .ice_relay_acceptance_min_wait
                    .or(parent.webrtc.ice_relay_acceptance_min_wait),
                port_range_start: child
                    .webrtc
                    .port_range_start
                    .or(parent.webrtc.port_range_start),
                port_range_end: child.webrtc.port_range_end.or(parent.webrtc.port_range_end),
                public_ips: if child.webrtc.public_ips.is_empty() {
                    parent.webrtc.public_ips
                } else {
                    child.webrtc.public_ips
                },
            },
            observability: crate::raw::RawObservabilityConfig {
                filter_level: child
                    .observability
                    .filter_level
                    .or(parent.observability.filter_level.clone()),
                tracing_enabled: child
                    .observability
                    .tracing_enabled
                    .or(parent.observability.tracing_enabled),
                tracing_endpoint: child
                    .observability
                    .tracing_endpoint
                    .or(parent.observability.tracing_endpoint),
                tracing_service_name: child
                    .observability
                    .tracing_service_name
                    .or(parent.observability.tracing_service_name),
            },
            websocket: crate::raw::RawWebSocketConfig {
                listen_port: child.websocket.listen_port.or(parent.websocket.listen_port),
                advertised_host: child
                    .websocket
                    .advertised_host
                    .or(parent.websocket.advertised_host),
            },
        }
    }

    fn validate_required_fields(&self, raw: &RawConfig) -> Result<()> {
        if raw.system.signaling.url.is_none() {
            return Err(ConfigError::MissingField("system.signaling.url"));
        }
        if raw.system.deployment.realm_id.is_none() {
            return Err(ConfigError::MissingField("system.deployment.realm"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RawConfig;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_parse_basic_config() {
        let toml_content = r#"
edition = 1
exports = ["proto/test.proto"]

[package]
name = "test-service"

[package.actr_type]
manufacturer = "acme"
name = "test-service"

[dependencies]
user-service = {}

[system.signaling]
url = "ws://localhost:8081"

[system.deployment]
realm_id = 1001

[scripts]
run = "cargo run"
"#;

        let tmpdir = TempDir::new().unwrap();
        let config_path = tmpdir.path().join("actr.toml");

        // 创建 proto 文件
        let proto_dir = tmpdir.path().join("proto");
        fs::create_dir_all(&proto_dir).unwrap();
        fs::write(proto_dir.join("test.proto"), "syntax = \"proto3\";").unwrap();

        // 写入配置
        fs::write(&config_path, toml_content).unwrap();

        // 解析
        let raw = RawConfig::from_file(&config_path).unwrap();
        let parser = ParserV1::new(&config_path);
        let config = parser.parse(raw).unwrap();

        assert_eq!(config.package.name, "test-service");
        assert_eq!(config.realm.realm_id, 1001);
        assert_eq!(config.dependencies.len(), 1);
        assert_eq!(config.exports.len(), 1);
    }

    #[test]
    fn test_parse_cross_realm_dependency() {
        let toml_content = r#"
edition = 1

[package]
name = "test"

[package.actr_type]
manufacturer = "acme"
name = "test"
version = "1.0.0"

[dependencies]
shared = { actr_type = "acme:logging-service:1.0.0", service = "LoggingService:abc123", realm = 9999 }

[system.signaling]
url = "ws://localhost:8081"

[system.deployment]
realm_id = 1001
"#;

        let tmpdir = TempDir::new().unwrap();
        let config_path = tmpdir.path().join("actr.toml");
        fs::write(&config_path, toml_content).unwrap();

        let raw = RawConfig::from_file(&config_path).unwrap();
        let parser = ParserV1::new(&config_path);
        let config = parser.parse(raw).unwrap();

        let dep = config.get_dependency("shared").unwrap();
        assert_eq!(dep.alias, "shared");
        assert_eq!(dep.realm.realm_id, 9999);
        assert_eq!(dep.actr_type.as_ref().unwrap().name, "logging-service");
        assert_eq!(dep.service.as_ref().unwrap().name, "LoggingService");
        assert_eq!(dep.service.as_ref().unwrap().fingerprint, "abc123");
        assert!(dep.is_cross_realm(&config.realm));
    }

    #[test]
    fn test_validate_actr_type_name() {
        // Test invalid manufacturer name (starts with number)
        let toml_content = r#"
edition = 1

[package]
name = "test"

[package.actr_type]
manufacturer = "1acme"
name = "test"

[system.signaling]
url = "ws://localhost:8081"

[system.deployment]
realm_id = 1001
"#;

        let tmpdir = TempDir::new().unwrap();
        let config_path = tmpdir.path().join("actr.toml");
        fs::write(&config_path, toml_content).unwrap();

        let raw = RawConfig::from_file(&config_path).unwrap();
        let parser = ParserV1::new(&config_path);
        let result = parser.parse(raw);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigError::InvalidActrType(_)
        ));
    }

    #[test]
    fn test_validate_actr_type_name_invalid() {
        // Test invalid actor type name (ends with hyphen)
        let toml_content = r#"
edition = 1

[package]
name = "test"

[package.actr_type]
manufacturer = "acme"
name = "test-"

[system.signaling]
url = "ws://localhost:8081"

[system.deployment]
realm_id = 1001
"#;

        let tmpdir = TempDir::new().unwrap();
        let config_path = tmpdir.path().join("actr.toml");
        fs::write(&config_path, toml_content).unwrap();

        let raw = RawConfig::from_file(&config_path).unwrap();
        let parser = ParserV1::new(&config_path);
        let result = parser.parse(raw);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigError::InvalidActrType(_)
        ));
    }

    #[test]
    fn test_invalid_port_range() {
        // Test invalid port range (start > end)
        let toml_content = r#"
edition = 1

[package]
name = "test"

[package.actr_type]
manufacturer = "acme"
name = "test"

[system.signaling]
url = "ws://localhost:8081"

[system.deployment]
realm_id = 1001

[system.webrtc]
port_range_start = 50100
port_range_end = 50000
"#;

        let tmpdir = TempDir::new().unwrap();
        let config_path = tmpdir.path().join("actr.toml");
        fs::write(&config_path, toml_content).unwrap();

        let raw = RawConfig::from_file(&config_path).unwrap();
        let parser = ParserV1::new(&config_path);
        let result = parser.parse(raw);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ConfigError::InvalidConfig(_)));
    }
}
