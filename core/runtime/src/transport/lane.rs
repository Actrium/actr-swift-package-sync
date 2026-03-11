//! DataLane - Business data transport channel
//!
//! DataLane is the core abstraction of the transport layer for message/data transmission.
//! Note: MediaTrack uses a separate MediaFrameRegistry path, not DataLane.
//!
//! ## Design Philosophy
//!
//! ```text
//! DataLane features:
//!   ✓ enum type with 3 variants (WebRtcDataChannel, Mpsc, WebSocket)
//!   ✓ Unified send/recv API for data messages
//!   ✓ Cloneable (uses Arc internally for sharing)
//!   ✓ Multi-consumer pattern (shared receive channel)
//! ```
//!
//! ## WebRTC DataChannel Fragmentation
//!
//! WebRTC DataChannel has a 64KB per-message limit. The `WebRtcDataChannel` variant
//! transparently fragments outgoing messages and reassembles incoming fragments.
//! Upper layers are unaware of this mechanism.
//!
//! ### Fragment header format (8 bytes, prepended to each DataChannel message)
//!
//! ```text
//! [4 bytes: msg_id       u32 big-endian]
//! [2 bytes: frag_index   u16 big-endian]
//! [2 bytes: total_frags  u16 big-endian]
//! ```
//!
//! When `total_frags == 1` the message fits in a single DataChannel message;
//! the receiver strips the header and returns the payload directly.

use super::error::{NetworkError, NetworkResult};
use actr_protocol::PayloadType;
use futures_util::SinkExt;
use futures_util::stream::SplitSink;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use webrtc::data_channel::RTCDataChannel;

// ── WebRTC DataChannel fragmentation constants ────────────────────────────────

/// Maximum size of a single WebRTC DataChannel message.
const DC_MAX_MESSAGE_SIZE: usize = 64 * 1024;

/// Size of the fragment header prepended to every DataChannel message.
const FRAGMENT_HEADER_SIZE: usize = 8;

/// Maximum payload bytes that fit in one DataChannel message after the header.
const DC_MAX_PAYLOAD_SIZE: usize = DC_MAX_MESSAGE_SIZE - FRAGMENT_HEADER_SIZE;

// ── Reassembly types ──────────────────────────────────────────────────────────

/// Holds the pieces of a multi-fragment message while waiting for all fragments.
struct FragmentEntry {
    total: u16,
    fragments: HashMap<u16, bytes::Bytes>,
}

/// Accumulates in-flight fragmented messages keyed by `msg_id`.
pub struct ReassemblyBuffer {
    pending: HashMap<u32, FragmentEntry>,
}

impl ReassemblyBuffer {
    fn new() -> Self {
        Self {
            pending: HashMap::new(),
        }
    }

    /// Insert a fragment.
    ///
    /// Returns the fully reassembled payload when all fragments for `msg_id`
    /// have arrived, or `None` if more fragments are still missing.
    fn insert(
        &mut self,
        msg_id: u32,
        frag_index: u16,
        total_frags: u16,
        payload: bytes::Bytes,
    ) -> Option<bytes::Bytes> {
        let entry = self.pending.entry(msg_id).or_insert_with(|| FragmentEntry {
            total: total_frags,
            fragments: HashMap::new(),
        });
        entry.fragments.insert(frag_index, payload);

        if entry.fragments.len() == entry.total as usize {
            // All fragments arrived – reassemble in order.
            let entry = self.pending.remove(&msg_id).unwrap();
            let mut ordered: Vec<(u16, bytes::Bytes)> = entry.fragments.into_iter().collect();
            ordered.sort_by_key(|(idx, _)| *idx);

            let total_len: usize = ordered.iter().map(|(_, b)| b.len()).sum();
            let mut out = bytes::BytesMut::with_capacity(total_len);
            for (_, frag) in ordered {
                out.extend_from_slice(&frag);
            }
            Some(out.freeze())
        } else {
            None
        }
    }
}

// ── Fragment header encode / decode ───────────────────────────────────────────

