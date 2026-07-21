//! Deterministic, host-network-free MQTT backend for KotoSim (KOTO-0249).
//!
//! The fake broker advances purely by poll count, never host time or a socket.
//! It delivers already-complete messages (the adversarial wire-format matrix
//! lives in `koto_core::mqtt`'s `MqttPacketDecoder` host tests); this layer
//! exercises the app-visible lifecycle: connect delay, retained-then-live
//! delivery, queue-overflow drop-oldest, and clean disconnect / failure.

use koto_core::{
    BackendMqttPoll, MqttBackend, MqttError, MqttMessageQueue, MqttOrigin, MqttSessionId,
    TopicFilter,
};

/// One scripted broker message. `retained` marks a broker-retained value the
/// profile documents as delivered first on subscribe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptedMessage {
    pub topic: Vec<u8>,
    pub payload: Vec<u8>,
    pub retained: bool,
}

impl ScriptedMessage {
    pub fn new(topic: &[u8], payload: &[u8], retained: bool) -> Self {
        Self {
            topic: topic.to_vec(),
            payload: payload.to_vec(),
            retained,
        }
    }
}

/// What the session does once every scripted message has been delivered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SimMqttTerminal {
    /// Stay connected (idle), the steady state for live telemetry.
    Idle,
    /// The broker cleanly drops the session.
    Disconnect,
    /// The session fails with a fixed error.
    Fail(MqttError),
}

/// A scripted broker session. Only one session is live at a time (the frozen
/// `MAX_GLOBAL_MQTT_SESSIONS == 1` profile).
#[derive(Clone, Debug)]
pub struct SimMqttBackend {
    available: bool,
    /// Result of the CONNECT admission itself (an immediate refusal, e.g. a
    /// denied credential grant on a real broker).
    connect_result: Result<(), MqttError>,
    /// Polls spent `Connecting` before CONNACK is accepted.
    connect_delay: u8,
    /// Deliver every pending message on a single poll (queue-overflow test).
    burst: bool,
    terminal: SimMqttTerminal,
    /// Immutable scenario script; `pending` is re-seeded from it on each connect.
    template: Vec<ScriptedMessage>,

    // Live session state.
    session: Option<MqttSessionId>,
    subscribed: Vec<Vec<u8>>,
    pending: Vec<ScriptedMessage>,
    polls: u8,
    connected: bool,
}

impl SimMqttBackend {
    /// A broker that connects after `connect_delay` polls and, once subscribed,
    /// delivers `messages` (in script order), then rests in `terminal`.
    pub fn scenario(
        connect_delay: u8,
        messages: Vec<ScriptedMessage>,
        terminal: SimMqttTerminal,
    ) -> Self {
        Self {
            available: true,
            connect_result: Ok(()),
            connect_delay,
            burst: false,
            terminal,
            template: messages,
            session: None,
            subscribed: Vec::new(),
            pending: Vec::new(),
            polls: 0,
            connected: false,
        }
    }

    /// A broker that refuses the CONNECT admission outright (e.g. a denied or
    /// revoked credential grant).
    pub fn refused(error: MqttError) -> Self {
        let mut backend = Self::scenario(0, Vec::new(), SimMqttTerminal::Idle);
        backend.connect_result = Err(error);
        backend
    }

    /// An offline / unsupported backend (returns a stable `Unavailable`).
    pub fn offline() -> Self {
        let mut backend = Self::scenario(0, Vec::new(), SimMqttTerminal::Idle);
        backend.available = false;
        backend
    }

    /// Deliver every pending message on one poll, so a burst overflows the
    /// eight-deep OS queue and exercises the drop-oldest policy.
    pub fn burst(mut self) -> Self {
        self.burst = true;
        self
    }

    fn matches_subscription(&self, topic: &[u8]) -> bool {
        self.subscribed
            .iter()
            .any(|filter| filter.as_slice() == topic)
    }
}

impl MqttBackend for SimMqttBackend {
    fn available(&self) -> bool {
        self.available
    }

    fn connect(&mut self, session: MqttSessionId, _origin: &MqttOrigin) -> Result<(), MqttError> {
        if !self.available {
            return Err(MqttError::Unavailable);
        }
        self.connect_result?;
        self.session = Some(session);
        self.subscribed.clear();
        self.pending.clone_from(&self.template);
        self.polls = 0;
        self.connected = false;
        Ok(())
    }

