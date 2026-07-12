//! PicoCalc STM32 keyboard bridge protocol and held-key tracking.

pub const FIFO_REGISTER: u8 = 0x09;
pub const FIFO_CAPACITY: usize = 10;
pub const FRAME_PERIOD_MS: u64 = 16;
pub const STABLE_SAMPLE_COUNT: u8 = 5;

pub const KEY_STATE_IDLE: u8 = 0;
pub const KEY_STATE_PRESSED: u8 = 1;
pub const KEY_STATE_HOLD: u8 = 2;
pub const KEY_STATE_RELEASED: u8 = 3;

pub const KEY_UP: u8 = 0xb5;
pub const KEY_DOWN: u8 = 0xb6;
pub const KEY_LEFT: u8 = 0xb4;
pub const KEY_RIGHT: u8 = 0xb7;

// PicoCalc function keys drive the shell command bar (favorite / sort /
// category), mirroring KotoSim's F2/F3/F4 bindings. These follow the
// ClockworkPi keyboard firmware's 0x81.. function-key block; the product
// firmware also logs raw pressed keycodes so the mapping can be confirmed on
// hardware (KOTO-0123).
pub const KEY_F1: u8 = 0x81;
pub const KEY_F2: u8 = 0x82;
pub const KEY_F3: u8 = 0x83;
pub const KEY_F4: u8 = 0x84;
pub const KEY_F5: u8 = 0x85;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KeyEvent {
    pub state: u8,
    pub key: u8,
}

impl KeyEvent {
    pub const fn from_wire(bytes: [u8; 2]) -> Self {
        Self {
            state: bytes[0],
            key: bytes[1],
        }
    }

    pub const fn is_empty(self) -> bool {
        self.state == KEY_STATE_IDLE && self.key == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeldKeys {
    keys: [u8; FIFO_CAPACITY],
    len: usize,
}

impl HeldKeys {
    pub const fn new() -> Self {
        Self {
            keys: [0; FIFO_CAPACITY],
            len: 0,
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.keys[..self.len]
    }

    pub fn apply(&mut self, event: KeyEvent) -> bool {
        match event.state {
            KEY_STATE_PRESSED | KEY_STATE_HOLD => self.insert(event.key),
            KEY_STATE_RELEASED => self.remove(event.key),
            _ => false,
        }
    }

    fn insert(&mut self, key: u8) -> bool {
        if key == 0 || self.as_slice().contains(&key) || self.len == self.keys.len() {
            return false;
        }
        self.keys[self.len] = key;
        self.len += 1;
        self.keys[..self.len].sort_unstable();
        true
    }

    fn remove(&mut self, key: u8) -> bool {
        let Some(index) = self
            .as_slice()
            .iter()
            .position(|candidate| *candidate == key)
        else {
            return false;
        };
        self.keys.copy_within(index + 1..self.len, index);
        self.len -= 1;
        self.keys[self.len] = 0;
        true
    }
}

impl Default for HeldKeys {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
pub struct Candidate {
    pub name: &'static str,
    pub bindings: [(u8, &'static str); 8],
}

impl Candidate {
    pub fn detected<'a>(&'a self, held: &'a HeldKeys) -> impl Iterator<Item = &'static str> + 'a {
        self.bindings
            .iter()
            .filter(move |(key, _)| held.as_slice().contains(key))
            .map(|(_, label)| *label)
    }
}

pub const CANDIDATES: [Candidate; 3] = [
    Candidate {
        name: "arrow-zxas",
        bindings: [
            (KEY_UP, "up"),
            (KEY_DOWN, "down"),
            (KEY_LEFT, "left"),
            (KEY_RIGHT, "right"),
            (b'z', "action_a"),
            (b'x', "action_b"),
            (b'a', "action_x"),
            (b's', "action_y"),
        ],
    },
    Candidate {
        name: "wasd-jkui",
        bindings: [
            (b'w', "up"),
            (b's', "down"),
            (b'a', "left"),
            (b'd', "right"),
            (b'j', "action_a"),
            (b'k', "action_b"),
            (b'u', "action_x"),
            (b'i', "action_y"),
        ],
    },
    Candidate {
        name: "ijkl-zxas",
        bindings: [
            (b'i', "up"),
            (b'k', "down"),
            (b'j', "left"),
            (b'l', "right"),
            (b'z', "action_a"),
            (b'x', "action_b"),
            (b'a', "action_x"),
            (b's', "action_y"),
        ],
    },
];

pub const fn key_name(key: u8) -> Option<&'static str> {
    match key {
        KEY_UP => Some("ArrowUp"),
        KEY_DOWN => Some("ArrowDown"),
        KEY_LEFT => Some("ArrowLeft"),
        KEY_RIGHT => Some("ArrowRight"),
        0x08 => Some("Backspace"),
        0x09 => Some("Tab"),
        0x0a => Some("Enter"),
        0xa1 => Some("Alt"),
        0xa2 => Some("ShiftLeft"),
        0xa3 => Some("ShiftRight"),
        0xa4 => Some("Sym"),
        0xa5 => Some("Control"),
        0xb1 => Some("Escape"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_keys_follow_press_hold_release_events() {
        let mut held = HeldKeys::new();
        assert!(held.apply(KeyEvent {
            state: KEY_STATE_PRESSED,
            key: b'z',
        }));
        assert!(!held.apply(KeyEvent {
            state: KEY_STATE_HOLD,
            key: b'z',
        }));
        assert_eq!(held.as_slice(), &[b'z']);
        assert!(held.apply(KeyEvent {
            state: KEY_STATE_RELEASED,
            key: b'z',
        }));
        assert!(held.as_slice().is_empty());
    }

    #[test]
    fn candidate_mapping_uses_bridge_key_codes() {
        let mut held = HeldKeys::new();
        held.apply(KeyEvent {
            state: KEY_STATE_PRESSED,
            key: KEY_UP,
        });
        held.apply(KeyEvent {
            state: KEY_STATE_PRESSED,
            key: b'z',
        });
        let detected: std::vec::Vec<_> = CANDIDATES[0].detected(&held).collect();
        assert_eq!(detected, ["up", "action_a"]);
    }
}
