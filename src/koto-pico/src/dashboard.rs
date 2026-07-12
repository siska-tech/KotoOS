//! Bounded data model and formatting helpers for the physical device probe.

use core::fmt;

pub const KEYBOARD_RAW_CAPACITY: usize = 8;
pub const RP2040_SRAM_BYTES: usize = 264 * 1024;
// Clears the longest diagnostic line: the KOTO-0131 `phase=160 app-frame` perf
// record carries the app id plus a dozen timing/count fields, and KOTO-0143 added
// the `full_reason=` full-repaint attribution — so the line must stay intact for
// the reason to be readable rather than truncated off the tail. Every other line
// (shell dashboard, app heartbeat) is well under this. It is a transient stack
// local, not a doubled `DeviceRuntimeHost` field, so the headroom is cheap.
pub const DASHBOARD_LINE_BUFFER_BYTES: usize = 352;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeStatus {
    Pending,
    Pass,
    Present,
    Absent,
    Error,
}

impl ProbeStatus {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Pass => "pass",
            Self::Present => "present",
            Self::Absent => "absent",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NormalizedKeys {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub action_a: bool,
    pub action_b: bool,
    pub action_x: bool,
    pub action_y: bool,
}

impl NormalizedKeys {
    pub fn compact(self, out: &mut LineBuffer) {
        let mut first = true;
        for (pressed, label) in [
            (self.up, "up"),
            (self.down, "down"),
            (self.left, "left"),
            (self.right, "right"),
            (self.action_a, "A"),
            (self.action_b, "B"),
            (self.action_x, "X"),
            (self.action_y, "Y"),
        ] {
            if pressed {
                if !first {
                    let _ = fmt::Write::write_str(out, "+");
                }
                let _ = fmt::Write::write_str(out, label);
                first = false;
            }
        }
        if first {
            let _ = fmt::Write::write_str(out, "none");
        }
    }
}

/// Best-effort HID-style mapping used only by the diagnostic view. Raw bytes
/// remain the source of truth while the STM32 keyboard protocol is validated.
pub fn normalize_hid_codes(raw: &[u8]) -> NormalizedKeys {
    let has = |code| raw.contains(&code);
    NormalizedKeys {
        up: has(82),
        down: has(81),
        left: has(80),
        right: has(79),
        action_a: has(29), // Z
        action_b: has(27), // X
        action_x: has(4),  // A
        action_y: has(22), // S
    }
}

pub struct LineBuffer {
    bytes: [u8; DASHBOARD_LINE_BUFFER_BYTES],
    len: usize,
}

impl LineBuffer {
    pub const fn new() -> Self {
        Self {
            bytes: [0; DASHBOARD_LINE_BUFFER_BYTES],
            len: 0,
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl fmt::Write for LineBuffer {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let remaining = self.bytes.len().saturating_sub(self.len);
        let take = text.len().min(remaining);
        self.bytes[self.len..self.len + take].copy_from_slice(&text.as_bytes()[..take]);
        self.len += take;
        if take == text.len() {
            Ok(())
        } else {
            Err(fmt::Error)
        }
    }
}

impl Default for LineBuffer {
    fn default() -> Self {
        Self::new()
    }
}
