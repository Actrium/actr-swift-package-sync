//! Raw configuration structures - direct TOML mapping

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// actr.toml 的直接映射（无任何处理）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawConfig {
    /// 配置文件格式版本（决定使用哪个 Parser）
    #[serde(default = "default_edition")]
    pub edition: u32,

    /// 继承的父配置文件路径
    #[serde(default)]
    pub inherit: Option<PathBuf>,

    /// Lock file 文件所在的目录
    #[serde(default)]
    pub config_dir: Option<PathBuf>,

    /// 包信息
    pub package: RawPackageConfig,

    /// 导出的 proto 文件列表
    #[serde(default)]
    pub exports: Vec<PathBuf>,

    /// 服务依赖
    #[serde(default)]
    pub dependencies: HashMap<String, RawDependency>,

    /// 系统配置
    #[serde(default)]
    pub system: RawSystemConfig,

    /// 访问控制列表（原始 TOML 值，稍后解析）
    #[serde(default)]
    pub acl: Option<toml::Value>,

    /// 脚本命令
    #[serde(default)]
    pub scripts: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawPackageConfig {
    pub name: String,
    pub actr_type: RawActrType,

    #[serde(default)]
    pub description: Option<String>,

    #[serde(default)]
    pub authors: Option<Vec<String>>,

    #[serde(default)]
    pub license: Option<String>,

    /// Service tags (e.g., ["latest", "stable", "v1.0"])
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Actor type configuration under [package.actr_type]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawActrType {
    pub manufacturer: String,
    pub name: String,
    /// Semantic version (e.g., "1.0.0"). Defaults to empty string if not specified.
    #[serde(default)]
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RawDependency {
    /// 已指定 ActrType 的依赖（必须先匹配，因为它有 required 的 `actr_type` 字段）
    ///
    /// Example:
    /// ```toml
    /// [dependencies]
    /// echo = { actr_type = "acme:echo-service:1.0.0", service = "EchoService:abc1f3d" }
    /// ```
    Specified {
        /// Full ActrType string: "manufacturer:name:version"
        #[serde(rename = "actr_type")]
        actr_type: String,

        /// Optional strict service reference: "ServiceName:fingerprint".
        /// When present, enables exact proto fingerprint matching.
        #[serde(default)]
        service: Option<String>,

        /// Optional cross-realm override. Defaults to self realm.
        #[serde(default)]
        realm: Option<u32>,
    },

    /// 空依赖声明：{}（由 actr install 填充）
    Empty {},
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawSystemConfig {
    #[serde(default)]
    pub signaling: RawSignalingConfig,

    #[serde(default)]
    pub deployment: RawDeploymentConfig,

    #[serde(default)]
    pub discovery: RawDiscoveryConfig,

    #[serde(default)]
    pub storage: RawStorageConfig,

    #[serde(default)]
    pub webrtc: RawWebRtcConfig,
    #[serde(default)]
    pub websocket: RawWebSocketConfig,
    #[serde(default)]
    pub observability: RawObservabilityConfig,
}

/// WebSocket 数据传输配置
///
/// 配置示例（actr.toml）：
/// ```toml
/// [system.websocket]
/// listen_port = 9001
/// advertised_host = "192.168.1.10"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawWebSocketConfig {
    /// 本节点监听入站 WebSocket 连接的端口（用于直连模式）
    ///
    /// 配置后本节点会在此端口启动 WebSocket 服务端，接受对等节点的直接连接。
    /// 不配置时本节点不监听任何端口（仅支持中继模式）。
    #[serde(default)]
    pub listen_port: Option<u16>,

    /// 对外广播的 WebSocket 主机名或 IP（用于信令注册）
    ///
    /// 当节点配置了 `listen_port` 时，信令服务器需要一个可被对等节点访问的地址。
    /// 此字段指定注册到信令服务器的主机名或 IP，例如 `"192.168.1.10"` 或
    /// `"mynode.example.com"`。若不配置，默认使用 `"127.0.0.1"`（仅适用于本地测试）。
    #[serde(default)]
    pub advertised_host: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawSignalingConfig {
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawDeploymentConfig {
    #[serde(default)]
    pub realm_id: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawDiscoveryConfig {
    #[serde(default)]
    pub visible: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawStorageConfig {
    #[serde(default)]
    pub mailbox_path: Option<PathBuf>,
}

/// WebRTC 配置
///
/// 不配置端口范围时使用默认模式（随机端口）
/// 配置 port_range_start/end 时启用固定端口模式
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawWebRtcConfig {
    /// STUN 服务器 URL 列表 (例如 ["stun:localhost:3478"])
    #[serde(default)]
    pub stun_urls: Vec<String>,

    /// TURN 服务器 URL 列表 (例如 ["turn:localhost:3478"])
    #[serde(default)]
    pub turn_urls: Vec<String>,

    /// 是否强制使用 TURN 中继 (默认 false)
    #[serde(default)]
    pub force_relay: bool,

    /// ICE host 候选等待时间（毫秒）
    #[serde(default)]
    pub ice_host_acceptance_min_wait: Option<u64>,

    /// ICE srflx 候选等待时间（毫秒）
    #[serde(default)]
    pub ice_srflx_acceptance_min_wait: Option<u64>,

    /// ICE prflx 候选等待时间（毫秒）
    #[serde(default)]
    pub ice_prflx_acceptance_min_wait: Option<u64>,

    /// ICE relay 候选等待时间（毫秒）
    #[serde(default)]
    pub ice_relay_acceptance_min_wait: Option<u64>,

    /// UDP 端口范围起始值（可选，配置后启用固定端口模式）
    #[serde(default)]
    pub port_range_start: Option<u16>,

    /// UDP 端口范围结束值（可选，配置后启用固定端口模式）
    #[serde(default)]
    pub port_range_end: Option<u16>,

    /// NAT 1:1 公网 IP 映射（可选）
    #[serde(default)]
    pub public_ips: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RawObservabilityConfig {
    /// Filter level (e.g., "info", "debug", "warn", "info,webrtc=debug").
    /// Used when RUST_LOG environment variable is not set. Default: "info".
    #[serde(default)]
    pub filter_level: Option<String>,

    #[serde(default)]
    pub tracing_enabled: Option<bool>,

    /// OTLP/Jaeger gRPC endpoint. Default: http://localhost:4317
    #[serde(default)]
    pub tracing_endpoint: Option<String>,

    /// Service name reported to the tracing backend. Default: package.name
    #[serde(default)]
    pub tracing_service_name: Option<String>,
}

fn default_edition() -> u32 {
    1
}

impl RawConfig {
    /// 从文件加载原始配置
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        content.parse()
    }

    /// 保存到文件
    pub fn save_to_file(&self, path: impl AsRef<Path>) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

impl FromStr for RawConfig {
    type Err = crate::error::ConfigError;

    fn from_str(s: &str) -> Result<Self> {
        toml::from_str(s).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_webrtc_with_port_range() {
        let toml_content = r#"
edition = 1
[package]
name = "test"
[package.actr_type]
manufacturer = "acme"
name = "test"
[system.webrtc]
port_range_start = 50000
port_range_end = 50100
public_ips = ["1.2.3.4"]
turn_urls = ["turn:Example"]
"#;
        let config = RawConfig::from_str(toml_content).unwrap();
        assert_eq!(config.system.webrtc.port_range_start, Some(50000));
        assert_eq!(config.system.webrtc.port_range_end, Some(50100));
        assert_eq!(config.system.webrtc.public_ips[0], "1.2.3.4");
        assert_eq!(config.system.webrtc.turn_urls[0], "turn:Example");
    }

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

        let config = RawConfig::from_str(toml_content).unwrap();
        assert_eq!(config.edition, 1);
        assert_eq!(config.package.name, "test-service");
        assert_eq!(config.exports.len(), 1);
        assert!(config.dependencies.contains_key("user-service"));
    }

    #[test]
    fn test_parse_dependency_with_empty_attributes() {
        let toml_content = r#"
[package]
name = "test"
[package.actr_type]
manufacturer = "acme"
name = "test"
[dependencies]
user-service = {}
"#;
        let config = RawConfig::from_str(toml_content).unwrap();
        let dep = config.dependencies.get("user-service").unwrap();
        assert!(matches!(dep, RawDependency::Empty {}));
    }

    #[test]
    fn test_parse_dependency_specified() {
        let toml_content = r#"
[package]
name = "test"
[package.actr_type]
manufacturer = "acme"
name = "test"
[dependencies]
shared = { actr_type = "acme:logging-service:1.0.0", service = "LoggingService:abc123", realm = 9999 }
"#;
        let config = RawConfig::from_str(toml_content).unwrap();
        let dep = config.dependencies.get("shared").unwrap();
        if let RawDependency::Specified {
            actr_type,
            service,
            realm,
        } = dep
        {
            assert_eq!(actr_type, "acme:logging-service:1.0.0");
            assert_eq!(service.as_deref(), Some("LoggingService:abc123"));
            assert_eq!(*realm, Some(9999));
        } else {
            panic!("Expected Specified");
        }
    }

    #[test]
    fn test_parse_dependency_specified_no_service() {
        let toml_content = r#"
[package]
name = "test"
[package.actr_type]
manufacturer = "acme"
name = "test"
[dependencies]
shared = { actr_type = "acme:logging-service:1.0.0" }
"#;
        let config = RawConfig::from_str(toml_content).unwrap();
        let dep = config.dependencies.get("shared").unwrap();
        if let RawDependency::Specified { service, .. } = dep {
            assert!(service.is_none());
        } else {
            panic!("Expected Specified");
        }
    }
}
