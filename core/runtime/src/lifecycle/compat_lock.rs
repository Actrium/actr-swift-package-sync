//! compat.lock.toml - 运行时兼容性协商缓存
//!
//! 当服务发现无法找到精确匹配但找到兼容匹配时，会创建此文件。
//! 此文件的存在表示系统处于亚健康状态 (SUB-HEALTHY)。
//!
//! ## 功能
//! - 缓存协商结果，避免重复进行兼容性检查
//! - 记录系统健康状态，方便运维监控
//! - 提供快速启动路径，优先尝试已知兼容版本
//!
//! ## 存储位置
//! 此文件存储在操作系统的临时目录中，而非项目目录：
//! - Linux/macOS: `/tmp/actr/<project_hash>/compat.lock.toml`
//! - Windows: `%TEMP%\actr\<project_hash>\compat.lock.toml`
//!
//! `project_hash` 是根据项目根目录绝对路径计算的唯一哈希值，
//! 确保同一机器上多个 Actor 实例各有独立的缓存。
//!
//! ## 注意
//! 此文件不应提交到版本控制，因为它反映的是运行时状态。

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{debug, info, warn};

/// 文件名常量
const COMPAT_LOCK_FILENAME: &str = "compat.lock.toml";

/// 临时目录下的子目录名称
const ACTR_TEMP_DIR: &str = "actr";

/// 默认缓存过期时间（24小时）
const DEFAULT_TTL_HOURS: i64 = 24;

/// 根据项目根目录路径计算唯一哈希值
///
/// 返回一个短哈希字符串（16个字符），用于创建临时目录子路径
fn compute_project_hash(project_root: &Path) -> String {
    let canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let path_str = canonical.to_string_lossy();
    let mut hasher = Sha256::new();
    hasher.update(path_str.as_bytes());
    let result = hasher.finalize();
    // 取前8字节（16个十六进制字符）作为哈希
    hex::encode(&result[..8])
}

/// 获取 compat.lock.toml 的存储目录
///
/// 路径格式：`<temp_dir>/actr/<project_hash>/`
fn get_compat_lock_dir(project_root: &Path) -> PathBuf {
    let temp_dir = std::env::temp_dir();
    let project_hash = compute_project_hash(project_root);
    temp_dir.join(ACTR_TEMP_DIR).join(project_hash)
}

/// 兼容性协商记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegotiationEntry {
    /// 服务名称（例如 "user-service"）
    pub service_name: String,

    /// 请求的指纹（客户端期望的版本）
    pub requested_fingerprint: String,

    /// 实际解析的指纹（服务端提供的版本）
    pub resolved_fingerprint: String,

    /// 兼容性检查结果
    pub compatibility_check: CompatibilityCheck,

    /// 协商时间
    pub negotiated_at: DateTime<Utc>,

    /// 过期时间
    pub expires_at: DateTime<Utc>,
}

/// 兼容性检查结果
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityCheck {
    /// 完全兼容（精确匹配）
    ExactMatch,
    /// 向后兼容
    BackwardCompatible,
    /// 破坏性变更（不应该出现在 lock 文件中）
    BreakingChanges,
}

impl std::fmt::Display for CompatibilityCheck {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompatibilityCheck::ExactMatch => write!(f, "exact_match"),
            CompatibilityCheck::BackwardCompatible => write!(f, "backward_compatible"),
            CompatibilityCheck::BreakingChanges => write!(f, "breaking_changes"),
        }
    }
}

/// compat.lock.toml 文件结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompatLockFile {
    /// 文件头注释信息
    #[serde(skip_serializing_if = "Option::is_none")]
    pub _comment: Option<String>,

    /// 协商记录列表
    #[serde(default)]
    pub negotiation: Vec<NegotiationEntry>,
}

