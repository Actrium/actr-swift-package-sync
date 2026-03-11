//! GeoIP 地理位置查询工具
//!
//! 基于 MaxMind GeoLite2 数据库提供 IP 地址到地理坐标的转换
//!
//! # 使用示例
//!
//! ```rust,ignore
//! use actr_framework::util::geoip::GeoIpService;
//! use std::net::IpAddr;
//!
//! // 初始化 GeoIP 服务
//! let geoip = GeoIpService::new("data/geoip/GeoLite2-City.mmdb")?;
//!
//! // 查询 IP 地址的坐标
//! let ip: IpAddr = "8.8.8.8".parse()?;
//! if let Some((lat, lon)) = geoip.lookup(ip) {
//!     println!("IP {} 位于坐标: ({}, {})", ip, lat, lon);
//! }
//! ```
//!
//! # 获取 GeoLite2 数据库
//!
//! **自动下载（推荐）：**
//! 1. 访问 https://www.maxmind.com/en/geolite2/signup 获取 License Key
//! 2. 设置环境变量：`export MAXMIND_LICENSE_KEY="your-key-here"`
//! 3. 首次调用 `GeoIpService::new()` 时自动下载
//!
//! **手动下载（生产环境）：**
//! ```bash
//! curl -o GeoLite2-City.tar.gz \
//!   "https://download.maxmind.com/app/geoip_download?edition_id=GeoLite2-City&license_key=YOUR_KEY&suffix=tar.gz"
//! tar -xzf GeoLite2-City.tar.gz --strip-components=1 -C data/geoip/ "*/GeoLite2-City.mmdb"
//! ```

#[cfg(feature = "geoip")]
use anyhow::{Context, Result};
#[cfg(feature = "geoip")]
use maxminddb::{MaxMindDBError, Reader, geoip2::City};
#[cfg(feature = "geoip")]
use std::net::IpAddr;
#[cfg(feature = "geoip")]
use std::path::Path;
#[cfg(feature = "geoip")]
use tracing::{debug, info, warn};

/// GeoIP 查询服务
///
/// 提供 IP 地址到地理坐标的转换功能
#[cfg(feature = "geoip")]
#[derive(Debug)]
pub struct GeoIpService {
    reader: Reader<Vec<u8>>,
}

#[cfg(feature = "geoip")]
impl GeoIpService {
    /// 初始化 GeoIP 服务（支持自动下载）
    ///
    /// # Arguments
    /// * `db_path` - GeoLite2-City.mmdb 数据库文件路径
    ///
    /// # Errors
    /// 如果数据库文件不存在或格式错误，返回错误
    ///
    /// # 自动下载
    /// 如果数据库文件不存在，且设置了 `MAXMIND_LICENSE_KEY` 环境变量，
    /// 将自动从 MaxMind 下载 GeoLite2-City 数据库。
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        let path = db_path.as_ref();

        // 如果数据库不存在，尝试自动下载
        if !path.exists() {
            info!("GeoIP database not found at {:?}", path);

            if let Ok(license_key) = std::env::var("MAXMIND_LICENSE_KEY") {
                info!("MAXMIND_LICENSE_KEY found, attempting auto-download...");
                Self::download_database(path, &license_key)?;
            } else {
                anyhow::bail!(
                    "GeoIP database not found at {:?}\n\
                     \n\
                     To auto-download:\n\
                     1. Get License Key: https://www.maxmind.com/en/geolite2/signup\n\
                     2. export MAXMIND_LICENSE_KEY=\"your-key\"\n\
                     3. Retry\n\
                     \n\
                     Or manually download:\n\
                     curl -o GeoLite2-City.tar.gz \\\n\
                       'https://download.maxmind.com/app/geoip_download?edition_id=GeoLite2-City&license_key=YOUR_KEY&suffix=tar.gz'\n\
                     tar -xzf GeoLite2-City.tar.gz --strip-components=1 -C {:?}/ '*/GeoLite2-City.mmdb'",
                    path,
                    path.parent().unwrap_or(Path::new("."))
                );
            }
        }

        info!("Loading GeoIP database from: {:?}", path);
        let reader = Reader::open_readfile(path)
            .context(format!("Failed to open GeoIP database at {path:?}"))?;