/// Encode a fragment header into `buf` (must have at least 8 bytes of capacity).
#[inline]
fn encode_fragment_header(buf: &mut Vec<u8>, msg_id: u32, frag_index: u16, total_frags: u16) {
    buf.extend_from_slice(&msg_id.to_be_bytes());
    buf.extend_from_slice(&frag_index.to_be_bytes());
    buf.extend_from_slice(&total_frags.to_be_bytes());
}

/// Decode a fragment header from a raw DataChannel message.
///
/// Returns `(msg_id, frag_index, total_frags, payload)` on success.
///
/// # Errors
/// Returns [`NetworkError::DataChannelError`] when the message is shorter than
/// [`FRAGMENT_HEADER_SIZE`].
#[inline]
fn decode_fragment_header(raw: bytes::Bytes) -> NetworkResult<(u32, u16, u16, bytes::Bytes)> {
    if raw.len() < FRAGMENT_HEADER_SIZE {
        return Err(NetworkError::DataChannelError(format!(
            "fragment too short: {} bytes (minimum {})",
            raw.len(),
            FRAGMENT_HEADER_SIZE
        )));
    }
    let msg_id = u32::from_be_bytes(raw[0..4].try_into().unwrap());
    let frag_index = u16::from_be_bytes(raw[4..6].try_into().unwrap());
    let total_frags = u16::from_be_bytes(raw[6..8].try_into().unwrap());
    let payload = raw.slice(FRAGMENT_HEADER_SIZE..);
    Ok((msg_id, frag_index, total_frags, payload))
}

// ── Type alias ────────────────────────────────────────────────────────────────

/// Type alias for WebSocket sink (shared across all PayloadTypes)
type WsSink = Arc<Mutex<Option<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, WsMessage>>>>;

/// DataLane - Data transport channel
///
/// Each DataLane represents a specific transport path for data/message transmission.
/// MediaTrack uses a separate path via MediaFrameRegistry, not DataLane.
#[derive(Clone)]
pub enum DataLane {
    /// WebRTC DataChannel Lane
    ///
    /// For transmitting messages via WebRTC DataChannel.
    /// Transparently fragments outgoing messages > [`DC_MAX_PAYLOAD_SIZE`] and
    /// reassembles incoming fragments before handing data to callers.
    WebRtcDataChannel {
        /// Underlying DataChannel
        data_channel: Arc<RTCDataChannel>,

        /// Receive channel (shared, uses Bytes for zero-copy)
        rx: Arc<Mutex<mpsc::Receiver<bytes::Bytes>>>,

        /// Monotonically increasing message-id counter for fragment correlation.
        msg_id_counter: Arc<AtomicU32>,

        /// Per-lane reassembly state for multi-fragment messages.
        reassembly: Arc<Mutex<ReassemblyBuffer>>,
    },

    /// Mpsc Lane
    ///
    /// For intra-process communication (Inproc transport)
    ///
    /// Note: directly passes RpcEnvelope objects, zero serialization
    Mpsc {
        /// PayloadType identifier
        payload_type: PayloadType,

        /// Send channel (directly passes RpcEnvelope)
        tx: mpsc::Sender<actr_protocol::RpcEnvelope>,

        /// Receive channel (shared)
        rx: Arc<Mutex<mpsc::Receiver<actr_protocol::RpcEnvelope>>>,
    },

    /// WebSocket Lane
    ///
    /// For business data transmission in C/S architecture
    WebSocket {
        /// Shared Sink (all PayloadTypes share the same WebSocket connection)
        /// Uses Option to support lazy initialization
        sink: WsSink,

        /// PayloadType identifier (used to add message header when sending)
        payload_type: PayloadType,

        /// Receive channel (independent, routed by dispatcher, uses Bytes for zero-copy)
        rx: Arc<Mutex<mpsc::Receiver<bytes::Bytes>>>,
    },
}