impl CompatLockFile {
    /// 创建新的空 lock 文件
    pub fn new() -> Self {
        Self {
            _comment: Some(
                "This file indicates the system is in SUB-HEALTHY state.\n\
                 Consider running 'actr install --force-update' to update dependencies."
                    .to_string(),
            ),
            negotiation: Vec::new(),
        }
    }

    /// 从文件加载
    pub async fn load(base_path: &Path) -> Result<Option<Self>, CompatLockError> {
        let file_path = base_path.join(COMPAT_LOCK_FILENAME);

        if !file_path.exists() {
            return Ok(None);
        }

        let content =
            fs::read_to_string(&file_path)
                .await
                .map_err(|e| CompatLockError::IoError {
                    path: file_path.clone(),
                    source: e,
                })?;

        let lock_file: Self =
            toml::from_str(&content).map_err(|e| CompatLockError::ParseError {
                path: file_path,
                source: e,
            })?;

        Ok(Some(lock_file))
    }

    /// 保存到文件
    pub async fn save(&self, base_path: &Path) -> Result<(), CompatLockError> {
        // 确保目录存在（临时目录可能不存在）
        if !base_path.exists() {
            fs::create_dir_all(base_path)
                .await
                .map_err(|e| CompatLockError::IoError {
                    path: base_path.to_path_buf(),
                    source: e,
                })?;
            debug!(
                "Created compat.lock cache directory: {}",
                base_path.display()
            );
        }

        let file_path = base_path.join(COMPAT_LOCK_FILENAME);

        let content = toml::to_string_pretty(self)
            .map_err(|e| CompatLockError::SerializeError { source: e })?;

        // 添加文件头注释
        let full_content = format!(
            "# compat.lock.toml - 兼容性协商缓存\n\
             # This file indicates the system is in SUB-HEALTHY state.\n\
             # Consider running 'actr install --force-update' to update dependencies.\n\
             # Location: {}\n\n\
             {content}",
            file_path.display()
        );

        fs::write(&file_path, full_content)
            .await
            .map_err(|e| CompatLockError::IoError {
                path: file_path,
                source: e,
            })?;

        Ok(())
    }

