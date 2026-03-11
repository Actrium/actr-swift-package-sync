//! UniFFI-exported types for cross-language bindings
//!
//! These types mirror the types from actr-protocol but with UniFFI derives.
//! They provide automatic conversion to/from the original types.
//! Additional types cover runtime lifecycle bindings.

use actr_runtime::lifecycle as runtime_lifecycle;

/// Security realm identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct Realm {
    pub realm_id: u32,
}

impl From<actr_protocol::Realm> for Realm {
    fn from(r: actr_protocol::Realm) -> Self {
        Self {
            realm_id: r.realm_id,
        }
    }
}

impl From<Realm> for actr_protocol::Realm {
    fn from(r: Realm) -> Self {
        Self {
            realm_id: r.realm_id,
        }
    }
}

/// Actor type (manufacturer + name + optional version)
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct ActrType {
    pub manufacturer: String,
    pub name: String,
    pub version: Option<String>,
}

impl From<actr_protocol::ActrType> for ActrType {
    fn from(t: actr_protocol::ActrType) -> Self {
        Self {
            manufacturer: t.manufacturer,
            name: t.name,
            version: if t.version.is_empty() {
                None
            } else {
                Some(t.version)
            },
        }
    }
}

impl From<ActrType> for actr_protocol::ActrType {
    fn from(t: ActrType) -> Self {
        Self {
            manufacturer: t.manufacturer,
            name: t.name,
            version: t.version.unwrap_or_default(),
        }
    }
}

/// Actor identifier (realm + serial_number + type)
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct ActrId {
    pub realm: Realm,
    pub serial_number: u64,
    pub r#type: ActrType,
}

impl From<actr_protocol::ActrId> for ActrId {
    fn from(id: actr_protocol::ActrId) -> Self {
        Self {
            realm: id.realm.into(),
            serial_number: id.serial_number,
            r#type: id.r#type.into(),
        }
    }
}

impl From<ActrId> for actr_protocol::ActrId {
    fn from(id: ActrId) -> Self {
        Self {
            realm: id.realm.into(),
            serial_number: id.serial_number,
            r#type: id.r#type.into(),
        }
    }
}

/// Metadata entry for DataStream
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct MetadataEntry {
    pub key: String,
    pub value: String,
}

impl From<actr_protocol::MetadataEntry> for MetadataEntry {
    fn from(m: actr_protocol::MetadataEntry) -> Self {
        Self {
            key: m.key,
            value: m.value,
        }
    }
}

impl From<MetadataEntry> for actr_protocol::MetadataEntry {
    fn from(m: MetadataEntry) -> Self {
        Self {
            key: m.key,
            value: m.value,
        }
    }
}

/// DataStream for fast-path data transmission
///
/// Used for streaming application data (non-media):
/// - File transfer chunks
/// - Game state updates
/// - Custom protocol streams
#[derive(Debug, Clone, uniffi::Record)]
pub struct DataStream {
    /// Stream identifier (globally unique)
    pub stream_id: String,
    /// Sequence number for ordering
    pub sequence: u64,
    /// Payload data
    pub payload: Vec<u8>,
    /// Optional metadata
    pub metadata: Vec<MetadataEntry>,
    /// Optional timestamp in milliseconds
    pub timestamp_ms: Option<i64>,
}

impl From<actr_protocol::DataStream> for DataStream {
    fn from(ds: actr_protocol::DataStream) -> Self {
        Self {
            stream_id: ds.stream_id,
            sequence: ds.sequence,
            payload: ds.payload.to_vec(),
            metadata: ds.metadata.into_iter().map(|m| m.into()).collect(),
            timestamp_ms: ds.timestamp_ms,
        }
    }
}

impl From<DataStream> for actr_protocol::DataStream {
    fn from(ds: DataStream) -> Self {
        Self {
            stream_id: ds.stream_id,
            sequence: ds.sequence,
            payload: ds.payload.into(),
            metadata: ds.metadata.into_iter().map(|m| m.into()).collect(),
            timestamp_ms: ds.timestamp_ms,
        }
    }
}

/// PayloadType enum for specifying transmission type
///
/// Determines which WebRTC channel/track to use for data transmission:
/// - `RpcReliable`: Reliable ordered channel (default for RPC)
/// - `RpcSignal`: Signaling channel for RPC
/// - `StreamReliable`: Reliable stream for DataStream
/// - `StreamLatencyFirst`: Low-latency stream (may drop packets)
/// - `MediaRtp`: Native RTP track for media
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, uniffi::Enum)]
pub enum PayloadType {
    #[default]
    RpcReliable,
    RpcSignal,
    StreamReliable,
    StreamLatencyFirst,
    MediaRtp,
}

