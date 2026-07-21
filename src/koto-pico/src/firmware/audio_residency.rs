//! Runtime Audio/Wi-Fi residency ownership state machine (KOTO-0227).

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResidencyState {
    FullAudio,
    QuiescingAudio,
    Offline,
    WifiStreamAudio,
    /// Wi-Fi remains online while the permanent stream-audio owner drains and
    /// releases its PCM/scratch/DMA storage for an HTTPS transaction.
    QuiescingStreamForTls,
    /// TLS owns the released stream-audio storage. Every audio API is
    /// temporarily unavailable; ordinary non-TLS Wi-Fi services remain live.
    TlsExclusive,
    /// TLS storage has been erased and released; stream audio is being rebuilt.
    RestoringStreamAfterTls,
    QuiescingWifi,
}

impl Default for ResidencyState {
    fn default() -> Self {
        Self::FullAudio
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResidencyToken {
    generation: u32,
}

impl ResidencyToken {
    pub const fn generation(self) -> u32 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionError {
    InvalidState,
    StaleToken,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioResidencyOwner {
    state: ResidencyState,
    generation: u32,
    transition_failures: u32,
}

impl AudioResidencyOwner {
    pub const fn new() -> Self {
        Self {
            state: ResidencyState::FullAudio,
            generation: 1,
            transition_failures: 0,
        }
    }

    pub const fn state(self) -> ResidencyState {
        self.state
    }

    pub const fn token(self) -> ResidencyToken {
        ResidencyToken {
            generation: self.generation,
        }
    }

    pub const fn transition_failures(self) -> u32 {
        self.transition_failures
    }

    pub const fn rich_audio_available(self, token: ResidencyToken) -> bool {
        matches!(self.state, ResidencyState::FullAudio) && self.token_matches(token)
    }

    pub const fn stream_audio_available(self, token: ResidencyToken) -> bool {
        matches!(
            self.state,
            ResidencyState::FullAudio | ResidencyState::WifiStreamAudio
        ) && self.token_matches(token)
    }

    pub fn begin_wifi(&mut self) -> Result<ResidencyToken, TransitionError> {
        if self.state != ResidencyState::FullAudio {
            return Err(TransitionError::InvalidState);
        }
        self.bump_generation();
        self.state = ResidencyState::QuiescingAudio;
        Ok(self.token())
    }

    pub fn mark_audio_offline(&mut self, token: ResidencyToken) -> Result<(), TransitionError> {
        self.require(token, ResidencyState::QuiescingAudio)?;
        self.state = ResidencyState::Offline;
        Ok(())
    }

    pub fn activate_wifi(&mut self, token: ResidencyToken) -> Result<(), TransitionError> {
        self.require(token, ResidencyState::Offline)?;
        self.state = ResidencyState::WifiStreamAudio;
        Ok(())
    }

    pub fn begin_full_audio(&mut self) -> Result<ResidencyToken, TransitionError> {
        if self.state != ResidencyState::WifiStreamAudio {
            return Err(TransitionError::InvalidState);
        }
        self.bump_generation();
        self.state = ResidencyState::QuiescingWifi;
        Ok(self.token())
    }

    /// Starts the RP2040-only TLS/audio exclusion window. The caller must stop
    /// accepting stream samples immediately, then acknowledge only after PWM,
    /// DMA, worker access, and all buffer borrows are dead.
    pub fn begin_tls_exclusive(&mut self) -> Result<ResidencyToken, TransitionError> {
        if self.state != ResidencyState::WifiStreamAudio {
            return Err(TransitionError::InvalidState);
        }
        self.bump_generation();
        self.state = ResidencyState::QuiescingStreamForTls;
        Ok(self.token())
    }

    /// Transfers the released stream-audio storage to TLS. This state change is
    /// the ownership fence: TLS must not touch those bytes before it succeeds.
    pub fn activate_tls_exclusive(&mut self, token: ResidencyToken) -> Result<(), TransitionError> {
        self.require(token, ResidencyState::QuiescingStreamForTls)?;
        self.state = ResidencyState::TlsExclusive;
        Ok(())
    }

    /// Starts returning storage to stream audio after the TLS connection,
    /// verifier, record buffers, adapter, and decoder have been dropped and
    /// their transient bytes erased.
    pub fn begin_stream_restore_after_tls(&mut self) -> Result<ResidencyToken, TransitionError> {
        if self.state != ResidencyState::TlsExclusive {
            return Err(TransitionError::InvalidState);
        }
        self.bump_generation();
        self.state = ResidencyState::RestoringStreamAfterTls;
        Ok(self.token())
    }

    /// Publishes ordinary Wi-Fi-plus-stream-audio operation after the audio
    /// worker and DMA path have been reconstructed for the current generation.
    pub fn activate_stream_after_tls(
        &mut self,
        token: ResidencyToken,
    ) -> Result<(), TransitionError> {
        self.require(token, ResidencyState::RestoringStreamAfterTls)?;
        self.state = ResidencyState::WifiStreamAudio;
        Ok(())
    }

    pub fn mark_wifi_offline(&mut self, token: ResidencyToken) -> Result<(), TransitionError> {
        self.require(token, ResidencyState::QuiescingWifi)?;
        self.state = ResidencyState::Offline;
        Ok(())
    }

    pub fn activate_full_audio(&mut self, token: ResidencyToken) -> Result<(), TransitionError> {
        self.require(token, ResidencyState::Offline)?;
        self.state = ResidencyState::FullAudio;
        Ok(())
    }

    pub fn fail_to_offline(&mut self) -> ResidencyToken {
        self.bump_generation();
        self.transition_failures = self.transition_failures.saturating_add(1);
        self.state = ResidencyState::Offline;
        self.token()
    }

    const fn token_matches(self, token: ResidencyToken) -> bool {
        self.generation == token.generation
    }

    fn require(
        &self,
        token: ResidencyToken,
        expected: ResidencyState,
    ) -> Result<(), TransitionError> {
        if !self.token_matches(token) {
            return Err(TransitionError::StaleToken);
        }
        if self.state != expected {
            return Err(TransitionError::InvalidState);
        }
        Ok(())
    }

    fn bump_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1).max(1);
    }
}

impl Default for AudioResidencyOwner {
    fn default() -> Self {
        Self::new()
    }
}
