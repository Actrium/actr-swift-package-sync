//! Final configuration structures - fully parsed and validated

use actr_protocol::{Acl, ActrType, Realm};
use std::collections::HashMap;
use std::path::PathBuf;
use url::Url;

/// 最终配置（已处理继承、默认值、验证、类型转换）
/// 注意：没有 edition 字段，edition 只作用于解析阶段
#[derive(Debug, Clone)]
pub struct Config {
    /// 包信息
    pub package: PackageInfo,

    /// 导出的 proto 文件（已读取内容）
    pub exports: Vec<ProtoFile>,

    /// 服务依赖（已展开）
    pub dependencies: Vec<Dependency>,

    /// 信令服务器 URL（已验证）
    pub signaling_url: Url,

    /// 所属 Realm (Security Realm)
    pub realm: Realm,

    /// 是否在服务发现中可见
    pub visible_in_discovery: bool,

    /// 访问控制列表
    pub acl: Option<Acl>,

    /// Mailbox 数据库路径
    ///
    /// - `Some(path)`: 使用持久化 SQLite 数据库
    /// - `None`: 使用内存模式 (`:memory:`)
    pub mailbox_path: Option<PathBuf>,

    /// Service tags (e.g., "latest", "stable", "v1.0")
    pub tags: Vec<String>,

    /// 脚本命令
    pub scripts: HashMap<String, String>,

    /// WebRTC 配置
    pub webrtc: WebRtcConfig,

    /// 本节点监听入站 WebSocket 连接的端口（直连模式，可选）
    ///
    /// 配置后节点启动时会在此端口开启 WebSocket 服务端。
    /// 对等节点可直接连接 `ws://<本机IP>:<port>/` 而无需中继。
    pub websocket_listen_port: Option<u16>,

    /// 向信令服务器广播的 WebSocket 主机名或 IP（直连模式，可选）
    ///
    /// 与 `websocket_listen_port` 配合使用。注册时上报到信令服务器，
    /// 使对等节点知道如何直连本节点。默认为 `"127.0.0.1"`（仅适用于本地测试）。
    pub websocket_advertised_host: Option<String>,

    /// Observability configuration (logging + tracing)
    pub observability: ObservabilityConfig,

    /// Directory containing the configuration file (actr.toml)
    /// Used for resolving relative paths and finding lock files
    pub config_dir: PathBuf,
}

/// 包信息
#[derive(Debug, Clone)]
pub struct PackageInfo {
    /// 包名
    pub name: String,

    /// Actor 类型
    pub actr_type: ActrType,

    /// 描述
    pub description: Option<String>,

    /// 作者列表
    pub authors: Vec<String>,

    /// 许可证
    pub license: Option<String>,
}

/// 已解析的 proto 文件（文件级别）
#[derive(Debug, Clone)]
pub struct ProtoFile {
    /// 文件路径（绝对路径）
    pub path: PathBuf,

    /// 文件内容
    pub content: String,
}

/// 已展开的依赖
#[derive(Debug, Clone)]
pub struct Dependency {
    /// 依赖别名（dependencies 中的 key）
    pub alias: String,

    /// 所属 Realm
    pub realm: Realm,

    /// Actor 类型（manufacturer:name:version）
    pub actr_type: Option<ActrType>,

    /// Strict service reference for exact fingerprint matching.
    /// Parsed from `service = "ServiceName:fingerprint"`.
    pub service: Option<ServiceRef>,
}

/// Strict service reference: proto service name + semantic fingerprint.
///
/// When present on a dependency, the runtime only connects to service
/// instances whose registered fingerprint exactly matches.
#[derive(Debug, Clone)]
pub struct ServiceRef {
    /// Proto service name (e.g., "EchoService")
    pub name: String,
    /// Proto semantic fingerprint (e.g., "abc1f3d")
    pub fingerprint: String,
}

/// ICE 传输策略
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum IceTransportPolicy {
    /// 使用所有可用候选（默认）
    #[default]
    All,
    /// 仅使用 TURN 中继候选
    Relay,
}

/// ICE 服务器配置
#[derive(Clone, Debug, Default)]
pub struct IceServer {
    /// 服务器 URL 列表
    pub urls: Vec<String>,
    /// 用户名（TURN 服务器需要）
    pub username: Option<String>,
    /// 凭证（TURN 服务器需要）
    pub credential: Option<String>,
}

/// UDP 端口配置
type UdpPorts = Option<(u16, u16)>;

