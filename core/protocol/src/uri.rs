//! # Actor-RTC URI 解析库
//!
//! 提供 actr:// 协议的标准 URI 解析功能，不包含业务逻辑。
//!
//! URI 格式: actr://<realm>:<manufacturer>+<name>@<version>
//! 例如: actr://101:acme+echo-service@v1

use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use thiserror::Error;

/// Actor-RTC URI 解析错误
#[derive(Error, Debug)]
pub enum ActrUriError {
    #[error("Invalid URI scheme, expected 'actr' but got '{0}'")]
    InvalidScheme(String),

    #[error("Missing actor authority in URI")]
    MissingAuthority,

    #[error("Invalid actor authority format, expected: <realm>:<manufacturer>+<name>@<version>")]
    InvalidAuthorityFormat(String),

    #[error("Missing version suffix '@v1' in URI")]
    MissingVersion,

    #[error("Invalid realm ID: {0}")]
    InvalidRealmId(String),

    #[error("URI parse error: {0}")]
    ParseError(String),
}

/// Actor-RTC URI 结构
/// 格式: actr://<realm>:<manufacturer>+<name>@<version>
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActrUri {
    /// Realm ID (u32)
    pub realm: u32,
    /// Manufacturer name
    pub manufacturer: String,
    /// Actor type name
    pub name: String,
    /// Version (e.g., "v1")
    pub version: String,
}

impl ActrUri {
    /// 创建新的 Actor-RTC URI
    pub fn new(realm: u32, manufacturer: String, name: String, version: String) -> Self {
        Self {
            realm,
            manufacturer,
            name,
            version,
        }
    }

    /// 获取 scheme 信息
    pub fn scheme(&self) -> &'static str {
        "actr"
    }

    /// 获取 actor type 字符串表示 (manufacturer+name)
    pub fn actor_type(&self) -> String {
        format!("{}+{}", self.manufacturer, self.name)
    }
}

impl Display for ActrUri {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "actr://{}:{}+{}@{}",
            self.realm, self.manufacturer, self.name, self.version
        )
    }
}

impl FromStr for ActrUri {
    type Err = ActrUriError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.starts_with("actr://") {
            return Err(ActrUriError::InvalidScheme(
                s.split(':').next().unwrap_or("").to_string(),
            ));
        }

        let without_scheme = &s[7..];

        // Check for empty authority
        if without_scheme.is_empty() {
            return Err(ActrUriError::MissingAuthority);
        }

        // Check for version suffix '@'
        let (authority, version) = without_scheme
            .rsplit_once('@')
            .ok_or(ActrUriError::MissingVersion)?;

        let version = version.to_string();

        // Parse realm:manufacturer+name
        let (realm_str, type_part) = authority
            .split_once(':')
            .ok_or_else(|| ActrUriError::InvalidAuthorityFormat(authority.to_string()))?;

        let realm = realm_str
            .parse::<u32>()
            .map_err(|_| ActrUriError::InvalidRealmId(realm_str.to_string()))?;

        let (manufacturer, name) = type_part
            .split_once('+')
            .ok_or_else(|| ActrUriError::InvalidAuthorityFormat(authority.to_string()))?;

        Ok(ActrUri {
            realm,
            manufacturer: manufacturer.to_string(),
            name: name.to_string(),
            version,
        })
    }
}

/// Actor-RTC URI 构建器
#[derive(Debug, Default)]
pub struct ActrUriBuilder {
    realm: Option<u32>,
    manufacturer: Option<String>,
    name: Option<String>,
    version: Option<String>,
}

impl ActrUriBuilder {
    /// 创建新的构建器
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置 Realm ID
    pub fn realm(mut self, realm: u32) -> Self {
        self.realm = Some(realm);
        self
    }

    /// 设置 Manufacturer
    pub fn manufacturer<S: Into<String>>(mut self, manufacturer: S) -> Self {
        self.manufacturer = Some(manufacturer.into());
        self
    }