    /// 删除 lock 文件（系统恢复健康时调用）
    pub async fn remove(base_path: &Path) -> Result<bool, CompatLockError> {
        let file_path = base_path.join(COMPAT_LOCK_FILENAME);

        if file_path.exists() {
            fs::remove_file(&file_path)
                .await
                .map_err(|e| CompatLockError::IoError {
                    path: file_path,
                    source: e,
                })?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// 查找服务的协商记录
    pub fn find_entry(&self, service_name: &str) -> Option<&NegotiationEntry> {
        self.negotiation
            .iter()
            .find(|e| e.service_name == service_name)
    }

    /// 查找未过期的协商记录
    pub fn find_valid_entry(&self, service_name: &str) -> Option<&NegotiationEntry> {
        let now = Utc::now();
        self.negotiation
            .iter()
            .find(|e| e.service_name == service_name && e.expires_at > now)
    }

    /// 添加或更新协商记录
    pub fn upsert_entry(&mut self, entry: NegotiationEntry) {
        // 移除已存在的同名记录
        self.negotiation
            .retain(|e| e.service_name != entry.service_name);
        // 添加新记录
        self.negotiation.push(entry);
    }

    /// 清理过期的记录
    pub fn cleanup_expired(&mut self) -> usize {
        let now = Utc::now();
        let before = self.negotiation.len();
        self.negotiation.retain(|e| e.expires_at > now);
        before - self.negotiation.len()
    }

    /// 检查文件是否存在（即系统是否处于亚健康状态）
    pub async fn exists(base_path: &Path) -> bool {
        base_path.join(COMPAT_LOCK_FILENAME).exists()
    }

    /// 检查是否有任何有效的非精确匹配记录（亚健康状态）
    pub fn is_sub_healthy(&self) -> bool {
        let now = Utc::now();
        self.negotiation.iter().any(|e| {
            e.expires_at > now && e.compatibility_check == CompatibilityCheck::BackwardCompatible
        })
    }
}

impl NegotiationEntry {
    /// 创建新的协商记录
    pub fn new(
        service_name: String,
        requested_fingerprint: String,
        resolved_fingerprint: String,
        compatibility_check: CompatibilityCheck,
    ) -> Self {
        let now = Utc::now();
        Self {
            service_name,
            requested_fingerprint,
            resolved_fingerprint,
            compatibility_check,
            negotiated_at: now,
            expires_at: now + Duration::hours(DEFAULT_TTL_HOURS),
        }
    }

    /// 检查是否已过期
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }
}

/// compat.lock 相关错误
#[derive(Debug, thiserror::Error)]
pub enum CompatLockError {
    #[error("IO error at {path}: {source}")]
    IoError {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Parse error at {path}: {source}")]
    ParseError {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("Serialize error: {source}")]
    SerializeError {
        #[source]
        source: toml::ser::Error,
    },
}

/// 兼容性协商管理器 - 运行时使用
pub struct CompatLockManager {
    /// lock 文件所在目录（计算得出的临时目录路径）
    base_path: PathBuf,
    /// 项目根目录（用于日志记录）
    #[allow(dead_code)]
    project_root: PathBuf,
    /// 缓存的 lock 文件内容
    cached: Option<CompatLockFile>,
}

impl CompatLockManager {
    /// 创建新的管理器
    ///
    /// # Arguments
    /// * `project_root` - 项目根目录的路径，用于计算唯一的缓存目录
    ///
    /// # 存储位置
    /// 文件将存储在 `<temp_dir>/actr/<project_hash>/compat.lock.toml`
    pub fn new(project_root: PathBuf) -> Self {
        let base_path = get_compat_lock_dir(&project_root);
        debug!(
            "CompatLockManager initialized: project_root={}, cache_dir={}",
            project_root.display(),
            base_path.display()
        );
        Self {
            base_path,
            project_root,
            cached: None,
        }
    }

    /// 获取 compat.lock 文件的存储目录
    pub fn cache_dir(&self) -> &Path {
        &self.base_path
    }

    /// 加载 lock 文件
    pub async fn load(&mut self) -> Result<Option<&CompatLockFile>, CompatLockError> {
        self.cached = CompatLockFile::load(&self.base_path).await?;
        Ok(self.cached.as_ref())
    }

    /// 获取缓存的 lock 文件
    pub fn get_cached(&self) -> Option<&CompatLockFile> {
        self.cached.as_ref()
    }

    /// 记录协商结果
    ///
    /// 当发现服务时调用：
    /// - 如果是精确匹配，尝试删除对应的协商记录
    /// - 如果是兼容匹配，添加/更新协商记录
    pub async fn record_negotiation(
        &mut self,
        service_name: &str,
        requested_fingerprint: &str,
        resolved_fingerprint: &str,
        is_exact_match: bool,
        compatibility_check: CompatibilityCheck,
    ) -> Result<(), CompatLockError> {
        if is_exact_match {
            // 精确匹配：尝试删除旧的协商记录
            if let Some(ref mut lock_file) = self.cached {
                lock_file
                    .negotiation
                    .retain(|e| e.service_name != service_name);

                // 如果所有记录都被清除，删除文件
                if lock_file.negotiation.is_empty() {
                    CompatLockFile::remove(&self.base_path).await?;
                    self.cached = None;
                    info!("✅ SYSTEM HEALTHY: 所有依赖精确匹配，已删除 compat.lock.toml");
                } else {
                    lock_file.save(&self.base_path).await?;
                }
            }
        } else {
            // 兼容匹配：记录到 lock 文件
            let entry = NegotiationEntry::new(
                service_name.to_string(),
                requested_fingerprint.to_string(),
                resolved_fingerprint.to_string(),
                compatibility_check,
            );

            let lock_file = self.cached.get_or_insert_with(CompatLockFile::new);
            lock_file.upsert_entry(entry);
            lock_file.save(&self.base_path).await?;

            warn!(
                "🟡 SYSTEM SUB-HEALTHY: Service '{}' using compatible fingerprint ({}) instead of exact match ({}). \
                 Run 'actr install --force-update' to restore health.",
                service_name,
                &resolved_fingerprint[..20.min(resolved_fingerprint.len())],
                &requested_fingerprint[..20.min(requested_fingerprint.len())],
            );
        }

        Ok(())
    }