/// WebRTC 高级参数配置
#[derive(Clone, Debug)]
pub struct WebRtcAdvancedConfig {
    /// UDP 端口策略
    pub udp_ports: UdpPorts,
    /// NAT 1:1 公网 IP 映射
    pub public_ips: Vec<String>,
    /// ICE host 候选等待时间（毫秒）
    pub ice_host_acceptance_min_wait: u64,
    /// ICE srflx 候选等待时间（毫秒）
    pub ice_srflx_acceptance_min_wait: u64,
    /// ICE prflx 候选等待时间（毫秒）
    pub ice_prflx_acceptance_min_wait: u64,
    /// ICE relay 候选等待时间（毫秒）
    pub ice_relay_acceptance_min_wait: u64,
}

impl WebRtcAdvancedConfig {
    /// 检查是否配置了高级参数且优先成为 Answerer
    pub fn prefer_answerer(&self) -> bool {
        // 如果配置了端口范围或 public_ips，则优先成为 Answerer
        self.udp_ports.is_some() || !self.public_ips.is_empty()
    }
}

impl Default for WebRtcAdvancedConfig {
    fn default() -> Self {
        Self {
            udp_ports: UdpPorts::default(),
            public_ips: Vec::new(),
            ice_host_acceptance_min_wait: 0,
            ice_srflx_acceptance_min_wait: 20,
            ice_prflx_acceptance_min_wait: 40,
            ice_relay_acceptance_min_wait: 100,
        }
    }
}

/// WebRTC 配置
#[derive(Clone, Debug, Default)]
pub struct WebRtcConfig {
    /// ICE 服务器列表
    pub ice_servers: Vec<IceServer>,
    /// ICE 传输策略（All 或 Relay）
    pub ice_transport_policy: IceTransportPolicy,
    /// 高级参数配置
    pub advanced: WebRtcAdvancedConfig,
}
/// Observability configuration (logging + tracing) resolved from raw config
#[derive(Debug, Clone)]
pub struct ObservabilityConfig {
    /// Filter level (e.g., "info", "debug", "warn", "info,webrtc=debug").
    /// Used when RUST_LOG environment variable is not set. Default: "info".
    pub filter_level: String,

    /// Whether to enable distributed tracing
    pub tracing_enabled: bool,

    /// OTLP/Jaeger gRPC endpoint
    pub tracing_endpoint: String,

    /// Service name reported to the tracing backend
    pub tracing_service_name: String,
}

// ============================================================================
// Config 辅助方法
// ============================================================================

impl Config {
    /// 获取包的 ActrType（用于注册）
    pub fn actr_type(&self) -> &ActrType {
        &self.package.actr_type
    }

    /// 获取所有 proto 文件路径
    pub fn proto_paths(&self) -> Vec<&PathBuf> {
        self.exports.iter().map(|p| &p.path).collect()
    }

    /// 获取所有 proto 内容（用于计算服务指纹）
    pub fn proto_contents(&self) -> Vec<&str> {
        self.exports.iter().map(|p| p.content.as_str()).collect()
    }

    /// 根据别名查找依赖
    pub fn get_dependency(&self, alias: &str) -> Option<&Dependency> {
        self.dependencies.iter().find(|d| d.alias == alias)
    }

    /// 获取所有跨 Realm 的依赖
    pub fn cross_realm_dependencies(&self) -> Vec<&Dependency> {
        self.dependencies
            .iter()
            .filter(|d| d.realm.realm_id != self.realm.realm_id)
            .collect()
    }

    /// 获取脚本命令
    pub fn get_script(&self, name: &str) -> Option<&str> {
        self.scripts.get(name).map(|s| s.as_str())
    }

    /// 列出所有脚本名称
    pub fn list_scripts(&self) -> Vec<&str> {
        self.scripts.keys().map(|s| s.as_str()).collect()
    }

    /// Calculate ServiceSpec from config
    ///
    /// Returns None if no proto files are exported
    pub fn calculate_service_spec(&self) -> Option<actr_protocol::ServiceSpec> {
        // If no exports, no ServiceSpec
        if self.exports.is_empty() {
            return None;
        }

        // Convert exports to ProtoFile format for fingerprint calculation
        let proto_files: Vec<actr_service_compat::ProtoFile> = self
            .exports
            .iter()
            .map(|export| actr_service_compat::ProtoFile {
                name: export
                    .path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown.proto")
                    .to_string(),
                content: export.content.clone(),
                path: export.path.to_str().map(|s| s.to_string()),
            })
            .collect();

        // Calculate service fingerprint
        let fingerprint =
            actr_service_compat::Fingerprint::calculate_service_semantic_fingerprint(&proto_files)
                .ok()?;

        // Build Protobuf entries
        let protobufs = self
            .exports
            .iter()
            .map(|export| {
                // Calculate individual file fingerprint
                let file_fingerprint =
                    actr_service_compat::Fingerprint::calculate_proto_semantic_fingerprint(
                        &export.content,
                    )
                    .unwrap_or_else(|_| "error".to_string());

                actr_protocol::service_spec::Protobuf {
                    package: export
                        .path
                        .file_stem()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown")
                        .to_string(),
                    content: export.content.clone(),
                    fingerprint: file_fingerprint,
                }
            })
            .collect();

        // Get current timestamp
        let published_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs() as i64;

        Some(actr_protocol::ServiceSpec {
            name: self.package.name.clone(),
            description: self.package.description.clone(),
            fingerprint,
            protobufs,
            published_at: Some(published_at),
            tags: self.tags.clone(),
        })
    }
}