impl DataLane {
    /// Send message
    ///
    /// # Arguments
    /// - `data`: message data (uses Bytes for zero-copy)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use bytes::Bytes;
    /// data_lane.send(Bytes::from_static(b"hello")).await?;
    /// ```
    pub async fn send(&self, data: bytes::Bytes) -> NetworkResult<()> {
        match self {
            DataLane::WebRtcDataChannel {
                data_channel,
                msg_id_counter,
                ..
            } => {
                use webrtc::data_channel::data_channel_state::RTCDataChannelState;

                // Wait for DataChannel to open (max 5 seconds)
                let start = tokio::time::Instant::now();
                loop {
                    let state = data_channel.ready_state();
                    if state == RTCDataChannelState::Open {
                        break;
                    }
                    if state == RTCDataChannelState::Closed || state == RTCDataChannelState::Closing
                    {
                        return Err(NetworkError::DataChannelError(format!(
                            "DataChannel closed: {state:?}"
                        )));
                    }
                    if start.elapsed() > std::time::Duration::from_secs(5) {
                        return Err(NetworkError::DataChannelError(format!(
                            "DataChannel open timeout: {state:?}"
                        )));
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }

                let msg_id = msg_id_counter.fetch_add(1, Ordering::Relaxed);
                let data_len = data.len();

                if data_len <= DC_MAX_PAYLOAD_SIZE {
                    // Single fragment (total_frags = 1)
                    let mut buf = Vec::with_capacity(FRAGMENT_HEADER_SIZE + data_len);
                    encode_fragment_header(&mut buf, msg_id, 0, 1);
                    buf.extend_from_slice(&data);
                    let frame = bytes::Bytes::from(buf);
                    data_channel
                        .send(&frame)
                        .await
                        .map_err(|e| NetworkError::DataChannelError(format!("Send failed: {e}")))?;
                    tracing::trace!(
                        "sent single fragment: msg_id={} payload={} bytes",
                        msg_id,
                        data_len
                    );
                } else {
                    // Multi-fragment send
                    let total_frags = data_len.div_ceil(DC_MAX_PAYLOAD_SIZE);
                    if total_frags > u16::MAX as usize {
                        return Err(NetworkError::DataChannelError(format!(
                            "message too large: {data_len} bytes would require {total_frags} fragments (max {})",
                            u16::MAX
                        )));
                    }
                    let total_frags = total_frags as u16;
                    tracing::debug!(
                        "fragmenting message: msg_id={} total_bytes={} fragments={}",
                        msg_id,
                        data_len,
                        total_frags
                    );
                    for (frag_index, chunk) in data.chunks(DC_MAX_PAYLOAD_SIZE).enumerate() {
                        let mut buf = Vec::with_capacity(FRAGMENT_HEADER_SIZE + chunk.len());
                        encode_fragment_header(&mut buf, msg_id, frag_index as u16, total_frags);
                        buf.extend_from_slice(chunk);
                        let frame = bytes::Bytes::from(buf);
                        data_channel.send(&frame).await.map_err(|e| {
                            NetworkError::DataChannelError(format!(
                                "Send fragment {frag_index} failed: {e}"
                            ))
                        })?;
                        tracing::debug!(
                            "sent fragment {}/{}: msg_id={} chunk={} bytes",
                            frag_index + 1,
                            total_frags,
                            msg_id,
                            chunk.len()
                        );
                    }
                }
                Ok(())
            }

            DataLane::Mpsc { .. } => {
                // Mpsc DataLane should use send_envelope() instead of send(bytes)
                Err(NetworkError::InvalidOperation(
                    "Mpsc DataLane requires send_envelope(), not send(bytes)".to_string(),
                ))
            }

            DataLane::WebSocket {
                sink, payload_type, ..
            } => {
                // 1. Encapsulate message (add PayloadType header)
                let mut buf = Vec::with_capacity(5 + data.len());

                // 1 byte: payload_type
                buf.push(*payload_type as u8);

                // 4 bytes: data length (big-endian)
                let len = data.len() as u32;
                buf.extend_from_slice(&len.to_be_bytes());

                // N bytes: data (copy from Bytes to Vec)
                buf.extend_from_slice(&data);

                // 2. Send to WebSocket
                let mut sink_opt = sink.lock().await;
                if let Some(s) = sink_opt.as_mut() {
                    s.send(WsMessage::Binary(buf.into())).await.map_err(|e| {
                        NetworkError::SendError(format!("WebSocket send failed: {e}"))
                    })?;

                    tracing::trace!(
                        "📤 WebSocket sent {} bytes (type={:?})",
                        data.len(),
                        payload_type
                    );
                    Ok(())
                } else {
                    Err(NetworkError::ConnectionError(
                        "WebSocket not connected".to_string(),
                    ))
                }
            }
        }
    }

    /// Send RpcEnvelope (Inproc only, zero serialization)
    ///
    /// # Arguments
    /// - `envelope`: RpcEnvelope object
    ///
    /// # Description
    /// This method is only for `DataLane::Mpsc`, directly passing RpcEnvelope objects,
    /// without serialization/deserialization, achieving zero-copy intra-process communication.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use actr_protocol::RpcEnvelope;
    /// let envelope = RpcEnvelope { /* ... */ };
    /// data_lane.send_envelope(envelope).await?;
    /// ```
    #[cfg_attr(
        feature = "opentelemetry",
        tracing::instrument(skip_all, name = "DataLane.send_envelope")
    )]
    pub async fn send_envelope(&self, envelope: actr_protocol::RpcEnvelope) -> NetworkResult<()> {
        match self {
            DataLane::Mpsc { tx, .. } => {
                tx.send(envelope)
                    .await
                    .map_err(|_| NetworkError::ChannelClosed("Mpsc channel closed".to_string()))?;

                tracing::trace!("📤 Mpsc sent RpcEnvelope");
                Ok(())
            }
            _ => Err(NetworkError::InvalidOperation(
                "send_envelope() only supports Mpsc DataLane".to_string(),
            )),
        }
    }

    /// Receive message
    ///
    /// Blocks until a message is received or the channel is closed.
    ///
    /// # Returns
    /// - `Ok(Bytes)`: received message data (zero-copy)
    /// - `Err`: channel closed or other error
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let data = data_lane.recv().await?;
    /// println!("Received {} bytes", data.len());
    /// ```
    pub async fn recv(&self) -> NetworkResult<bytes::Bytes> {
        match self {
            DataLane::WebRtcDataChannel { rx, reassembly, .. } => {
                loop {
                    let raw = {
                        let mut receiver = rx.lock().await;
                        receiver.recv().await.ok_or_else(|| {
                            NetworkError::ChannelClosed("DataLane receiver closed".to_string())
                        })?
                    };
                    let (msg_id, frag_index, total_frags, payload) = decode_fragment_header(raw)?;
                    if total_frags == 1 {
                        // Single-fragment fast path
                        return Ok(payload);
                    }
                    // Multi-fragment: accumulate and reassemble
                    let mut buf = reassembly.lock().await;
                    if let Some(complete) = buf.insert(msg_id, frag_index, total_frags, payload) {
                        tracing::debug!(
                            "reassembled message: msg_id={} total_bytes={}",
                            msg_id,
                            complete.len()
                        );
                        return Ok(complete);
                    }
                    // Fragment stored; wait for the rest
                }
            }
            DataLane::WebSocket { rx, .. } => {
                let mut receiver = rx.lock().await;
                receiver.recv().await.ok_or_else(|| {
                    NetworkError::ChannelClosed("DataLane receiver closed".to_string())
                })
            }
            DataLane::Mpsc { .. } => {
                // Mpsc DataLane should use recv_envelope() instead of recv()
                Err(NetworkError::InvalidOperation(
                    "Mpsc DataLane requires recv_envelope(), not recv()".to_string(),
                ))
            }
        }
    }

    /// Receive RpcEnvelope (Inproc only)
    ///
    /// # Returns
    /// - `Ok(RpcEnvelope)`: received message object
    /// - `Err`: channel closed
    ///
    /// # Description
    /// This method is only for `DataLane::Mpsc`, directly receiving RpcEnvelope objects, zero-copy.
    pub async fn recv_envelope(&self) -> NetworkResult<actr_protocol::RpcEnvelope> {
        match self {
            DataLane::Mpsc { rx, .. } => {
                let mut receiver = rx.lock().await;
                receiver
                    .recv()
                    .await
                    .ok_or_else(|| NetworkError::ChannelClosed("Mpsc channel closed".to_string()))
            }
            _ => Err(NetworkError::InvalidOperation(
                "recv_envelope() only supports Mpsc DataLane".to_string(),
            )),
        }
    }

    /// Try to receive message (non-blocking)
    ///
    /// # Returns
    /// - `Ok(Some(data))`: received message (zero-copy)
    /// - `Ok(None)`: no message available
    /// - `Err`: channel closed or other error
    pub async fn try_recv(&self) -> NetworkResult<Option<bytes::Bytes>> {
        match self {
            DataLane::WebRtcDataChannel { rx, reassembly, .. } => {
                // Drain available fragments until we either assemble a complete message
                // or the channel has no more pending data.
                loop {
                    let raw = {
                        let mut receiver = rx.lock().await;
                        match receiver.try_recv() {
                            Ok(data) => data,
                            Err(mpsc::error::TryRecvError::Empty) => return Ok(None),
                            Err(mpsc::error::TryRecvError::Disconnected) => {
                                return Err(NetworkError::ChannelClosed(
                                    "Lane receiver closed".to_string(),
                                ));
                            }
                        }
                    };
                    let (msg_id, frag_index, total_frags, payload) = decode_fragment_header(raw)?;
                    if total_frags == 1 {
                        return Ok(Some(payload));
                    }
                    let mut buf = reassembly.lock().await;
                    if let Some(complete) = buf.insert(msg_id, frag_index, total_frags, payload) {
                        tracing::debug!(
                            "reassembled message (try_recv): msg_id={} total_bytes={}",
                            msg_id,
                            complete.len()
                        );
                        return Ok(Some(complete));
                    }
                    // Fragment stored, try to read the next one immediately
                }
            }
            DataLane::WebSocket { rx, .. } => {
                let mut receiver = rx.lock().await;
                match receiver.try_recv() {
                    Ok(data) => Ok(Some(data)),
                    Err(mpsc::error::TryRecvError::Empty) => Ok(None),
                    Err(mpsc::error::TryRecvError::Disconnected) => Err(
                        NetworkError::ChannelClosed("Lane receiver closed".to_string()),
                    ),
                }
            }
            DataLane::Mpsc { .. } => {
                // Mpsc Lane should use try_recv_envelope()
                Err(NetworkError::InvalidOperation(
                    "Mpsc Lane requires try_recv_envelope(), not try_recv()".to_string(),
                ))
            }
        }
    }

    /// Get DataLane type name (for logging)
    #[inline]
    pub fn lane_type(&self) -> &'static str {
        match self {
            DataLane::WebRtcDataChannel { .. } => "WebRtcDataChannel",
            DataLane::Mpsc { .. } => "Mpsc",
            DataLane::WebSocket { .. } => "WebSocket",
        }
    }
}

