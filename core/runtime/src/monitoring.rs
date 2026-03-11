//! monitoringandalert

use actr_protocol::{ActorResult, ActrError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Alert severity
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AlertSeverity {
    /// info
    Info = 1,
    /// Warning
    Warning = 2,
    /// Error
    Error = 3,
    /// critical
    Critical = 4,
}

impl AlertSeverity {
    /// Getseverity description
    pub fn description(&self) -> &'static str {
        match self {
            AlertSeverity::Info => "info",
            AlertSeverity::Warning => "Warning",
            AlertSeverity::Error => "Error",
            AlertSeverity::Critical => "critical",
        }
    }
}

/// Alert information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    /// alert ID
    pub id: Uuid,

    /// alert title
    pub title: String,

    /// alert description
    pub description: String,

    /// severity
    pub severity: AlertSeverity,

    /// alert source
    pub source: String,

    /// occurrence time
    pub timestamp: DateTime<Utc>,

    /// whetheracknowledged
    pub acknowledged: bool,

    /// whetherresolved
    pub resolved: bool,

    /// tags
    pub labels: HashMap<String, String>,

    /// metric value
    pub metric_value: Option<f64>,

    /// threshold
    pub threshold: Option<f64>,
}

impl Alert {
    /// Createnew alert
    pub fn new(
        title: String,
        description: String,
        severity: AlertSeverity,
        source: String,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            title,
            description,
            severity,
            source,
            timestamp: Utc::now(),
            acknowledged: false,
            resolved: false,
            labels: HashMap::new(),
            metric_value: None,
            threshold: None,
        }
    }

    /// add tags
    pub fn with_label(mut self, key: String, value: String) -> Self {
        self.labels.insert(key, value);
        self
    }

    /// Setmetric valueandthreshold
    pub fn with_metric(mut self, value: f64, threshold: f64) -> Self {
        self.metric_value = Some(value);
        self.threshold = Some(threshold);
        self
    }

    /// acknowledge alert
    pub fn acknowledge(&mut self) {
        self.acknowledged = true;
    }

    /// resolve alert
    pub fn resolve(&mut self) {
        self.resolved = true;
    }
}

/// alertconfiguration
#[derive(Debug, Clone)]
pub struct AlertConfig {
    /// whetherenable alerts
    pub enabled: bool,

    /// CPU usage ratealertthreshold
    pub cpu_warning_threshold: f64,
    pub cpu_critical_threshold: f64,

    /// memoryusage ratealertthreshold
    pub memory_warning_threshold: f64,
    pub memory_critical_threshold: f64,

    /// Errorrate alertthreshold
    pub error_rate_warning_threshold: f64,
    pub error_rate_critical_threshold: f64,

    /// response respond temporal duration alertthreshold（milliseconds）
    pub response_time_warning_threshold_ms: f64,
    pub response_time_critical_threshold_ms: f64,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cpu_warning_threshold: 0.8,
            cpu_critical_threshold: 0.95,
            memory_warning_threshold: 0.8,
            memory_critical_threshold: 0.95,
            error_rate_warning_threshold: 0.05,
            error_rate_critical_threshold: 0.1,
            response_time_warning_threshold_ms: 1000.0,
            response_time_critical_threshold_ms: 5000.0,
        }
    }
}

/// monitoringconfiguration
#[derive(Debug, Clone)]
pub struct MonitoringConfig {
    /// whetherenable monitoring
    pub enabled: bool,

    /// monitoringinterval（seconds）
    pub monitoring_interval_seconds: u64,

    /// metrics keep retain temporal duration （seconds）
    pub metrics_retention_seconds: u64,

    /// alertconfiguration
    pub alert_config: AlertConfig,
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            monitoring_interval_seconds: 30,
            metrics_retention_seconds: 7 * 24 * 3600, // 7 days
            alert_config: AlertConfig::default(),
        }
    }
}

/// Monitoring metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metric {
    /// metric name
    pub name: String,

    /// metric value
    pub value: f64,

    /// timestamp
    pub timestamp: DateTime<Utc>,

    /// tags
    pub labels: HashMap<String, String>,

    /// unit
    pub unit: Option<String>,
}

/// Monitor interface
pub trait Monitor: Send + Sync {
    /// record metrics
    fn record_metric(&mut self, metric: Metric) -> ActorResult<()>;

    /// Getmetrics
    fn get_metrics(&self, name: &str, duration_seconds: u64) -> ActorResult<Vec<Metric>>;