    fn subscribe(&mut self, session: MqttSessionId, filter: &TopicFilter) -> Result<(), MqttError> {
        if self.session != Some(session) {
            return Err(MqttError::StaleSession);
        }
        self.subscribed.push(filter.as_bytes().to_vec());
        Ok(())
    }

    fn poll(&mut self, session: MqttSessionId, queue: &mut MqttMessageQueue) -> BackendMqttPoll {
        if self.session != Some(session) {
            return BackendMqttPoll::Failed(MqttError::StaleSession);
        }
        self.polls = self.polls.saturating_add(1);
        if self.polls <= self.connect_delay {
            return BackendMqttPoll::Connecting;
        }
        self.connected = true;

        // Deliver only messages whose topic matches an active subscription.
        if !self.subscribed.is_empty() {
            if self.burst {
                let mut remaining = Vec::new();
                for message in core::mem::take(&mut self.pending) {
                    if self.matches_subscription(&message.topic) {
                        queue.push(&message.topic, &message.payload, message.retained);
                    } else {
                        remaining.push(message);
                    }
                }
                self.pending = remaining;
            } else if let Some(index) = self
                .pending
                .iter()
                .position(|m| self.matches_subscription(&m.topic))
            {
                let message = self.pending.remove(index);
                queue.push(&message.topic, &message.payload, message.retained);
            }
        }

        // The scripted terminal (disconnect / failure) only applies once the app
        // has subscribed and every matching message has been delivered, so a
        // scenario always reaches `Connected` first.
        let terminal_ready = !self.subscribed.is_empty()
            && !self
                .pending
                .iter()
                .any(|m| self.matches_subscription(&m.topic));
        if terminal_ready {
            match self.terminal {
                SimMqttTerminal::Idle => BackendMqttPoll::Connected,
                SimMqttTerminal::Disconnect => BackendMqttPoll::Disconnected,
                SimMqttTerminal::Fail(error) => BackendMqttPoll::Failed(error),
            }
        } else {
            BackendMqttPoll::Connected
        }
    }