impl std::fmt::Debug for DataLane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataLane::WebRtcDataChannel { .. } => write!(f, "DataLane::WebRtcDataChannel(..)"),
            DataLane::Mpsc { .. } => write!(f, "DataLane::Mpsc(..)"),
            DataLane::WebSocket { payload_type, .. } => {
                write!(f, "DataLane::WebSocket(type={payload_type:?})")
            }
        }
    }
}

/// DataLane factory methods
impl DataLane {
    /// Create Mpsc DataLane (accepts plain Receiver)
    ///
    /// # Arguments
    /// - `payload_type`: PayloadType identifier
    /// - `tx`: send channel (directly passes RpcEnvelope)
    /// - `rx`: receive channel (automatically wrapped in Arc<Mutex<>>)
    #[inline]
    pub fn mpsc(
        payload_type: PayloadType,
        tx: mpsc::Sender<actr_protocol::RpcEnvelope>,
        rx: mpsc::Receiver<actr_protocol::RpcEnvelope>,
    ) -> Self {
        DataLane::Mpsc {
            payload_type,
            tx,
            rx: Arc::new(Mutex::new(rx)),
        }
    }

    /// Create Mpsc DataLane (accepts shared Receiver)
    ///
    /// # Arguments
    /// - `payload_type`: PayloadType identifier
    /// - `tx`: send channel (directly passes RpcEnvelope)
    /// - `rx`: shared receive channel
    #[inline]
    pub fn mpsc_shared(
        payload_type: PayloadType,
        tx: mpsc::Sender<actr_protocol::RpcEnvelope>,
        rx: Arc<Mutex<mpsc::Receiver<actr_protocol::RpcEnvelope>>>,
    ) -> Self {
        DataLane::Mpsc {
            payload_type,
            tx,
            rx,
        }
    }

