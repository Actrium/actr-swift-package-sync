//! Configuration parser - converts RawConfig to Config

use crate::error::{ConfigError, Result};
use crate::{Config, RawConfig};
use std::path::Path;

mod v1;

/// 配置解析器工厂
pub struct ConfigParser;

impl ConfigParser {
    /// 根据 edition 选择合适的解析器并解析配置
    pub fn parse(raw: RawConfig, config_path: impl AsRef<Path>) -> Result<Config> {
        match raw.edition {
            1 => v1::ParserV1::new(config_path).parse(raw),
            // 未来可以添加更多版本
            // 2 => v2::ParserV2::new(config_path).parse(raw),
            edition => Err(ConfigError::UnsupportedEdition(edition)),
        }
    }

    /// 从文件加载并解析配置（便捷方法）
    pub fn from_file(path: impl AsRef<Path>) -> Result<Config> {
        let raw = RawConfig::from_file(path.as_ref())?;
        Self::parse(raw, path)
    }
}