    /// 查找已缓存的兼容版本（用于快速启动）
    pub fn find_cached_compatible(
        &self,
        service_name: &str,
        requested_fingerprint: &str,
    ) -> Option<&NegotiationEntry> {
        self.cached.as_ref().and_then(|lock_file| {
            lock_file
                .find_valid_entry(service_name)
                .filter(|entry| entry.requested_fingerprint == requested_fingerprint)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_compat_lock_file_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path();

        // 创建并保存
        let mut lock_file = CompatLockFile::new();
        lock_file.upsert_entry(NegotiationEntry::new(
            "user-service".to_string(),
            "sha256:old".to_string(),
            "sha256:new".to_string(),
            CompatibilityCheck::BackwardCompatible,
        ));
        lock_file.save(base_path).await.unwrap();

        // 验证文件存在
        assert!(CompatLockFile::exists(base_path).await);

        // 重新加载
        let loaded = CompatLockFile::load(base_path).await.unwrap().unwrap();
        assert_eq!(loaded.negotiation.len(), 1);
        assert_eq!(loaded.negotiation[0].service_name, "user-service");
        assert!(loaded.is_sub_healthy());
    }

    #[tokio::test]
    async fn test_compat_lock_manager() {
        let temp_dir = TempDir::new().unwrap();
        // 使用临时目录作为项目根目录
        let project_root = temp_dir.path().to_path_buf();

        let mut manager = CompatLockManager::new(project_root.clone());

        // 验证缓存目录在系统临时目录下
        let cache_dir = manager.cache_dir().to_path_buf();
        assert!(cache_dir.starts_with(std::env::temp_dir()));
        assert!(cache_dir.to_string_lossy().contains("actr"));

        // 记录兼容匹配
        manager
            .record_negotiation(
                "user-service",
                "sha256:old",
                "sha256:new",
                false,
                CompatibilityCheck::BackwardCompatible,
            )
            .await
            .unwrap();

        // 验证文件存在于计算出的缓存目录
        assert!(CompatLockFile::exists(&cache_dir).await);

        // 验证文件不在项目目录中
        assert!(!project_root.join(COMPAT_LOCK_FILENAME).exists());

        // 查找缓存
        let entry = manager.find_cached_compatible("user-service", "sha256:old");
        assert!(entry.is_some());

        // 精确匹配后应该删除记录
        manager
            .record_negotiation(
                "user-service",
                "sha256:exact",
                "sha256:exact",
                true,
                CompatibilityCheck::ExactMatch,
            )
            .await
            .unwrap();

        // 文件应该被删除（因为没有其他记录）
        assert!(!CompatLockFile::exists(&cache_dir).await);
    }

    #[test]
    fn test_project_hash_deterministic() {
        let path1 = PathBuf::from("/tmp/test-project");
        let path2 = PathBuf::from("/tmp/test-project");
        let path3 = PathBuf::from("/tmp/other-project");

        let hash1 = compute_project_hash(&path1);
        let hash2 = compute_project_hash(&path2);
        let hash3 = compute_project_hash(&path3);

        // 相同路径应该产生相同哈希
        assert_eq!(hash1, hash2);
        // 不同路径应该产生不同哈希
        assert_ne!(hash1, hash3);
        // 哈希应该是16个十六进制字符
        assert_eq!(hash1.len(), 16);
    }
}