        info!(
            "✅ GeoIP service initialized (build epoch: {})",
            reader.metadata.build_epoch
        );
        Ok(Self { reader })
    }

    /// 自动下载 GeoLite2-City 数据库
    fn download_database(db_path: &Path, license_key: &str) -> Result<()> {
        use reqwest::blocking::Client;

        info!("📥 Downloading GeoLite2-City database (~70MB)...");

        // 构建下载 URL
        let url = format!(
            "https://download.maxmind.com/app/geoip_download?edition_id=GeoLite2-City&license_key={}&suffix=tar.gz",
            license_key
        );

        // 下载
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(300)) // 5 分钟超时
            .build()?;

        let response = client
            .get(&url)
            .send()
            .context("Failed to download GeoLite2 database")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Download failed with status: {} - Check your MAXMIND_LICENSE_KEY",
                response.status()
            );
        }

        info!("📦 Download complete, extracting...");

        // 解压 tar.gz
        let tar_gz_data = response.bytes()?;
        let tar_decoder = flate2::read::GzDecoder::new(&tar_gz_data[..]);
        let mut archive = tar::Archive::new(tar_decoder);

        // 查找并提取 .mmdb 文件
        for entry in archive.entries()? {
            let mut entry = entry?;
            let path_in_archive = entry.path()?;

            if path_in_archive.extension() == Some(std::ffi::OsStr::new("mmdb"))
                && path_in_archive.to_string_lossy().contains("GeoLite2-City")
            {
                // 创建父目录
                if let Some(parent) = db_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }

                // 解压到目标位置
                let mut output = std::fs::File::create(db_path)?;
                std::io::copy(&mut entry, &mut output)?;

                let size = std::fs::metadata(db_path)?.len();
                info!(
                    "✅ GeoIP database downloaded to {:?} ({:.1} MB)",
                    db_path,
                    size as f64 / 1_048_576.0
                );
                return Ok(());
            }
        }

        anyhow::bail!("GeoLite2-City.mmdb not found in downloaded archive");
    }

    /// 查询 IP 地址的地理坐标
    ///
    /// # Arguments
    /// * `ip` - 要查询的 IP 地址
    ///
    /// # Returns
    /// * `Some((latitude, longitude))` - 成功找到坐标
    /// * `None` - IP 地址不在数据库中或无坐标信息
    pub fn lookup(&self, ip: IpAddr) -> Option<(f64, f64)> {
        match self.reader.lookup::<City>(ip) {
            Ok(city) => {
                if let Some(location) = city.location {
                    if let (Some(lat), Some(lon)) = (location.latitude, location.longitude) {
                        debug!("GeoIP lookup: {} -> ({}, {})", ip, lat, lon);
                        return Some((lat, lon));
                    }
                }
                debug!("GeoIP lookup: {} found but no coordinates", ip);
                None
            }
            Err(MaxMindDBError::AddressNotFoundError(_)) => {
                debug!("GeoIP lookup: {} not in database", ip);
                None
            }
            Err(e) => {
                warn!("GeoIP lookup error for {}: {}", ip, e);
                None
            }
        }
    }

    /// 获取数据库元信息
    pub fn metadata(&self) -> &maxminddb::Metadata {
        &self.reader.metadata
    }
}

/// 无 GeoIP 功能时的降级实现
#[cfg(not(feature = "geoip"))]
#[derive(Debug)]
pub struct GeoIpService;

#[cfg(not(feature = "geoip"))]
impl GeoIpService {
    /// 初始化失败（需要启用 geoip feature）
    pub fn new<P>(_db_path: P) -> anyhow::Result<Self> {
        anyhow::bail!("GeoIP feature is not enabled. Rebuild with --features geoip")
    }

    /// 总是返回 None（需要启用 geoip feature）
    pub fn lookup(&self, _ip: std::net::IpAddr) -> Option<(f64, f64)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_geoip_module_compiles() {
        // 确保模块编译通过
        assert!(true);
    }

    #[cfg(feature = "geoip")]
    #[test]
    fn test_geoip_lookup_requires_database() {
        // 测试需要真实的数据库文件，这里只验证 API 可用性
        let result = GeoIpService::new("/nonexistent/path.mmdb");
        assert!(result.is_err());
    }

    #[cfg(not(feature = "geoip"))]
    #[test]
    fn test_geoip_feature_disabled() {
        let result = GeoIpService::new("/any/path.mmdb");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("GeoIP feature is not enabled")
        );
    }
}