impl From<PayloadType> for actr_protocol::PayloadType {
    fn from(pt: PayloadType) -> Self {
        match pt {
            PayloadType::RpcReliable => actr_protocol::PayloadType::RpcReliable,
            PayloadType::RpcSignal => actr_protocol::PayloadType::RpcSignal,
            PayloadType::StreamReliable => actr_protocol::PayloadType::StreamReliable,
            PayloadType::StreamLatencyFirst => actr_protocol::PayloadType::StreamLatencyFirst,
            PayloadType::MediaRtp => actr_protocol::PayloadType::MediaRtp,
        }
    }
}

impl From<actr_protocol::PayloadType> for PayloadType {
    fn from(pt: actr_protocol::PayloadType) -> Self {
        match pt {
            actr_protocol::PayloadType::RpcReliable => PayloadType::RpcReliable,
            actr_protocol::PayloadType::RpcSignal => PayloadType::RpcSignal,
            actr_protocol::PayloadType::StreamReliable => PayloadType::StreamReliable,
            actr_protocol::PayloadType::StreamLatencyFirst => PayloadType::StreamLatencyFirst,
            actr_protocol::PayloadType::MediaRtp => PayloadType::MediaRtp,
        }
    }
}

/// Network event types for runtime lifecycle callbacks
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum NetworkEvent {
    Available,
    Lost,
    TypeChanged { is_wifi: bool, is_cellular: bool },
    CleanupConnections,
}

impl From<runtime_lifecycle::NetworkEvent> for NetworkEvent {
    fn from(event: runtime_lifecycle::NetworkEvent) -> Self {
        match event {
            runtime_lifecycle::NetworkEvent::Available => NetworkEvent::Available,
            runtime_lifecycle::NetworkEvent::Lost => NetworkEvent::Lost,
            runtime_lifecycle::NetworkEvent::TypeChanged {
                is_wifi,
                is_cellular,
            } => NetworkEvent::TypeChanged {
                is_wifi,
                is_cellular,
            },
            runtime_lifecycle::NetworkEvent::CleanupConnections => NetworkEvent::CleanupConnections,
        }
    }
}

/// Network event processing result returned by the runtime
#[derive(Debug, Clone, uniffi::Record)]
pub struct NetworkEventResult {
    pub event: NetworkEvent,
    pub success: bool,
    pub error: Option<String>,
    pub duration_ms: u64,
}

impl From<runtime_lifecycle::NetworkEventResult> for NetworkEventResult {
    fn from(result: runtime_lifecycle::NetworkEventResult) -> Self {
        Self {
            event: result.event.into(),
            success: result.success,
            error: result.error,
            duration_ms: result.duration_ms,
        }
    }
}

/// Media type for MediaTrack
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MediaType {
    Audio,
    Video,
}

impl From<MediaType> for actr_framework::MediaType {
    fn from(mt: MediaType) -> Self {
        match mt {
            MediaType::Audio => actr_framework::MediaType::Audio,
            MediaType::Video => actr_framework::MediaType::Video,
        }
    }
}

impl From<actr_framework::MediaType> for MediaType {
    fn from(mt: actr_framework::MediaType) -> Self {
        match mt {
            actr_framework::MediaType::Audio => MediaType::Audio,
            actr_framework::MediaType::Video => MediaType::Video,
        }
    }
}

/// Media sample for WebRTC native track
#[derive(Debug, Clone, uniffi::Record)]
pub struct MediaSample {
    pub data: Vec<u8>,
    pub timestamp: u32,
    pub codec: String,
    pub media_type: MediaType,
}

impl From<MediaSample> for actr_framework::MediaSample {
    fn from(s: MediaSample) -> Self {
        Self {
            data: s.data.into(),
            timestamp: s.timestamp,
            codec: s.codec,
            media_type: s.media_type.into(),
        }
    }
}

impl From<actr_framework::MediaSample> for MediaSample {
    fn from(s: actr_framework::MediaSample) -> Self {
        Self {
            data: s.data.to_vec(),
            timestamp: s.timestamp,
            codec: s.codec,
            media_type: s.media_type.into(),
        }
    }
}
