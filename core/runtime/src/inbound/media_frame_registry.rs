//! MediaFrameRegistry - Fast path media frame registry (WebRTC native)

use actr_protocol::{ActorResult, ActrId};
use dashmap::DashMap;
use futures_util::future::BoxFuture;
use std::sync::Arc;

// Use MediaSample from framework (dependency inversion)
use actr_framework::MediaSample;

/// MediaTrack callback type
///
/// # Design Rationale
/// - Uses WebRTC native types (no protobuf overhead)
/// - Receives MediaSample directly from RTCTrackRemote
/// - Sender ActrId provided for source identification
/// - Fast path: direct callback, no queue
pub type MediaTrackCallback =
    Arc<dyn Fn(MediaSample, ActrId) -> BoxFuture<'static, ActorResult<()>> + Send + Sync>;

/// MediaFrameRegistry - WebRTC native media track callback manager
///
/// # Architecture
///
/// MediaFrameRegistry works with WebRTC PeerConnection to receive native media:
///
/// ```text
/// WebRTC PeerConnection
///   └─ RTCTrackRemote (native RTP channel)
///       └─ on_track callback
///           └─ MediaFrameRegistry::dispatch()
///               └─ User callback (MediaSample, sender_id)
/// ```
///
/// # Key Points
/// - **No protobuf**: Uses WebRTC native sample data
/// - **No DataChannel**: Media goes through RTCTrackRemote (RTP)
/// - **Zero serialization**: Direct sample bytes from RTP packets
/// - **Low latency**: ~1-2ms from network to callback
///
/// # Typical Use Cases
/// - Real-time audio/video calls
/// - Screen sharing
/// - Audio/video recording
/// - Media transcoding
pub struct MediaFrameRegistry {
    /// Concurrent mapping of track_id → callback function
    callbacks: DashMap<String, MediaTrackCallback>,
}

impl Default for MediaFrameRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaFrameRegistry {
    pub fn new() -> Self {
        Self {
            callbacks: DashMap::new(),
        }
    }

    /// Register media track callback
    ///
    /// # Arguments
    /// - `track_id`: Track identifier (must be globally unique)
    /// - `callback`: Media sample handler callback
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// registry.register("video-track-1", Arc::new(|sample, sender| {
    ///     Box::pin(async move {
    ///         println!("Received {} bytes from {:?}", sample.data.len(), sender);
    ///         // Decode and render video frame...
    ///         Ok(())
    ///     })
    /// }));
    /// ```
    pub fn register(&self, track_id: String, callback: MediaTrackCallback) {
        self.callbacks.insert(track_id.clone(), callback);
        tracing::info!("🎬 Registered MediaTrack: {}", track_id);
    }

    /// Unregister media track callback
    ///
    /// # Arguments
    /// - `track_id`: Track identifier to unregister
    pub fn unregister(&self, track_id: &str) {
        self.callbacks.remove(track_id);
        tracing::info!("🚫 Unregistered MediaTrack: {}", track_id);
    }

    /// Dispatch media sample to callback (concurrent execution)
    ///
    /// Called by WebRTC on_track handler when a media sample arrives.
    ///
    /// # Arguments
    /// - `track_id`: Track identifier
    /// - `sample`: Media sample from RTCTrackRemote
    /// - `sender_id`: Sender ActrId
    ///
    /// # Performance
    /// - Direct callback invocation, no queueing overhead
    /// - Latency: ~1-2μs dispatch time (excluding callback execution)
    /// - Concurrent execution, doesn't block other tracks
    pub async fn dispatch(&self, track_id: &str, sample: MediaSample, sender_id: ActrId) {
        let start = std::time::Instant::now();

        if let Some(callback) = self.callbacks.get(track_id) {
            let callback = callback.clone();
            tokio::spawn(async move {
                if let Err(e) = callback(sample, sender_id).await {
                    tracing::error!("❌ MediaTrack callback error: {:?}", e);
                }
            });

            tracing::debug!("🎬 Dispatched media sample in {:?}", start.elapsed());
        } else {
            tracing::warn!("⚠️ No callback registered for track: {}", track_id);
        }
    }

    /// Get active track count
    pub fn active_tracks(&self) -> usize {
        self.callbacks.len()
    }
}
