//! Deterministic, host-network-free Fetch backend for KotoSim (KOTO-0245).

use koto_core::{BackendPoll, FetchBackend, FetchError, FetchRequestId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FakeFetchTerminal {
    Complete,
    Failed(FetchError),
}

/// A scripted response advances by poll count, never host time. The body is
/// copied incrementally and no DNS/socket operation is performed.
#[derive(Clone, Debug)]
pub struct FakeFetchBackend {
    available: bool,
    status: u16,
    pending_polls: u8,
    polls: u8,
    template: Vec<u8>,
    body: Vec<u8>,
    offset: usize,
    request: Option<FetchRequestId>,
    terminal: FakeFetchTerminal,
    cancelled: bool,
}

impl FakeFetchBackend {
    pub fn response(status: u16, body: &[u8], pending_polls: u8) -> Self {
        Self {
            available: true,
            status,
            pending_polls,
            polls: 0,
            template: body.to_vec(),
            body: body.to_vec(),
            offset: 0,
            request: None,
            terminal: FakeFetchTerminal::Complete,
            cancelled: false,
        }
    }

    pub fn failure(error: FetchError, pending_polls: u8) -> Self {
        let mut backend = Self::response(0, &[], pending_polls);
        backend.terminal = FakeFetchTerminal::Failed(error);
        backend
    }

    pub fn offline() -> Self {
        let mut backend = Self::failure(FetchError::Unavailable, 0);
        backend.available = false;
        backend
    }

    pub const fn cancelled(&self) -> bool {
        self.cancelled
    }

    /// Re-script a scenario's next response without constructing a new service
    /// (KOTO-0247). The caller tears down and re-generations the owning
    /// `AppFetchService` so no live request survives the change.
    pub fn configure_response(&mut self, status: u16, body: &[u8], pending_polls: u8) {
        self.available = true;
        self.status = status;
        self.pending_polls = pending_polls;
        self.polls = 0;
        self.template = body.to_vec();
        self.body = body.to_vec();
        self.offset = 0;
        self.request = None;
        self.terminal = FakeFetchTerminal::Complete;
        self.cancelled = false;
    }

    pub fn configure_failure(&mut self, error: FetchError, pending_polls: u8) {
        self.configure_response(0, &[], pending_polls);
        self.terminal = FakeFetchTerminal::Failed(error);
    }

    pub fn configure_offline(&mut self) {
        self.configure_failure(FetchError::Unavailable, 0);
        self.available = false;
    }

    fn owns(&self, request: FetchRequestId) -> Result<(), FetchError> {
        if self.request == Some(request) && !self.cancelled {
            Ok(())
        } else {
            Err(FetchError::StaleRequest)
        }
    }
}

impl FetchBackend for FakeFetchBackend {
    fn available(&self) -> bool {
        self.available
    }

    fn start(
        &mut self,
        request: FetchRequestId,
        _url: &str,
        _pins: koto_core::FetchPinSet,
    ) -> Result<(), FetchError> {
        if !self.available {
            return Err(FetchError::Unavailable);
        }
        self.request = Some(request);
        self.polls = 0;
        self.offset = 0;
        self.cancelled = false;
        self.body.clone_from(&self.template);
        Ok(())
    }

    fn poll(&mut self, request: FetchRequestId) -> BackendPoll {
        if let Err(error) = self.owns(request) {
            return BackendPoll::Failed(error);
        }
        self.polls = self.polls.saturating_add(1);
        if self.polls <= self.pending_polls {
            return BackendPoll::Pending;
        }
        if let FakeFetchTerminal::Failed(error) = self.terminal {
            return BackendPoll::Failed(error);
        }
        if self.polls == self.pending_polls.saturating_add(1) {
            BackendPoll::Headers {
                status: self.status,
            }
        } else if self.offset < self.body.len() {
            BackendPoll::Body
        } else {
            BackendPoll::Complete
        }
    }

    fn read(&mut self, request: FetchRequestId, dst: &mut [u8]) -> Result<usize, FetchError> {
        self.owns(request)?;
        let count = dst.len().min(self.body.len().saturating_sub(self.offset));
        dst[..count].copy_from_slice(&self.body[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }

    fn cancel(&mut self, request: FetchRequestId) {
        if self.request == Some(request) {
            self.cancelled = true;
            self.body.fill(0);
            self.offset = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use koto_core::{AppContext, AppFetchService, FetchAllowlist, FetchOrigin, FetchPoll};

    fn app() -> AppContext {
        AppContext {
            app_id: 9,
            generation: 1,
        }
    }

    fn permission() -> FetchAllowlist {
        let mut result = FetchAllowlist::empty();
        result
            .push(FetchOrigin::parse("https://weather.example").unwrap())
            .unwrap();
        result
    }

    #[test]
    fn scripted_success_is_pending_then_incrementally_readable() {
        let mut service = AppFetchService::new(FakeFetchBackend::response(200, b"abcdef", 2));
        let id = service
            .start(app(), &permission(), "https://weather.example/current", 0)
            .unwrap();
        assert_eq!(service.poll(app(), id, 1), Ok(FetchPoll::Pending));
        assert_eq!(service.poll(app(), id, 2), Ok(FetchPoll::Pending));
        assert_eq!(
            service.poll(app(), id, 3),
            Ok(FetchPoll::Headers { status: 200 })
        );
        let mut chunk = [0; 3];
        assert_eq!(service.read(app(), id, &mut chunk), Ok(3));
        assert_eq!(&chunk, b"abc");
        assert_eq!(service.poll(app(), id, 4), Ok(FetchPoll::Body));
        assert_eq!(service.read(app(), id, &mut chunk), Ok(3));
        assert_eq!(service.poll(app(), id, 5), Ok(FetchPoll::Complete));
    }

    #[test]
    fn fixed_failures_offline_and_cancel_need_no_host_network() {
        for error in [
            FetchError::Dns,
            FetchError::ForbiddenAddress,
            FetchError::Protocol,
            FetchError::Disconnected,
        ] {
            let mut service = AppFetchService::new(FakeFetchBackend::failure(error, 0));
            let id = service
                .start(app(), &permission(), "https://weather.example/current", 0)
                .unwrap();
            assert_eq!(service.poll(app(), id, 1), Ok(FetchPoll::Failed(error)));
        }
        let mut offline = AppFetchService::new(FakeFetchBackend::offline());
        assert_eq!(
            offline.start(app(), &permission(), "https://weather.example/current", 0),
            Err(FetchError::Unavailable)
        );

        let mut service = AppFetchService::new(FakeFetchBackend::response(200, b"secret", 0));
        let id = service
            .start(app(), &permission(), "https://weather.example/current", 0)
            .unwrap();
        service.cancel(app(), id).unwrap();
        assert!(service.backend_mut().cancelled());
        assert_eq!(
            service.read(app(), id, &mut [0; 1]),
            Err(FetchError::StaleRequest)
        );

        let restarted = service
            .start(app(), &permission(), "https://weather.example/current", 2)
            .unwrap();
        assert_eq!(
            service.poll(app(), restarted, 3),
            Ok(FetchPoll::Headers { status: 200 })
        );
        let mut restored = [0; 6];
        assert_eq!(service.read(app(), restarted, &mut restored), Ok(6));
        assert_eq!(&restored, b"secret");
    }
}
