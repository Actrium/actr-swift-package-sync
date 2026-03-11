//! Platform-specific observability initialization for libactr.
//!
//! This module adapts the core observability features from `actr-runtime`
//! to mobile platforms (Android Logcat, iOS os_log).

use actr_config::ObservabilityConfig;
use actr_runtime::observability::{ObservabilityGuard, init_observability_with_layer};
use std::sync::OnceLock;

static GUARD: OnceLock<ObservabilityGuard> = OnceLock::new();

/// Initialize the logger and tracing system with provided configuration.
///
/// This function is idempotent and can be safely called multiple times.
/// It will automatically detect the platform and use the appropriate logging backend:
/// - Android: Logcat (via `tracing-android`)
/// - macOS: Apple Unified Logging (via `tracing-oslog`)
/// - iOS: Stdout (via `fmt::layer`, without ANSI colors)
/// - Linux/Unix: Stdout (via `fmt::layer`, with ANSI colors for terminal)
/// - Others: Stdout (via `fmt::layer`, without ANSI colors)
pub fn init_observability(config: ObservabilityConfig) {
    if GUARD.get().is_some() {
        return;
    }

    let guard: Option<ObservabilityGuard> = {
        #[cfg(target_os = "android")]
        {
            let layer =
                tracing_android::layer("actr").expect("Failed to create Android tracing layer");
            init_observability_with_layer(&config, Some(layer)).ok()
        }

        #[cfg(target_os = "macos")]
        {
            let result = {
                #[cfg(feature = "macos-oslog")]
                {
                    let layer = tracing_oslog::OsLogger::new("com.actor-rtc.actr", "core");
                    init_observability_with_layer(&config, Some(layer)).ok()
                }
                #[cfg(not(feature = "macos-oslog"))]
                {
                    None
                }
            };

            // Fallback to fmt layer if OsLogger failed or feature is disabled
            result.or_else(|| {
                init_observability_with_layer(&config, None::<tracing_subscriber::layer::Identity>)
                    .ok()
            })
        }

        #[cfg(not(any(target_os = "android", target_os = "macos")))]
        {
            // On iOS (simulator/device) and other platforms, use fmt layer without ANSI
            // The fmt layer is already configured with .with_ansi(false) in actr-runtime
            init_observability_with_layer(&config, None::<tracing_subscriber::layer::Identity>).ok()
        }
    };

    if let Some(g) = guard {
        let _ = GUARD.set(g);
    } else {
        // If initialization failed, log a warning but don't panic
        // The system will continue to work, just without structured logging
    }
}
