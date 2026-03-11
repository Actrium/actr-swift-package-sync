//! Resource management

use actr_protocol::{ActorResult, ActrError};
use serde::{Deserialize, Serialize};

/// Resource quota
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceQuota {
    /// CPU quota（CPU cores）
    pub cpu_cores: f64,

    /// memory quota（bytes）
    pub memory_bytes: u64,

    /// network bandwidth quota（bytes/sec）
    pub network_bandwidth_bps: u64,

    /// disk IO quota（bytes/sec）
    pub disk_io_bps: u64,
}

impl Default for ResourceQuota {
    fn default() -> Self {
        Self {
            cpu_cores: 1.0,
            memory_bytes: 1024 * 1024 * 1024,         // 1GB
            network_bandwidth_bps: 100 * 1024 * 1024, // 100MB/s
            disk_io_bps: 100 * 1024 * 1024,           // 100MB/s
        }
    }
}

/// Resource usage
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceUsage {
    /// CPU usage rate (0.0-1.0)
    pub cpu_usage: f64,

    /// memoryusingusage（bytes）
    pub memory_used_bytes: u64,

    /// network carry widthusingusage（bytes/sec）
    pub network_usage_bps: u64,

    /// disk IO usingusage（bytes/sec）
    pub disk_io_bps: u64,
}

/// Resource configuration
#[derive(Debug, Clone)]
pub struct ResourceConfig {
    /// whetherenable resourcelimit
    pub enable_limits: bool,

    /// resourcemonitoringinterval（seconds）
    pub monitoring_interval_seconds: u64,

    /// resourceusingWarningthreshold (0.0-1.0)
    pub warning_threshold: f64,

    /// resourceusinglimitthreshold (0.0-1.0)
    pub limit_threshold: f64,
}

impl Default for ResourceConfig {
    fn default() -> Self {
        Self {
            enable_limits: true,
            monitoring_interval_seconds: 5,
            warning_threshold: 0.8,
            limit_threshold: 0.95,
        }
    }
}

/// Resource manager
pub struct ResourceManager {
    config: ResourceConfig,
    quota: ResourceQuota,
    current_usage: ResourceUsage,
}

impl ResourceManager {
    /// Create newResource manager
    pub fn new(config: ResourceConfig, quota: ResourceQuota) -> Self {
        Self {
            config,
            quota,
            current_usage: ResourceUsage::default(),
        }
    }

    /// Checkresourcewhetheravailable
    pub fn check_resource_availability(&self, required: &ResourceUsage) -> ActorResult<bool> {
        if !self.config.enable_limits {
            return Ok(true);
        }

        // Check CPU
        let cpu_available =
            self.quota.cpu_cores - (self.current_usage.cpu_usage * self.quota.cpu_cores);
        if required.cpu_usage * self.quota.cpu_cores > cpu_available {
            return Ok(false);
        }

        // Checkmemory
        let memory_available = self.quota.memory_bytes - self.current_usage.memory_used_bytes;
        if required.memory_used_bytes > memory_available {
            return Ok(false);
        }

        Ok(true)
    }

    /// Allocateresource
    pub fn allocate_resources(&mut self, usage: &ResourceUsage) -> ActorResult<()> {
        if !self.check_resource_availability(usage)? {
            return Err(ActrError::Unavailable(
                "Insufficient resources available".to_string(),
            ));
        }

        self.current_usage.cpu_usage += usage.cpu_usage;
        self.current_usage.memory_used_bytes += usage.memory_used_bytes;
        self.current_usage.network_usage_bps += usage.network_usage_bps;
        self.current_usage.disk_io_bps += usage.disk_io_bps;

        Ok(())
    }

    /// Releaseresource
    pub fn release_resources(&mut self, usage: &ResourceUsage) -> ActorResult<()> {
        self.current_usage.cpu_usage = (self.current_usage.cpu_usage - usage.cpu_usage).max(0.0);
        self.current_usage.memory_used_bytes = self
            .current_usage
            .memory_used_bytes
            .saturating_sub(usage.memory_used_bytes);
        self.current_usage.network_usage_bps = self
            .current_usage
            .network_usage_bps
            .saturating_sub(usage.network_usage_bps);
        self.current_usage.disk_io_bps = self
            .current_usage
            .disk_io_bps
            .saturating_sub(usage.disk_io_bps);

        Ok(())
    }

    /// GetcurrentResource usage
    pub fn get_usage(&self) -> &ResourceUsage {
        &self.current_usage
    }

    /// GetResource quota
    pub fn get_quota(&self) -> &ResourceQuota {
        &self.quota
    }

    /// compute resourcesusage rate
    pub fn calculate_usage_ratio(&self) -> ResourceUsageRatio {
        ResourceUsageRatio {
            cpu_ratio: self.current_usage.cpu_usage,
            memory_ratio: self.current_usage.memory_used_bytes as f64
                / self.quota.memory_bytes as f64,
            network_ratio: self.current_usage.network_usage_bps as f64
                / self.quota.network_bandwidth_bps as f64,
            disk_ratio: self.current_usage.disk_io_bps as f64 / self.quota.disk_io_bps as f64,
        }
    }
}

/// resourceusage rate
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUsageRatio {
    /// CPU usage rate (0.0-1.0)
    pub cpu_ratio: f64,

    /// memoryusage rate (0.0-1.0)
    pub memory_ratio: f64,

    /// networkusage rate (0.0-1.0)
    pub network_ratio: f64,

    /// diskusage rate (0.0-1.0)
    pub disk_ratio: f64,
}