    /// 设置 Actor 类型名称
    pub fn name<S: Into<String>>(mut self, name: S) -> Self {
        self.name = Some(name.into());
        self
    }

    /// 设置版本
    pub fn version<S: Into<String>>(mut self, version: S) -> Self {
        self.version = Some(version.into());
        self
    }

    /// 构建 URI
    pub fn build(self) -> Result<ActrUri, ActrUriError> {
        let realm = self.realm.ok_or(ActrUriError::MissingAuthority)?;
        let manufacturer = self
            .manufacturer
            .ok_or(ActrUriError::InvalidAuthorityFormat(
                "missing manufacturer".to_string(),
            ))?;
        let name = self.name.ok_or(ActrUriError::InvalidAuthorityFormat(
            "missing name".to_string(),
        ))?;
        let version = self.version.unwrap_or_else(|| "v1".to_string());

        Ok(ActrUri {
            realm,
            manufacturer,
            name,
            version,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_uri_parsing() {
        let uri = "actr://101:acme+echo-service@v1"
            .parse::<ActrUri>()
            .unwrap();
        assert_eq!(uri.realm, 101);
        assert_eq!(uri.manufacturer, "acme");
        assert_eq!(uri.name, "echo-service");
        assert_eq!(uri.version, "v1");
    }

    #[test]
    fn test_uri_builder() {
        let uri = ActrUriBuilder::new()
            .realm(101)
            .manufacturer("acme")
            .name("order-service")
            .version("v1")
            .build()
            .unwrap();

        assert_eq!(uri.realm, 101);
        assert_eq!(uri.manufacturer, "acme");
        assert_eq!(uri.name, "order-service");
        assert_eq!(uri.version, "v1");
    }

    #[test]
    fn test_uri_to_string() {
        let uri = ActrUri::new(
            101,
            "acme".to_string(),
            "user-service".to_string(),
            "v1".to_string(),
        );
        let uri_string = uri.to_string();
        assert_eq!(uri_string, "actr://101:acme+user-service@v1");
    }

    #[test]
    fn test_invalid_scheme() {
        let result = "http://101:acme+service@v1".parse::<ActrUri>();
        assert!(matches!(result, Err(ActrUriError::InvalidScheme(_))));
    }

    #[test]
    fn test_missing_authority() {
        let result = "actr://".parse::<ActrUri>();
        assert!(matches!(result, Err(ActrUriError::MissingAuthority)));
    }

    #[test]
    fn test_missing_version() {
        let result = "actr://101:acme+service".parse::<ActrUri>();
        assert!(matches!(result, Err(ActrUriError::MissingVersion)));
    }

    #[test]
    fn test_invalid_realm_id() {
        let result = "actr://abc:acme+service@v1".parse::<ActrUri>();
        assert!(matches!(result, Err(ActrUriError::InvalidRealmId(_))));
    }

    #[test]
    fn test_invalid_authority_format() {
        let result = "actr://101:acme:service@v1".parse::<ActrUri>();
        assert!(matches!(
            result,
            Err(ActrUriError::InvalidAuthorityFormat(_))
        ));
    }

    #[test]
    fn test_actor_type_method() {
        let uri = "actr://101:acme+user-service@v1"
            .parse::<ActrUri>()
            .unwrap();
        assert_eq!(uri.actor_type(), "acme+user-service");
    }

    #[test]
    fn test_roundtrip() {
        let uri = ActrUriBuilder::new()
            .realm(9999)
            .manufacturer("test")
            .name("service")
            .build()
            .unwrap();

        let uri_str = uri.to_string();
        let parsed = uri_str.parse::<ActrUri>().unwrap();
        assert_eq!(uri.realm, parsed.realm);
        assert_eq!(uri.manufacturer, parsed.manufacturer);
        assert_eq!(uri.name, parsed.name);
        assert_eq!(uri.version, parsed.version);
    }
}
