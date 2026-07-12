use crate::{AudioError, AudioResult};

/// Default undecided sample rate placeholder for v0.
///
/// TODO(KA-M8-002): replace or configure this after PicoCalc backend measurement.
pub const DEFAULT_SAMPLE_RATE_HZ: u32 = 16_000;

/// Default undecided mixer block size placeholder for v0.
///
/// TODO(KA-M8-002): replace or configure this after PicoCalc backend measurement.
pub const DEFAULT_BLOCK_FRAMES: u16 = 128;

/// Default v0 SFX source limit.
pub const DEFAULT_MAX_SFX_SOURCES: u8 = 4;

/// Default bounded source queue depth.
pub const DEFAULT_SOURCE_QUEUE_DEPTH: u8 = 8;

/// Default bounded event queue depth.
pub const DEFAULT_EVENT_QUEUE_DEPTH: u8 = 16;

/// Fixed limits for the v0 bounded audio runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioLimits {
    /// Mixer/runtime sample rate in hertz.
    pub sample_rate_hz: u32,
    /// Fixed mixer block length in mono frames.
    pub block_frames: u16,
    /// Maximum concurrent SFX sources.
    pub max_sfx_sources: u8,
    /// Maximum queued source requests.
    pub source_queue_depth: u8,
    /// Maximum queued events.
    pub event_queue_depth: u8,
}

impl AudioLimits {
    /// Returns the current v0 default limits.
    pub const fn v0_default() -> Self {
        Self {
            sample_rate_hz: DEFAULT_SAMPLE_RATE_HZ,
            block_frames: DEFAULT_BLOCK_FRAMES,
            max_sfx_sources: DEFAULT_MAX_SFX_SOURCES,
            source_queue_depth: DEFAULT_SOURCE_QUEUE_DEPTH,
            event_queue_depth: DEFAULT_EVENT_QUEUE_DEPTH,
        }
    }

    /// Validates that all bounded runtime limits are non-zero.
    pub fn validate(self) -> AudioResult<Self> {
        if self.sample_rate_hz == 0
            || self.block_frames == 0
            || self.max_sfx_sources == 0
            || self.source_queue_depth == 0
            || self.event_queue_depth == 0
        {
            return Err(AudioError::InvalidArgument);
        }

        Ok(self)
    }

    /// Returns the configured concurrent SFX source count.
    pub const fn sfx_source_count(self) -> u8 {
        self.max_sfx_sources
    }
}

impl Default for AudioLimits {
    fn default() -> Self {
        Self::v0_default()
    }
}

/// Admission behavior when bounded source capacity is exhausted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DropPolicy {
    /// Reject the new request immediately.
    RejectNew,
    /// Drop the new request and report it through counters/events.
    DropNew,
    /// Reserve a future voice stealing path without implementing it yet.
    ///
    /// TODO(KA-M1-004): connect priority-based source stealing.
    AllowSteal,
}

/// Runtime policy owned by the audio service/system layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioPolicy {
    /// Static bounded limits.
    pub limits: AudioLimits,
    /// Source capacity behavior.
    pub drop_policy: DropPolicy,
    /// Inclusive minimum logical volume.
    pub min_volume: u16,
    /// Inclusive maximum logical volume.
    pub max_volume: u16,
    /// Default logical volume for new sources.
    pub default_volume: u16,
}

impl AudioPolicy {
    /// Returns the current v0 default policy.
    pub const fn v0_default() -> Self {
        Self {
            limits: AudioLimits::v0_default(),
            drop_policy: DropPolicy::RejectNew,
            min_volume: 0,
            max_volume: 256,
            default_volume: 256,
        }
    }

    /// Validates basic policy invariants.
    pub fn validate(self) -> AudioResult<Self> {
        self.limits.validate()?;

        if self.min_volume > self.max_volume
            || self.default_volume < self.min_volume
            || self.default_volume > self.max_volume
        {
            return Err(AudioError::InvalidArgument);
        }

        Ok(self)
    }
}

impl Default for AudioPolicy {
    fn default() -> Self {
        Self::v0_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_is_valid() {
        assert_eq!(
            AudioPolicy::default().validate(),
            Ok(AudioPolicy::default())
        );
    }

    #[test]
    fn zero_limits_are_invalid() {
        let mut limits = AudioLimits::default();
        limits.max_sfx_sources = 0;

        assert_eq!(limits.validate(), Err(AudioError::InvalidArgument));
    }
}