    fn disconnect(&mut self, session: MqttSessionId) {
        if self.session == Some(session) {
            self.session = None;
            self.subscribed.clear();
            self.pending.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use koto_core::{
        AppContext, AppMqttService, BrokerAllowlist, MqttOrigin, MqttPoll, TopicFilter,
        TopicFilterSet, MAX_MQTT_MESSAGE_QUEUE,
    };

    fn app() -> AppContext {
        AppContext {
            app_id: 9,
            generation: 1,
        }
    }

    fn brokers() -> BrokerAllowlist {
        let mut set = BrokerAllowlist::empty();
        set.push(MqttOrigin::parse("mqtt://broker.example").unwrap())
            .unwrap();
        set
    }

    fn topics() -> TopicFilterSet {
        let mut set = TopicFilterSet::empty();
        set.push(TopicFilter::parse("sensors/temp").unwrap())
            .unwrap();
        set
    }

    /// Drive connect → connected → subscribe → deliver, poll by poll.
    fn connect_and_subscribe<F: FnOnce() -> SimMqttBackend>(
        make: F,
    ) -> (AppMqttService<SimMqttBackend>, koto_core::MqttSessionId) {
        let mut service = AppMqttService::new(make());
        let id = service.connect(app(), &brokers(), 0, 0).unwrap();
        // Advance until connected.
        let mut now = 0;
        loop {
            now += 16;
            match service.poll(app(), id, now).unwrap() {
                MqttPoll::Connecting => continue,
                MqttPoll::Connected => break,
                other => panic!("unexpected pre-subscribe state {other:?}"),
            }
        }
        service.subscribe(app(), id, &topics(), 0, now).unwrap();
        (service, id)
    }

    #[test]
    fn delivers_a_live_message_after_subscribe() {
        let (mut service, id) = connect_and_subscribe(|| {
            SimMqttBackend::scenario(
                1,
                vec![ScriptedMessage::new(b"sensors/temp", b"21.5", false)],
                SimMqttTerminal::Idle,
            )
        });
        // One more poll delivers the message.
        assert_eq!(service.poll(app(), id, 64).unwrap(), MqttPoll::Message);
        let mut topic = [0u8; 32];
        let mut payload = [0u8; 32];
        let message = service
            .read_message(app(), id, &mut topic, &mut payload)
            .unwrap()
            .unwrap();
        assert_eq!(&topic[..message.topic_len as usize], b"sensors/temp");
        assert_eq!(&payload[..message.payload_len as usize], b"21.5");
        assert!(!message.retained);
    }

    #[test]
    fn retained_message_is_marked_retained() {
        let (mut service, id) = connect_and_subscribe(|| {
            SimMqttBackend::scenario(
                0,
                vec![ScriptedMessage::new(b"sensors/temp", b"20.0", true)],
                SimMqttTerminal::Idle,
            )
        });
        assert_eq!(service.poll(app(), id, 64).unwrap(), MqttPoll::Message);
        let mut topic = [0u8; 32];
        let mut payload = [0u8; 32];
        let message = service
            .read_message(app(), id, &mut topic, &mut payload)
            .unwrap()
            .unwrap();
        assert!(message.retained);
    }

    #[test]
    fn burst_overflows_the_queue_with_drop_oldest() {
        let messages: Vec<ScriptedMessage> = (0..(MAX_MQTT_MESSAGE_QUEUE + 3))
            .map(|n| ScriptedMessage::new(b"sensors/temp", &[n as u8], false))
            .collect();
        let (mut service, id) = connect_and_subscribe(|| {
            SimMqttBackend::scenario(0, messages, SimMqttTerminal::Idle).burst()
        });
        assert_eq!(service.poll(app(), id, 64).unwrap(), MqttPoll::Message);
        // Three of the eleven were dropped-oldest; the queue keeps the freshest 8.
        assert_eq!(service.dropped(app(), id).unwrap(), 3);
        let mut topic = [0u8; 32];
        let mut payload = [0u8; 32];
        // The oldest surviving payload is message index 3.
        let message = service
            .read_message(app(), id, &mut topic, &mut payload)
            .unwrap()
            .unwrap();
        assert_eq!(&payload[..message.payload_len as usize], &[3u8]);
    }

    #[test]
    fn clean_disconnect_transitions_to_disconnected() {
        let (mut service, id) = connect_and_subscribe(|| {
            SimMqttBackend::scenario(0, Vec::new(), SimMqttTerminal::Disconnect)
        });
        assert_eq!(service.poll(app(), id, 64).unwrap(), MqttPoll::Disconnected);
    }

    #[test]
    fn reconnect_reseeds_the_script_for_a_fresh_session() {
        let make = || {
            SimMqttBackend::scenario(
                0,
                vec![ScriptedMessage::new(b"sensors/temp", b"1", false)],
                SimMqttTerminal::Idle,
            )
        };
        let (mut service, id) = connect_and_subscribe(make);
        assert_eq!(service.poll(app(), id, 64).unwrap(), MqttPoll::Message);
        service.disconnect(app(), id).unwrap();
        // A second connect re-seeds the script, so the message is delivered again.
        let id2 = service.connect(app(), &brokers(), 0, 128).unwrap();
        let mut now = 128;
        loop {
            now += 16;
            match service.poll(app(), id2, now).unwrap() {
                MqttPoll::Connected => break,
                MqttPoll::Connecting => continue,
                other => panic!("unexpected {other:?}"),
            }
        }
        service.subscribe(app(), id2, &topics(), 0, now).unwrap();
        assert_eq!(
            service.poll(app(), id2, now + 16).unwrap(),
            MqttPoll::Message
        );
    }

    #[test]
    fn stale_handle_from_a_prior_session_is_rejected() {
        let (mut service, id) = connect_and_subscribe(|| {
            SimMqttBackend::scenario(0, Vec::new(), SimMqttTerminal::Idle)
        });
        service.disconnect(app(), id).unwrap();
        // The old handle no longer names a live session.
        assert_eq!(service.poll(app(), id, 128), Err(MqttError::StaleSession));
    }

    #[test]
    fn refused_connect_needs_no_host_network() {
        let mut service = AppMqttService::new(SimMqttBackend::refused(MqttError::Denied));
        assert_eq!(
            service.connect(app(), &brokers(), 0, 0),
            Err(MqttError::Denied)
        );
        let mut offline = AppMqttService::new(SimMqttBackend::offline());
        assert_eq!(
            offline.connect(app(), &brokers(), 0, 0),
            Err(MqttError::Unavailable)
        );
    }
}
