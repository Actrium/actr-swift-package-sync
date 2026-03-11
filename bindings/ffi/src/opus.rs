use crate::error::{ActrError, ActrResult};
use opus::{Application, Bitrate, Channels, Encoder};
use parking_lot::Mutex;
use std::sync::Arc;

const MAX_OPUS_PACKET_SIZE: usize = 1500;

/// Reusable Opus encoder for Swift audio capture.
#[derive(uniffi::Object)]
pub struct OpusEncoder {
    inner: Mutex<Encoder>,
    frame_size: usize,
    channels: usize,
}

#[uniffi::export]
impl OpusEncoder {
    /// Create an Opus encoder for fixed-size PCM float frames.
    #[uniffi::constructor]
    pub fn new(sample_rate: u32, channels: u8, frame_size: u16) -> ActrResult<Arc<Self>> {
        let channels = usize::from(channels);
        let frame_size = usize::from(frame_size);
        let opus_channels = match channels {
            1 => Channels::Mono,
            2 => Channels::Stereo,
            _ => {
                return Err(ActrError::InternalError {
                    msg: format!("Unsupported Opus channel count: {channels}"),
                });
            }
        };

        let mut encoder =
            Encoder::new(sample_rate, opus_channels, Application::Audio).map_err(|err| {
                ActrError::InternalError {
                    msg: format!("Failed to create Opus encoder: {err}"),
                }
            })?;
        encoder
            .set_bitrate(Bitrate::Bits(32_000))
            .map_err(|err| ActrError::InternalError {
                msg: format!("Failed to configure Opus bitrate: {err}"),
            })?;

        Ok(Arc::new(Self {
            inner: Mutex::new(encoder),
            frame_size,
            channels,
        }))
    }

    /// Encode one PCM float frame into one Opus packet.
    pub fn encode(&self, pcm: Vec<f32>) -> ActrResult<Vec<u8>> {
        let expected_len = self.frame_size * self.channels;
        if pcm.len() != expected_len {
            return Err(ActrError::InternalError {
                msg: format!(
                    "Expected {expected_len} PCM samples for Opus frame, got {}",
                    pcm.len()
                ),
            });
        }

        let mut output = vec![0_u8; MAX_OPUS_PACKET_SIZE];
        let encoded_len = self
            .inner
            .lock()
            .encode_float(&pcm, &mut output)
            .map_err(|err| ActrError::InternalError {
                msg: format!("Failed to encode Opus frame: {err}"),
            })?;
        output.truncate(encoded_len);
        Ok(output)
    }

    pub fn frame_size(&self) -> u16 {
        self.frame_size as u16
    }
}