// ============================================================================
// PackageInfo 辅助方法
// ============================================================================

impl PackageInfo {
    /// 获取 manufacturer（ActrType.manufacturer）
    pub fn manufacturer(&self) -> &str {
        &self.actr_type.manufacturer
    }

    /// 获取 type name（ActrType.name）
    pub fn type_name(&self) -> &str {
        &self.actr_type.name
    }
}

// ============================================================================
// Dependency 辅助方法
// ============================================================================

impl Dependency {
    /// 是否跨 Realm 依赖
    pub fn is_cross_realm(&self, self_realm: &Realm) -> bool {
        self.realm.realm_id != self_realm.realm_id
    }

    /// 检查是否要求严格指纹匹配（即 `service` 字段存在）
    pub fn requires_exact_fingerprint(&self) -> bool {
        self.service.is_some()
    }

    /// 检查指纹是否匹配
    ///
    /// - 无 `service` 字段：总是匹配（松散依赖）
    /// - 有 `service` 字段：必须精确匹配
    pub fn matches_fingerprint(&self, fingerprint: &str) -> bool {
        self.service
            .as_ref()
            .map(|s| s.fingerprint == fingerprint)
            .unwrap_or(true)
    }
}

// ============================================================================
// ProtoFile 辅助方法
// ============================================================================

impl ProtoFile {
    /// 获取文件名
    pub fn file_name(&self) -> Option<&str> {
        self.path.file_name()?.to_str()
    }

    /// 获取文件扩展名
    pub fn extension(&self) -> Option<&str> {
        self.path.extension()?.to_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_methods() {
        let config = Config {
            package: PackageInfo {
                name: "test-service".to_string(),
                actr_type: ActrType {
                    manufacturer: "acme".to_string(),
                    name: "test-service".to_string(),
                    version: "1.0.0".to_string(),
                },
                description: None,
                authors: vec![],
                license: None,
            },
            exports: vec![],
            dependencies: vec![
                Dependency {
                    alias: "user-service".to_string(),
                    realm: Realm { realm_id: 1001 },
                    actr_type: Some(ActrType {
                        manufacturer: "acme".to_string(),
                        name: "user-service".to_string(),
                        version: "2.1.0".to_string(),
                    }),
                    service: Some(ServiceRef {
                        name: "UserService".to_string(),
                        fingerprint: "abc123".to_string(),
                    }),
                },
                Dependency {
                    alias: "shared-logger".to_string(),
                    realm: Realm { realm_id: 9999 },
                    actr_type: Some(ActrType {
                        manufacturer: "common".to_string(),
                        name: "logging-service".to_string(),
                        version: "v1".to_string(),
                    }),
                    service: None,
                },
            ],
            signaling_url: Url::parse("ws://localhost:8081").unwrap(),
            realm: Realm { realm_id: 1001 },
            visible_in_discovery: true,
            acl: None,
            mailbox_path: None,
            tags: vec![],
            scripts: HashMap::new(),
            webrtc: WebRtcConfig::default(),
            websocket_listen_port: None,
            websocket_advertised_host: None,
            observability: ObservabilityConfig {
                filter_level: "info".to_string(),
                tracing_enabled: false,
                tracing_endpoint: "http://localhost:4317".to_string(),
                tracing_service_name: "test-service".to_string(),
            },
            config_dir: PathBuf::from("."),
        };

        // 测试依赖查找
        assert!(config.get_dependency("user-service").is_some());
        assert!(config.get_dependency("not-exists").is_none());

        // 测试跨 Realm 依赖
        let cross_realm = config.cross_realm_dependencies();
        assert_eq!(cross_realm.len(), 1);
        assert_eq!(cross_realm[0].alias, "shared-logger");

        // 测试指纹匹配（有 service 字段 = 严格匹配）
        let user_dep = config.get_dependency("user-service").unwrap();
        assert!(user_dep.matches_fingerprint("abc123"));
        assert!(!user_dep.matches_fingerprint("different"));

        // 无 service 字段 = 松散依赖，任何指纹都匹配
        let logger_dep = config.get_dependency("shared-logger").unwrap();
        assert!(logger_dep.matches_fingerprint("any-fingerprint"));
        assert!(!logger_dep.requires_exact_fingerprint());
    }
}