    /// Create WebRTC DataChannel DataLane
    ///
    /// # Arguments
    /// - `data_channel`: DataChannel reference
    /// - `rx`: receive channel (Bytes zero-copy)
    #[inline]
    pub fn webrtc_data_channel(
        data_channel: Arc<RTCDataChannel>,
        rx: mpsc::Receiver<bytes::Bytes>,
    ) -> Self {
        DataLane::WebRtcDataChannel {
            data_channel,
            rx: Arc::new(Mutex::new(rx)),
            msg_id_counter: Arc::new(AtomicU32::new(0)),
            reassembly: Arc::new(Mutex::new(ReassemblyBuffer::new())),
        }
    }

    /// Create WebSocket DataLane
    ///
    /// # Arguments
    /// - `sink`: shared WebSocket Sink (may not be connected yet, uses Option)
    /// - `payload_type`: message type identifier
    /// - `rx`: receive channel (Bytes zero-copy)
    #[inline]
    pub fn websocket(
        sink: WsSink,
        payload_type: PayloadType,
        rx: mpsc::Receiver<bytes::Bytes>,
    ) -> Self {
        DataLane::WebSocket {
            sink,
            payload_type,
            rx: Arc::new(Mutex::new(rx)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    // ── Fragment header encode / decode ───────────────────────────────────────

    #[test]
    fn test_encode_decode_fragment_header_single() {
        let mut buf = Vec::new();
        encode_fragment_header(&mut buf, 42, 0, 1);
        assert_eq!(buf.len(), FRAGMENT_HEADER_SIZE);

        let payload = Bytes::from(b"hello".as_slice().to_vec());
        let mut raw = buf;
        raw.extend_from_slice(&payload);

        let (msg_id, frag_index, total_frags, decoded_payload) =
            decode_fragment_header(Bytes::from(raw)).unwrap();
        assert_eq!(msg_id, 42);
        assert_eq!(frag_index, 0);
        assert_eq!(total_frags, 1);
        assert_eq!(decoded_payload, payload);
    }

    #[test]
    fn test_encode_decode_fragment_header_multi() {
        let mut buf = Vec::new();
        encode_fragment_header(&mut buf, 0xDEAD_BEEF, 3, 7);
        let (msg_id, frag_index, total_frags, _) =
            decode_fragment_header(Bytes::from(buf)).unwrap();
        assert_eq!(msg_id, 0xDEAD_BEEF);
        assert_eq!(frag_index, 3);
        assert_eq!(total_frags, 7);
    }

    #[test]
    fn test_decode_too_short_returns_error() {
        let short = Bytes::from_static(b"short");
        assert!(decode_fragment_header(short).is_err());
    }

    // ── ReassemblyBuffer ─────────────────────────────────────────────────────

    #[test]
    fn test_reassembly_single_fragment() {
        let mut buf = ReassemblyBuffer::new();
        let payload = Bytes::from_static(b"single");
        // total_frags = 1 means a single fragment; inserting it should complete immediately.
        let result = buf.insert(1, 0, 1, payload.clone());
        assert_eq!(result, Some(payload));
    }

    #[test]
    fn test_reassembly_two_fragments_in_order() {
        let mut buf = ReassemblyBuffer::new();
        let part0 = Bytes::from_static(b"hello ");
        let part1 = Bytes::from_static(b"world");

        // First fragment – not complete yet
        assert!(buf.insert(5, 0, 2, part0).is_none());
        // Second fragment – complete
        let result = buf.insert(5, 1, 2, part1).unwrap();
        assert_eq!(result, Bytes::from_static(b"hello world"));
    }

    #[test]
    fn test_reassembly_two_fragments_out_of_order() {
        let mut buf = ReassemblyBuffer::new();
        let part0 = Bytes::from_static(b"hello ");
        let part1 = Bytes::from_static(b"world");

        // Second fragment arrives first
        assert!(buf.insert(7, 1, 2, part1).is_none());
        // First fragment arrives – triggers reassembly
        let result = buf.insert(7, 0, 2, part0).unwrap();
        assert_eq!(result, Bytes::from_static(b"hello world"));
    }

    #[test]
    fn test_reassembly_multiple_messages_interleaved() {
        let mut buf = ReassemblyBuffer::new();

        // Two messages interleaved
        assert!(buf.insert(1, 0, 2, Bytes::from_static(b"A1")).is_none());
        assert!(buf.insert(2, 0, 2, Bytes::from_static(b"B1")).is_none());
        assert!(buf.insert(1, 1, 2, Bytes::from_static(b"A2")).is_some()); // msg 1 done
        let msg2 = buf.insert(2, 1, 2, Bytes::from_static(b"B2")).unwrap();
        assert_eq!(msg2, Bytes::from_static(b"B1B2"));
    }

    // ── Fragment count calculation ────────────────────────────────────────────

    #[test]
    fn test_fragment_count_small_message() {
        let size = DC_MAX_PAYLOAD_SIZE;
        let count = size.div_ceil(DC_MAX_PAYLOAD_SIZE);
        assert_eq!(
            count, 1,
            "message equal to payload size should be 1 fragment"
        );
    }

    #[test]
    fn test_fragment_count_one_byte_over() {
        let size = DC_MAX_PAYLOAD_SIZE + 1;
        let count = size.div_ceil(DC_MAX_PAYLOAD_SIZE);
        assert_eq!(count, 2, "one byte over should require 2 fragments");
    }

    #[test]
    fn test_fragment_count_200kb() {
        let size: usize = 200 * 1024; // 200 KB
        let count = size.div_ceil(DC_MAX_PAYLOAD_SIZE);
        // 200*1024 / (64*1024 - 8) = 204800 / 65528 ≈ 3.126 → 4 fragments
        assert_eq!(count, 4);
    }

    // ── Round-trip via mpsc (simulated channel, no real DataChannel) ──────────

    /// Helper: build a framed bytes buffer as the DataChannel would produce it.
    fn make_frame(msg_id: u32, frag_index: u16, total_frags: u16, payload: &[u8]) -> Bytes {
        let mut buf = Vec::with_capacity(FRAGMENT_HEADER_SIZE + payload.len());
        encode_fragment_header(&mut buf, msg_id, frag_index, total_frags);
        buf.extend_from_slice(payload);
        Bytes::from(buf)
    }

    /// Simulate what the recv() loop does: decode header from a raw frame.
    fn recv_one(raw: Bytes, reassembly: &mut ReassemblyBuffer) -> Option<Bytes> {
        let (msg_id, frag_index, total_frags, payload) = decode_fragment_header(raw).unwrap();
        if total_frags == 1 {
            return Some(payload);
        }
        reassembly.insert(msg_id, frag_index, total_frags, payload)
    }

    #[test]
    fn test_roundtrip_small_message() {
        let data = b"small message";
        let frame = make_frame(0, 0, 1, data);
        let mut buf = ReassemblyBuffer::new();
        let result = recv_one(frame, &mut buf).unwrap();
        assert_eq!(result.as_ref(), data);
    }

    #[test]
    fn test_roundtrip_exactly_max_payload() {
        let data = vec![0xABu8; DC_MAX_PAYLOAD_SIZE];
        let frame = make_frame(1, 0, 1, &data);
        let mut buf = ReassemblyBuffer::new();
        let result = recv_one(frame, &mut buf).unwrap();
        assert_eq!(result.as_ref(), data.as_slice());
    }

    #[test]
    fn test_roundtrip_one_byte_over_max_payload() {
        let data = vec![0xCDu8; DC_MAX_PAYLOAD_SIZE + 1];
        let (part0, part1) = data.split_at(DC_MAX_PAYLOAD_SIZE);

        let frame0 = make_frame(2, 0, 2, part0);
        let frame1 = make_frame(2, 1, 2, part1);

        let mut buf = ReassemblyBuffer::new();
        assert!(recv_one(frame0, &mut buf).is_none());
        let result = recv_one(frame1, &mut buf).unwrap();
        assert_eq!(result.as_ref(), data.as_slice());
    }

    #[test]
    fn test_roundtrip_200kb_message() {
        let data: Vec<u8> = (0u8..=255).cycle().take(200 * 1024).collect();
        let total_frags = data.len().div_ceil(DC_MAX_PAYLOAD_SIZE) as u16;

        let mut buf = ReassemblyBuffer::new();
        let mut result = None;
        for (i, chunk) in data.chunks(DC_MAX_PAYLOAD_SIZE).enumerate() {
            let frame = make_frame(99, i as u16, total_frags, chunk);
            result = recv_one(frame, &mut buf);
        }
        let result = result.unwrap();
        assert_eq!(result.as_ref(), data.as_slice());
    }

    #[tokio::test]
    async fn test_mpsc_lane() {
        use actr_protocol::RpcEnvelope;

        let (tx, rx) = mpsc::channel(10);
        let lane = DataLane::mpsc(PayloadType::RpcReliable, tx.clone(), rx);

        // Send message (using RpcEnvelope)
        let envelope = RpcEnvelope {
            request_id: "test-1".to_string(),
            route_key: "test.route".to_string(),
            payload: Some(Bytes::from_static(b"hello")),
            traceparent: None,
            tracestate: None,
            metadata: vec![],
            timeout_ms: 30000,
            error: None,
        };
        lane.send_envelope(envelope.clone()).await.unwrap();

        // Receive message
        let received = lane.recv_envelope().await.unwrap();
        assert_eq!(received.request_id, "test-1");
        assert_eq!(received.payload, Some(Bytes::from_static(b"hello")));
    }

    #[tokio::test]
    async fn test_mpsc_lane_clone() {
        use actr_protocol::RpcEnvelope;

        let (tx, rx) = mpsc::channel(10);
        let lane = DataLane::mpsc(PayloadType::RpcReliable, tx.clone(), rx);

        // Clone lane
        let lane2 = lane.clone();

        // Send via lane
        let envelope = RpcEnvelope {
            request_id: "test-2".to_string(),
            route_key: "test.route".to_string(),
            payload: Some(Bytes::from_static(b"test")),
            traceparent: None,
            tracestate: None,
            metadata: vec![],
            timeout_ms: 30000,
            error: None,
        };
        lane.send_envelope(envelope.clone()).await.unwrap();

        // Receive via lane2
        let received = lane2.recv_envelope().await.unwrap();
        assert_eq!(received.request_id, "test-2");
        assert_eq!(received.payload, Some(Bytes::from_static(b"test")));
    }

    #[tokio::test]
    async fn test_mpsc_lane_with_shared_rx() {
        use actr_protocol::RpcEnvelope;

        let (tx, rx) = mpsc::channel(10);
        let rx_shared = Arc::new(Mutex::new(rx));

        // Use shared rx
        let lane = DataLane::mpsc_shared(PayloadType::RpcReliable, tx.clone(), rx_shared.clone());

        let envelope = RpcEnvelope {
            request_id: "test-3".to_string(),
            route_key: "test.route".to_string(),
            payload: Some(Bytes::from_static(b"shared")),
            traceparent: None,
            tracestate: None,
            metadata: vec![],
            timeout_ms: 30000,
            error: None,
        };
        lane.send_envelope(envelope.clone()).await.unwrap();

        let received = lane.recv_envelope().await.unwrap();
        assert_eq!(received.request_id, "test-3");
        assert_eq!(received.payload, Some(Bytes::from_static(b"shared")));
    }

    #[test]
    fn test_lane_type_name() {
        let (tx, rx) = mpsc::channel(10);
        let lane = DataLane::mpsc(PayloadType::RpcReliable, tx, rx);
        assert_eq!(lane.lane_type(), "Mpsc");
    }
}