    /// Checkalert conditions
    fn check_alerts(&mut self) -> ActorResult<Vec<Alert>>;

    /// Getactive alerts
    fn get_active_alerts(&self) -> Vec<&Alert>;

    /// acknowledge alert
    fn acknowledge_alert(&mut self, alert_id: Uuid) -> ActorResult<()>;

    /// resolve alert
    fn resolve_alert(&mut self, alert_id: Uuid) -> ActorResult<()>;
}

/// Basic monitor implementation
pub struct BasicMonitor {
    config: MonitoringConfig,
    metrics: Vec<Metric>,
    alerts: Vec<Alert>,
}

impl BasicMonitor {
    /// Create newmonitor
    pub fn new(config: MonitoringConfig) -> Self {
        Self {
            config,
            metrics: Vec::new(),
            alerts: Vec::new(),
        }
    }

    /// Check CPU usage ratealert
    fn check_cpu_alerts(&mut self, cpu_usage: f64) -> ActorResult<Option<Alert>> {
        if !self.config.alert_config.enabled {
            return Ok(None);
        }

        if cpu_usage >= self.config.alert_config.cpu_critical_threshold {
            let alert = Alert::new(
                "CPU usage ratecritical".to_string(),
                format!("CPU usage ratereachedto {:.1}%", cpu_usage * 100.0),
                AlertSeverity::Critical,
                "system".to_string(),
            )
            .with_metric(cpu_usage, self.config.alert_config.cpu_critical_threshold);

            Ok(Some(alert))
        } else if cpu_usage >= self.config.alert_config.cpu_warning_threshold {
            let alert = Alert::new(
                "CPU usage rateWarning".to_string(),
                format!("CPU usage ratereachedto {:.1}%", cpu_usage * 100.0),
                AlertSeverity::Warning,
                "system".to_string(),
            )
            .with_metric(cpu_usage, self.config.alert_config.cpu_warning_threshold);

            Ok(Some(alert))
        } else {
            Ok(None)
        }
    }
}

impl Monitor for BasicMonitor {
    fn record_metric(&mut self, metric: Metric) -> ActorResult<()> {
        if !self.config.enabled {
            return Ok(());
        }

        self.metrics.push(metric);

        // Clean up expired metrics
        let cutoff =
            Utc::now() - chrono::Duration::seconds(self.config.metrics_retention_seconds as i64);
        self.metrics.retain(|m| m.timestamp > cutoff);

        Ok(())
    }

    fn get_metrics(&self, name: &str, duration_seconds: u64) -> ActorResult<Vec<Metric>> {
        let cutoff = Utc::now() - chrono::Duration::seconds(duration_seconds as i64);

        let metrics: Vec<Metric> = self
            .metrics
            .iter()
            .filter(|m| m.name == name && m.timestamp > cutoff)
            .cloned()
            .collect();

        Ok(metrics)
    }

    fn check_alerts(&mut self) -> ActorResult<Vec<Alert>> {
        if !self.config.alert_config.enabled {
            return Ok(Vec::new());
        }

        let mut new_alerts = Vec::new();

        // Check CPU usage rate
        if let Ok(cpu_metrics) = self.get_metrics("cpu_usage", 300) {
            if let Some(latest) = cpu_metrics.last() {
                if let Some(alert) = self.check_cpu_alerts(latest.value)? {
                    new_alerts.push(alert);
                }
            }
        }

        // Add new alerttolist
        for alert in &new_alerts {
            self.alerts.push(alert.clone());
        }

        Ok(new_alerts)
    }

    fn get_active_alerts(&self) -> Vec<&Alert> {
        self.alerts.iter().filter(|alert| !alert.resolved).collect()
    }

    fn acknowledge_alert(&mut self, alert_id: Uuid) -> ActorResult<()> {
        if let Some(alert) = self.alerts.iter_mut().find(|a| a.id == alert_id) {
            alert.acknowledge();
            Ok(())
        } else {
            Err(ActrError::NotFound("Alert not found".to_string()))
        }
    }

    fn resolve_alert(&mut self, alert_id: Uuid) -> ActorResult<()> {
        if let Some(alert) = self.alerts.iter_mut().find(|a| a.id == alert_id) {
            alert.resolve();
            Ok(())
        } else {
            Err(ActrError::NotFound("Alert not found".to_string()))
        }
    }
}
