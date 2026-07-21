//! Bounded host-owned state for the retained KotoUI application ABI.

use core::str;
use koto_ui::{
    Button, Checkbox, DamageRects, DamageSet, FocusEntry, FocusManager, FocusScopeId, GlyphRun,
    ImeComposition, Label, List, ListModel, ListRow, Navigation, PaintError, Painter, Panel,
    ResponseKind, Rgb565, TextAlign, TextField, TextMetrics, TextRun, Theme, UiAction, UiContext,
    UiEvent, UiRect, Utf8Buffer, WidgetId,
};

use crate::{ConfigSnapshot, VmInputSnapshot};

pub const UI_MAX_MOUNT_BYTES: usize = 4096;
pub const UI_MAX_NODES: usize = 32;
pub const UI_DATA_CAPACITY: usize = 2048;
pub const UI_MAX_UPDATE_BYTES: usize = 2048;
pub const UI_MOUNT_HEADER_SIZE: usize = HEADER;
pub const UI_NODE_RECORD_SIZE: usize = STRIDE;
pub const UI_UPDATE_HEADER_SIZE: usize = UPDATE_HEADER;
pub const UI_UPDATE_RECORD_SIZE: usize = UPDATE_STRIDE;
pub const UI_MAX_UPDATE_RECORDS: usize = UI_MAX_UPDATES;
pub const UI_EVENT_QUEUE_CAPACITY: usize = 8;
pub const UI_EVENT_HEADER_SIZE: usize = 32;
pub const UI_MAX_TEXT_FIELDS: usize = 4;
pub const UI_MAX_TEXT_FIELD_BYTES: usize = 256;
pub const UI_MAX_LIST_ROWS: usize = 32;
pub const UI_DAMAGE_CAPACITY: usize = 8;
pub const UI_MAX_OPEN_MODALS: usize = 1;
const UI_EVENT_TEXT_CAPACITY: usize = 256;
const UI_IME_TEXT_CAPACITY: usize = 128;
const UI_IME_CANDIDATE_CAPACITY: usize = 64;

const HEADER: usize = 40;
const STRIDE: usize = 48;
const UPDATE_HEADER: usize = 32;
const UPDATE_STRIDE: usize = 32;
const UI_MAX_UPDATES: usize = 16;
const NONE_ID: u16 = 0xffff;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiMountError {
    BadArgument,
    Unsupported,
    NoMemory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UiPollError {
    NotMounted,
    NoMemory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiImeTarget<'a> {
    pub widget_id: u16,
    pub value: &'a str,
    pub cursor: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct UiSemanticEvent {
    kind: u8,
    widget_id: u16,
    value: i32,
    aux: i32,
    text_len: u16,
    text: [u8; UI_EVENT_TEXT_CAPACITY],
}

impl UiSemanticEvent {
    const EMPTY: Self = Self {
        kind: 0,
        widget_id: 0,
        value: 0,
        aux: 0,
        text_len: 0,
        text: [0; UI_EVENT_TEXT_CAPACITY],
    };

    fn new(kind: u8, widget_id: u16, value: i32, aux: i32, text: &[u8]) -> Self {
        let mut event = Self::EMPTY;
        event.kind = kind;
        event.widget_id = widget_id;
        event.value = value;
        event.aux = aux;
        let len = text.len().min(UI_EVENT_TEXT_CAPACITY);
        event.text[..len].copy_from_slice(&text[..len]);
        event.text_len = len as u16;
        event
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiNode {
    pub id: u16,
    pub parent_id: u16,
    pub kind: u8,
    pub flags: u8,
    pub state_flags: u16,
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
    text_start: u16,
    text_len: u16,
    text_capacity: u16,
    value_start: u16,
    value_len: u16,
    value_capacity: u16,
    pub args: [i32; 4],
}

impl UiNode {
    const EMPTY: Self = Self {
        id: 0,
        parent_id: NONE_ID,
        kind: 0,
        flags: 0,
        state_flags: 0,
        x: 0,
        y: 0,
        width: 0,
        height: 0,
        text_start: 0,
        text_len: 0,
        text_capacity: 0,
        value_start: 0,
        value_len: 0,
        value_capacity: 0,
        args: [0; 4],
    };

    pub const fn visible(self) -> bool {
        self.flags & 1 != 0
    }

    pub const fn enabled(self) -> bool {
        self.flags & 2 != 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiSession {
    nodes: [UiNode; UI_MAX_NODES],
    node_count: u8,
    data: [u8; UI_DATA_CAPACITY],
    data_used: u16,
    root_id: u16,
    initial_focus_id: u16,
    focused_id: u16,
    editing_id: u16,
    damage: DamageSet<8>,
    events: [UiSemanticEvent; UI_EVENT_QUEUE_CAPACITY],
    event_count: u8,
    dropped_events: i32,
    locale_generation: u32,
    ime_owner_id: u16,
    ime_text: [u8; UI_IME_TEXT_CAPACITY],
    ime_text_len: u8,
    ime_candidate: [u8; UI_IME_CANDIDATE_CAPACITY],
    ime_candidate_len: u8,
}

pub const UI_SESSION_SRAM_BYTES: usize = core::mem::size_of::<UiSession>();
const _: [(); 1] = [(); (UI_SESSION_SRAM_BYTES <= 8192) as usize];

impl Default for UiSession {
    fn default() -> Self {
        Self::new()
    }
}

impl UiSession {
    pub const fn new() -> Self {
        Self {
            nodes: [UiNode::EMPTY; UI_MAX_NODES],
            node_count: 0,
            data: [0; UI_DATA_CAPACITY],
            data_used: 0,
            root_id: NONE_ID,
            initial_focus_id: NONE_ID,
            focused_id: NONE_ID,
            editing_id: NONE_ID,
            damage: DamageSet::new(UiRect::new(0, 0, 320, 320)),
            events: [UiSemanticEvent::EMPTY; UI_EVENT_QUEUE_CAPACITY],
            event_count: 0,
            dropped_events: 0,
            locale_generation: 0,
            ime_owner_id: NONE_ID,
            ime_text: [0; UI_IME_TEXT_CAPACITY],
            ime_text_len: 0,
            ime_candidate: [0; UI_IME_CANDIDATE_CAPACITY],
            ime_candidate_len: 0,
        }
    }

    pub const fn is_mounted(&self) -> bool {
        self.node_count != 0
    }

    pub fn nodes(&self) -> &[UiNode] {
        &self.nodes[..usize::from(self.node_count)]
    }

    pub const fn root_id(&self) -> Option<u16> {
        if self.root_id == NONE_ID {
            None
        } else {
            Some(self.root_id)
        }
    }

    pub const fn initial_focus_id(&self) -> Option<u16> {
        if self.initial_focus_id == NONE_ID {
            None
        } else {
            Some(self.initial_focus_id)
        }
    }

    pub const fn focused_id(&self) -> Option<u16> {
        if self.focused_id == NONE_ID {
            None
        } else {
            Some(self.focused_id)
        }
    }

    pub const fn editing_id(&self) -> Option<u16> {
        if self.editing_id == NONE_ID {
            None
        } else {
            Some(self.editing_id)
        }
    }

    pub fn damaged_rects(&self) -> DamageRects<'_, 8> {
        self.damage.iter()
    }

    pub fn clear_damage(&mut self) {
        self.damage.clear();
    }

    /// Replays the retained tree once for every pending damage clip. Damage is
    /// retained until [`Self::complete_present`] so a backend can roll back a
    /// partially scheduled command list on error.
    pub fn paint_pending(
        &self,
        painter: &mut impl Painter,
        theme: &Theme,
    ) -> Result<usize, PaintError> {
        let mut painted = 0usize;
        for clip in self.damage.iter() {
            self.paint_clip(painter, theme, clip)?;
            painted += 1;
        }
        Ok(painted)
    }

    /// Builds one stable full-scene command image only when damage exists.
    /// Retained backends use this for deterministic old/new command diffing.
    pub fn paint_full_if_damaged(
        &self,
        painter: &mut impl Painter,
        theme: &Theme,
    ) -> Result<bool, PaintError> {
        if self.damage.is_empty() {
            return Ok(false);
        }
        self.paint_clip(painter, theme, UiRect::new(0, 0, 320, 320))?;
        Ok(true)
    }

    pub fn complete_present(&mut self) {
        self.damage.clear();
    }

    pub fn text(&self, node: &UiNode) -> &str {
        let start = usize::from(node.text_start);
        str::from_utf8(&self.data[start..start + usize::from(node.text_len)]).unwrap_or("")
    }

    pub fn value(&self, node: &UiNode) -> &[u8] {
        let start = usize::from(node.value_start);
        &self.data[start..start + usize::from(node.value_len)]
    }

    pub fn reset(&mut self) {
        self.nodes.fill(UiNode::EMPTY);
        self.data.fill(0);
        self.node_count = 0;
        self.data_used = 0;
        self.root_id = NONE_ID;
        self.initial_focus_id = NONE_ID;
        self.focused_id = NONE_ID;
        self.editing_id = NONE_ID;
        self.damage.clear();
        self.events.fill(UiSemanticEvent::EMPTY);
        self.event_count = 0;
        self.dropped_events = 0;
        self.locale_generation = 0;
        self.clear_ime_composition();
    }

    /// Returns the editing TextField that opted into host IME state.
    pub fn ime_target(&self) -> Option<UiImeTarget<'_>> {
        let index = self.focused_index()?;
        let node = self.nodes[index];
        if node.kind != 5
            || node.id != self.editing_id
            || node.state_flags & 1 == 0
            || !self.node_eligible(index)
        {
            return None;
        }
        Some(UiImeTarget {
            widget_id: node.id,
            value: str::from_utf8(self.value(&node)).ok()?,
            cursor: self.text_cursor(index),
        })
    }

    /// Copies one host-IME result back into session-owned storage. No host
    /// pointer survives this call; text changes become ordinary KUE1 events.
    pub fn apply_ime_snapshot(
        &mut self,
        widget_id: u16,
        value: &str,
        cursor: usize,
        composition: &str,
        candidate: Option<&str>,
    ) -> Result<bool, UiMountError> {
        let index = self
            .nodes()
            .iter()
            .position(|node| node.id == widget_id && node.kind == 5)
            .ok_or(UiMountError::BadArgument)?;
        if self.focused_id != widget_id || self.nodes[index].state_flags & 1 == 0 {
            return Err(UiMountError::BadArgument);
        }
        if value.len() > usize::from(self.nodes[index].value_capacity)
            || composition.len() > UI_IME_TEXT_CAPACITY
            || candidate.is_some_and(|text| text.len() > UI_IME_CANDIDATE_CAPACITY)
        {
            self.enqueue(UiSemanticEvent::new(9, widget_id, 1, 0, &[]));
            return Err(UiMountError::NoMemory);
        }
        if cursor > value.len() || !value.is_char_boundary(cursor) {
            return Err(UiMountError::BadArgument);
        }
        let value_changed = self.value(&self.nodes[index]) != value.as_bytes();
        let composition_changed = self.ime_owner_id != widget_id
            || self.ime_text() != composition
            || self.ime_candidate() != candidate;
        if value_changed {
            self.copy_node_value(index, value.as_bytes());
        }
        self.nodes[index].args[0] = cursor as i32;
        self.ime_owner_id = widget_id;
        copy_fixed(
            &mut self.ime_text,
            &mut self.ime_text_len,
            composition.as_bytes(),
        );
        copy_fixed(
            &mut self.ime_candidate,
            &mut self.ime_candidate_len,
            candidate.unwrap_or("").as_bytes(),
        );
        if value_changed || composition_changed {
            self.damage.push(node_bounds(self.nodes[index]));
        }
        if value_changed {
            self.queue_text_event(index, 3);
        }
        Ok(value_changed || composition_changed)
    }

    pub fn clear_ime_composition(&mut self) {
        if let Some(index) = self
            .nodes()
            .iter()
            .position(|node| node.id == self.ime_owner_id)
        {
            if self.ime_text_len != 0 || self.ime_candidate_len != 0 {
                self.damage.push(node_bounds(self.nodes[index]));
            }
        }
        self.ime_owner_id = NONE_ID;
        self.ime_text.fill(0);
        self.ime_text_len = 0;
        self.ime_candidate.fill(0);
        self.ime_candidate_len = 0;
    }

    fn ime_text(&self) -> &str {
        str::from_utf8(&self.ime_text[..usize::from(self.ime_text_len)]).unwrap_or("")
    }

    fn ime_candidate(&self) -> Option<&str> {
        (self.ime_candidate_len != 0).then(|| {
            str::from_utf8(&self.ime_candidate[..usize::from(self.ime_candidate_len)]).unwrap_or("")
        })
    }

    /// Dispatch one frame's normalized input before app bytecode resumes.
    pub fn frame_begin(&mut self, input: VmInputSnapshot, config: ConfigSnapshot) {
        if !self.is_mounted() {
            return;
        }
        self.observe_locale(config);

        let pressed = input.pressed_bits;
        let intents = input.intent_bits;
        let actions = [
            (
                pressed & (1 << 0) != 0 || intents & crate::runtime::text_intent::UP != 0,
                UiAction::Navigate(Navigation::Up),
            ),
            (
                pressed & (1 << 1) != 0 || intents & crate::runtime::text_intent::DOWN != 0,
                UiAction::Navigate(Navigation::Down),
            ),
            (
                pressed & (1 << 2) != 0 || intents & crate::runtime::text_intent::LEFT != 0,
                UiAction::Navigate(Navigation::Left),
            ),
            (
                pressed & (1 << 3) != 0 || intents & crate::runtime::text_intent::RIGHT != 0,
                UiAction::Navigate(Navigation::Right),
            ),
            (
                intents & crate::runtime::text_intent::CONVERT != 0,
                UiAction::Navigate(Navigation::Next),
            ),
            (pressed & (1 << 4) != 0, UiAction::Activate),
            (
                pressed & (1 << 5) != 0 || intents & crate::runtime::text_intent::CANCEL != 0,
                UiAction::Cancel,
            ),
            (
                intents & crate::runtime::text_intent::BACKSPACE != 0,
                UiAction::Backspace,
            ),
            (
                intents & crate::runtime::text_intent::DELETE != 0,
                UiAction::Delete,
            ),
            (
                intents & crate::runtime::text_intent::HOME != 0,
                UiAction::Home,
            ),
            (
                intents & crate::runtime::text_intent::END != 0,
                UiAction::End,
            ),
            (
                intents & crate::runtime::text_intent::NEWLINE != 0,
                UiAction::Submit,
            ),
        ];
        let mut entered_editing = false;
        for (active, action) in actions {
            if active {
                if action == UiAction::Submit && entered_editing {
                    continue;
                }
                let enters_editing = action == UiAction::Activate
                    && self.editing_id != self.focused_id
                    && self.focused_index().is_some_and(|index| {
                        self.node_eligible(index) && self.nodes[index].kind == 5
                    });
                self.dispatch_event(UiEvent::pressed(action));
                entered_editing |= enters_editing && self.editing_id == self.focused_id;
            }
        }
        if let Some(character) = char::from_u32(input.text_codepoint).filter(|c| *c != '\0') {
            let space_activates = character == ' '
                && pressed & (1 << 4) == 0
                && self
                    .focused_index()
                    .is_some_and(|index| matches!(self.nodes[index].kind, 2..=4));
            if space_activates {
                self.dispatch_event(UiEvent::pressed(UiAction::Activate));
            } else {
                self.dispatch_event(UiEvent::pressed(UiAction::Text(character)));
            }
        }
    }

    /// Encode and dequeue one KUE1 response. A short destination is untouched.
    pub fn poll_event(&mut self, dst: &mut [u8]) -> Result<usize, UiPollError> {
        if !self.is_mounted() {
            return Err(UiPollError::NotMounted);
        }
        let event = if self.event_count != 0 {
            self.events[0]
        } else if self.dropped_events != 0 {
            UiSemanticEvent::new(9, 0, 0, self.dropped_events, &[])
        } else {
            return Ok(0);
        };
        let total = UI_EVENT_HEADER_SIZE + usize::from(event.text_len);
        if dst.len() < total {
            return Err(UiPollError::NoMemory);
        }
        dst[..total].fill(0);
        dst[..4].copy_from_slice(b"KUE1");
        put_u16(dst, 4, 1);
        put_u16(dst, 6, 0);
        put_u32(dst, 8, total as u32);
        dst[12] = event.kind;
        put_u16(dst, 14, event.widget_id);
        put_i32(dst, 16, event.value);
        put_i32(dst, 20, event.aux);
        if event.text_len != 0 {
            put_u32(dst, 24, UI_EVENT_HEADER_SIZE as u32);
            put_u16(dst, 28, event.text_len);
            dst[UI_EVENT_HEADER_SIZE..total]
                .copy_from_slice(&event.text[..usize::from(event.text_len)]);
        }
        if self.event_count != 0 {
            let count = usize::from(self.event_count);
            self.events.copy_within(1..count, 0);
            self.event_count -= 1;
            self.events[usize::from(self.event_count)] = UiSemanticEvent::EMPTY;
        } else {
            self.dropped_events = 0;
        }
        Ok(total)
    }

    /// Validates the complete packet before changing live state, then copies all
    /// source text/value bytes into host-owned fixed-capacity slots.
    pub fn mount(&mut self, packet: &[u8]) -> Result<(), UiMountError> {
        let meta = validate(packet)?;

        self.reset();
        let mut arena = 0usize;
        for index in 0..meta.count {
            let r = record(packet, index);
            let text_len = u16_at(r, 20);
            let text_cap = u16_at(r, 22);
            let value_len = u16_at(r, 28);
            let value_cap = u16_at(r, 30);
            let text_start = arena;
            copy_source(
                packet,
                meta.data_offset,
                u32_at(r, 16),
                text_len,
                &mut self.data,
                arena,
            );
            arena += usize::from(text_cap);
            let value_start = arena;
            copy_source(
                packet,
                meta.data_offset,
                u32_at(r, 24),
                value_len,
                &mut self.data,
                arena,
            );
            arena += usize::from(value_cap);
            self.nodes[index] = UiNode {
                id: u16_at(r, 0),
                parent_id: u16_at(r, 2),
                kind: r[4],
                flags: r[5],
                state_flags: u16_at(r, 6),
                x: i16_at(r, 8),
                y: i16_at(r, 10),
                width: u16_at(r, 12),
                height: u16_at(r, 14),
                text_start: text_start as u16,
                text_len,
                text_capacity: text_cap,
                value_start: value_start as u16,
                value_len,
                value_capacity: value_cap,
                args: [i32_at(r, 32), i32_at(r, 36), i32_at(r, 40), i32_at(r, 44)],
            };
        }
        self.node_count = meta.count as u8;
        self.data_used = arena as u16;
        self.root_id = meta.root_id;
        self.initial_focus_id = meta.focus_id;
        self.focused_id = meta.focus_id;
        self.editing_id = NONE_ID;
        self.damage.push(UiRect::new(0, 0, 320, 320));
        Ok(())
    }

    /// Atomically applies a KUP1 property packet to the mounted session.
    pub fn update(&mut self, packet: &[u8]) -> Result<(), UiMountError> {
        if !self.is_mounted() {
            return Err(UiMountError::BadArgument);
        }
        let meta = validate_update(self, packet)?;
        for record_index in 0..meta.count {
            let r = update_record(packet, record_index);
            let node_index = self
                .nodes()
                .iter()
                .position(|node| node.id == u16_at(r, 0))
                .expect("validated update widget");
            let property = r[2];
            let old_bounds = node_bounds(self.nodes[node_index]);
            let data_len = usize::from(u16_at(r, 8));
            let src = &packet[meta.data_offset + u32_at(r, 4) as usize
                ..meta.data_offset + u32_at(r, 4) as usize + data_len];
            let args = [i32_at(r, 12), i32_at(r, 16), i32_at(r, 20), i32_at(r, 24)];
            let changed = match property {
                1 => self.text(&self.nodes[node_index]).as_bytes() != src,
                2 => i32::from(self.nodes[node_index].enabled()) != args[0],
                3 => i32::from(self.nodes[node_index].visible()) != args[0],
                4 | 9 => i32::from(self.nodes[node_index].state_flags & 1) != args[0],
                5 => self.nodes[node_index].args[1] != args[0],
                6 => {
                    self.value(&self.nodes[node_index]) != src
                        || self.nodes[node_index].args[0] != args[0]
                }
                7 => old_bounds != UiRect::new(args[0], args[1], args[2], args[3]),
                8 => {
                    self.value(&self.nodes[node_index]) != src
                        || self.nodes[node_index].args[0] != args[0]
                        || self.nodes[node_index].args[1] != args[1]
                }
                10 => self.focused_id != self.nodes[node_index].id,
                _ => unreachable!("validated update property"),
            };
            match property {
                1 => self.copy_node_text(node_index, src),
                2 => {
                    self.nodes[node_index].flags =
                        (self.nodes[node_index].flags & !2) | ((args[0] as u8) << 1)
                }
                3 => {
                    self.nodes[node_index].flags =
                        (self.nodes[node_index].flags & !1) | args[0] as u8
                }
                4 => {
                    self.nodes[node_index].state_flags =
                        (self.nodes[node_index].state_flags & !1) | args[0] as u16
                }
                5 => self.nodes[node_index].args[1] = args[0],
                6 => {
                    self.copy_node_value(node_index, src);
                    self.nodes[node_index].args[0] = args[0];
                }
                7 => {
                    self.nodes[node_index].x = args[0] as i16;
                    self.nodes[node_index].y = args[1] as i16;
                    self.nodes[node_index].width = args[2] as u16;
                    self.nodes[node_index].height = args[3] as u16;
                }
                8 => {
                    self.copy_node_value(node_index, src);
                    self.nodes[node_index].args[0] = args[0];
                    self.nodes[node_index].args[1] = args[1];
                }
                9 => {
                    self.nodes[node_index].state_flags =
                        (self.nodes[node_index].state_flags & !1) | args[0] as u16
                }
                10 => {
                    let prior = self.focused_id;
                    if prior != self.nodes[node_index].id {
                        self.end_text_editing();
                        self.clear_ime_composition();
                    }
                    self.focused_id = self.nodes[node_index].id;
                    if changed {
                        if let Some(prior_index) =
                            self.nodes().iter().position(|node| node.id == prior)
                        {
                            self.damage.push(node_bounds(self.nodes[prior_index]));
                        }
                    }
                }
                _ => unreachable!("validated update property"),
            }
            let new_bounds = node_bounds(self.nodes[node_index]);
            if changed {
                if property == 7 {
                    self.damage.push_transition(old_bounds, new_bounds);
                } else {
                    self.damage.push(new_bounds);
                }
            }
        }
        Ok(())
    }

    fn copy_node_text(&mut self, index: usize, src: &[u8]) {
        let start = usize::from(self.nodes[index].text_start);
        let capacity = usize::from(self.nodes[index].text_capacity);
        self.data[start..start + capacity].fill(0);
        self.data[start..start + src.len()].copy_from_slice(src);
        self.nodes[index].text_len = src.len() as u16;
    }

    fn copy_node_value(&mut self, index: usize, src: &[u8]) {
        let start = usize::from(self.nodes[index].value_start);
        let capacity = usize::from(self.nodes[index].value_capacity);
        self.data[start..start + capacity].fill(0);
        self.data[start..start + src.len()].copy_from_slice(src);
        self.nodes[index].value_len = src.len() as u16;
    }

    fn observe_locale(&mut self, config: ConfigSnapshot) {
        if self.locale_generation == 0 {
            self.locale_generation = config.locale_generation;
            return;
        }
        if self.locale_generation == config.locale_generation {
            return;
        }
        self.locale_generation = config.locale_generation;
        let event = UiSemanticEvent::new(
            10,
            0,
            config.locale_generation as i32,
            0,
            config.locale.tag().as_bytes(),
        );
        if let Some(index) = self.events[..usize::from(self.event_count)]
            .iter()
            .position(|queued| queued.kind == 10)
        {
            self.events[index] = event;
        } else {
            self.enqueue(event);
        }
    }

    fn enqueue(&mut self, event: UiSemanticEvent) {
        let count = usize::from(self.event_count);
        if count == UI_EVENT_QUEUE_CAPACITY {
            self.dropped_events = self.dropped_events.saturating_add(1);
            return;
        }
        self.events[count] = event;
        self.event_count += 1;
    }

    fn dispatch_event(&mut self, event: UiEvent) {
        if let UiAction::Navigate(direction) = event.action {
            if self.editing_id == self.focused_id
                && self
                    .focused_index()
                    .is_some_and(|index| self.nodes[index].kind == 5)
                && matches!(direction, Navigation::Left | Navigation::Right)
            {
                let _ = self.dispatch_control_event(event);
                return;
            }
            // A focused List owns vertical navigation until its selection can
            // no longer move. Only the following key press at an edge leaves
            // the control through spatial focus navigation.
            let list_selection = self.focused_index().and_then(|index| {
                (self.node_eligible(index)
                    && self.nodes[index].kind == 4
                    && matches!(direction, Navigation::Up | Navigation::Down))
                .then_some((index, self.nodes[index].args[1]))
            });
            if let Some((index, prior_selection)) = list_selection {
                let _ = self.dispatch_control_event(event);
                if self.nodes[index].args[1] != prior_selection {
                    return;
                }
            }
            if self.move_focus(direction) {
                return;
            }
            if list_selection.is_none() && self.dispatch_control_event(event) {
                return;
            }
            return;
        }
        if event.action == UiAction::Activate
            && self
                .focused_index()
                .is_some_and(|index| self.nodes[index].kind == 5)
        {
            let _ = self.begin_text_editing();
            return;
        }
        if event.action == UiAction::Cancel && self.editing_id != NONE_ID {
            self.end_text_editing();
            return;
        }
        if !self.dispatch_control_event(event) && event.action == UiAction::Cancel {
            if let Some(index) = self
                .focused_index()
                .filter(|index| self.node_eligible(*index))
            {
                self.enqueue(UiSemanticEvent::new(7, self.nodes[index].id, 0, 0, &[]));
            }
        }
    }

    fn dispatch_control_event(&mut self, event: UiEvent) -> bool {
        let Some(index) = self.focused_index() else {
            return false;
        };
        if !self.node_eligible(index) {
            return false;
        }
        match self.nodes[index].kind {
            2 => self.dispatch_button(index, event),
            3 => self.dispatch_checkbox(index, event),
            4 => self.dispatch_list(index, event),
            5 => self.dispatch_text_field(index, event),
            _ => false,
        }
    }

    fn dispatch_button(&mut self, index: usize, event: UiEvent) -> bool {
        if event.action != UiAction::Activate {
            return false;
        }
        let node = self.nodes[index];
        let mut context = UiContext::<8>::new(UiRect::new(0, 0, 320, 320), Theme::DARK);
        let mut button = Button::new(WidgetId::new(node.id), node_bounds(node), self.text(&node));
        button.set_enabled(node.enabled(), &mut context);
        button.set_focused(true, &mut context);
        context.clear_damage();
        let response = button.handle_event(event, &mut context);
        // VmInputSnapshot carries an activation pulse rather than persistent
        // press/release state. Keep the KotoUI semantic response, but do not
        // schedule a transient pressed repaint that cannot survive this call.
        if response.is_some_and(|response| response.kind == ResponseKind::Activated) {
            self.enqueue(UiSemanticEvent::new(1, node.id, 0, 0, &[]));
        }
        true
    }

    fn dispatch_checkbox(&mut self, index: usize, event: UiEvent) -> bool {
        if event.action != UiAction::Activate {
            return false;
        }
        let node = self.nodes[index];
        let mut context = UiContext::<8>::new(UiRect::new(0, 0, 320, 320), Theme::DARK);
        let mut checkbox =
            Checkbox::new(WidgetId::new(node.id), node_bounds(node), self.text(&node));
        checkbox.set_enabled(node.enabled(), &mut context);
        checkbox.set_checked(node.state_flags & 1 != 0, &mut context);
        checkbox.set_focused(true, &mut context);
        context.clear_damage();
        let response = checkbox.handle_event(event, &mut context);
        self.absorb_context_damage(&context);
        if let Some(value) = response.and_then(|response| match response.kind {
            ResponseKind::ValueChanged(value) => Some(value),
            _ => None,
        }) {
            self.nodes[index].state_flags =
                (self.nodes[index].state_flags & !1) | (value as u16 & 1);
            self.enqueue(UiSemanticEvent::new(2, node.id, value, 0, &[]));
        }
        true
    }

    fn dispatch_list(&mut self, index: usize, event: UiEvent) -> bool {
        let consumed = matches!(
            event.action,
            UiAction::Navigate(Navigation::Up | Navigation::Down)
                | UiAction::Home
                | UiAction::End
                | UiAction::Activate
        );
        if !consumed {
            return false;
        }
        let node = self.nodes[index];
        let (response, selected, context) = {
            let model = AbiListModel {
                blob: self.value(&node),
                rows: node.args[0] as usize,
            };
            let mut context = UiContext::<8>::new(UiRect::new(0, 0, 320, 320), Theme::DARK);
            let mut list = List::new(WidgetId::new(node.id), node_bounds(node), node.args[2]);
            list.set_enabled(node.enabled(), &mut context);
            let _ = list.sync_model(&model, &mut context);
            if node.args[1] >= 0 {
                let _ = list.set_selected(node.args[1] as usize, &model, &mut context);
            }
            list.set_focused(true, &mut context);
            context.clear_damage();
            let response = list.handle_event(&model, event, &mut context);
            (response, list.selected(), context)
        };
        self.absorb_context_damage(&context);
        self.nodes[index].args[1] = selected.map_or(-1, |selected| selected as i32);
        match response.map(|response| response.kind) {
            Some(ResponseKind::SelectionChanged(_)) => self.queue_list_event(index, 4),
            Some(ResponseKind::SelectionActivated(_)) => self.queue_list_event(index, 5),
            _ => {}
        }
        true
    }

    fn dispatch_text_field(&mut self, index: usize, event: UiEvent) -> bool {
        if self.editing_id != self.nodes[index].id {
            return false;
        }
        let consumed = matches!(
            event.action,
            UiAction::Navigate(Navigation::Left | Navigation::Right)
                | UiAction::Text(_)
                | UiAction::Backspace
                | UiAction::Delete
                | UiAction::Home
                | UiAction::End
                | UiAction::Submit
                | UiAction::Cancel
        );
        if !consumed {
            return false;
        }
        let node = self.nodes[index];
        let capacity = usize::from(node.value_capacity);
        let mut storage = [0u8; 256];
        let value = self.value(&node);
        storage[..value.len()].copy_from_slice(value);
        let mut buffer = Utf8Buffer::from_initialized(&mut storage[..capacity], value.len())
            .expect("validated TextField value");
        let mut context = UiContext::<8>::new(UiRect::new(0, 0, 320, 320), Theme::DARK);
        let mut field = TextField::new(WidgetId::new(node.id), node_bounds(node), self.text(&node));
        field.set_enabled(node.enabled(), &mut context);
        field.set_focused(true, &mut context);
        field.set_editing(true, &mut context);
        field
            .set_cursor(&buffer, self.text_cursor(index), &mut context)
            .expect("validated TextField cursor");
        context.clear_damage();
        let response = field.handle_event(&mut buffer, event, &mut context);
        let cursor = field.cursor();
        let len = buffer.len();
        let mut updated = [0u8; 256];
        updated[..len].copy_from_slice(buffer.as_str().as_bytes());
        self.absorb_context_damage(&context);
        self.nodes[index].args[0] = cursor as i32;
        match response.map(|response| response.kind) {
            Some(ResponseKind::TextChanged(_)) => {
                self.copy_node_value(index, &updated[..len]);
                self.queue_text_event(index, 3);
            }
            Some(ResponseKind::Submitted) => self.queue_text_event(index, 6),
            Some(ResponseKind::Cancelled) => {
                self.enqueue(UiSemanticEvent::new(7, node.id, 0, 0, &[]))
            }
            Some(ResponseKind::CapacityRejected) => {
                self.enqueue(UiSemanticEvent::new(9, node.id, 1, 0, &[]))
            }
            _ => {}
        }
        true
    }

    fn absorb_context_damage(&mut self, context: &UiContext<8>) {
        for rect in context.damaged_rects() {
            self.damage.push(rect);
        }
    }

    fn begin_text_editing(&mut self) -> bool {
        let Some(index) = self.focused_index() else {
            return false;
        };
        if self.nodes[index].kind != 5 || !self.node_eligible(index) {
            return false;
        }
        let id = self.nodes[index].id;
        if self.editing_id != id {
            self.editing_id = id;
            self.damage.push(node_bounds(self.nodes[index]));
        }
        true
    }

    fn end_text_editing(&mut self) {
        if self.editing_id == NONE_ID {
            return;
        }
        if let Some(index) = self
            .nodes()
            .iter()
            .position(|node| node.id == self.editing_id)
        {
            self.damage.push(node_bounds(self.nodes[index]));
        }
        self.editing_id = NONE_ID;
        self.clear_ime_composition();
    }

    fn move_focus(&mut self, direction: Navigation) -> bool {
        let prior = self.focused_id;
        let mut manager = FocusManager::<UI_MAX_NODES>::new();
        for (index, node) in self.nodes().iter().enumerate() {
            if !matches!(node.kind, 2..=5) {
                continue;
            }
            let mut entry = FocusEntry::new(
                WidgetId::new(node.id),
                node_bounds(*node),
                self.focus_scope(index),
            );
            entry.enabled = node.enabled();
            entry.visible = self.node_effectively_visible(*node);
            manager
                .register(entry)
                .expect("validated unique focus node");
        }
        if manager.is_empty() {
            return false;
        }
        let mut context = UiContext::<8>::new(UiRect::new(0, 0, 320, 320), Theme::DARK);
        if let Some(dialog) = self
            .nodes()
            .iter()
            .find(|node| node.kind == 7 && node.state_flags & 1 != 0)
        {
            manager
                .open_modal(
                    FocusScopeId::new(dialog.id),
                    (prior != NONE_ID).then_some(WidgetId::new(prior)),
                    &mut context,
                )
                .expect("validated modal focus");
        } else if prior != NONE_ID {
            manager
                .focus(WidgetId::new(prior), &mut context)
                .expect("validated root focus");
        } else {
            let _ = manager.focus_first(&mut context);
        }
        context.clear_damage();
        let _ = match direction {
            Navigation::Next => manager.move_id_next(&mut context),
            Navigation::Previous => manager.move_id_previous(&mut context),
            Navigation::Up | Navigation::Down | Navigation::Left | Navigation::Right => {
                manager.move_spatial(direction, &mut context)
            }
            Navigation::PageUp | Navigation::PageDown => return false,
        };
        let next = manager.focused().map_or(NONE_ID, WidgetId::get);
        if next != prior {
            self.absorb_context_damage(&context);
            self.end_text_editing();
            self.clear_ime_composition();
            self.focused_id = next;
            self.enqueue(UiSemanticEvent::new(8, next, i32::from(prior), 0, &[]));
            true
        } else {
            false
        }
    }

    fn focus_scope(&self, index: usize) -> FocusScopeId {
        self.nodes()
            .iter()
            .find(|dialog| {
                dialog.kind == 7
                    && descends_from_nodes(self.nodes(), self.nodes[index].id, dialog.id)
            })
            .map_or(FocusScopeId::ROOT, |dialog| FocusScopeId::new(dialog.id))
    }

    fn focused_index(&self) -> Option<usize> {
        self.nodes()
            .iter()
            .position(|node| node.id == self.focused_id)
    }

    fn node_eligible(&self, index: usize) -> bool {
        let node = self.nodes[index];
        if !matches!(node.kind, 2..=5) || !node.enabled() || !self.node_effectively_visible(node) {
            return false;
        }
        let open_dialog = self
            .nodes()
            .iter()
            .find(|candidate| candidate.kind == 7 && candidate.state_flags & 1 != 0);
        match open_dialog {
            Some(dialog) => descends_from_nodes(self.nodes(), node.id, dialog.id),
            None => !self.nodes().iter().any(|dialog| {
                dialog.kind == 7 && descends_from_nodes(self.nodes(), node.id, dialog.id)
            }),
        }
    }

    fn queue_list_event(&mut self, index: usize, kind: u8) {
        let selected = self.nodes[index].args[1];
        if selected < 0 {
            return;
        }
        let start = selected as usize * 12;
        let app_value = self
            .value(&self.nodes[index])
            .get(start + 8..start + 12)
            .map_or(0, |bytes| i32_at(bytes, 0));
        self.enqueue(UiSemanticEvent::new(
            kind,
            self.nodes[index].id,
            selected,
            app_value,
            &[],
        ));
    }

    fn text_cursor(&self, index: usize) -> usize {
        if self.nodes[index].args[0] < 0 {
            usize::from(self.nodes[index].value_len)
        } else {
            self.nodes[index].args[0] as usize
        }
    }

    fn queue_text_event(&mut self, index: usize, kind: u8) {
        let node = self.nodes[index];
        let mut text = [0u8; UI_EVENT_TEXT_CAPACITY];
        let value = self.value(&node);
        text[..value.len()].copy_from_slice(value);
        self.enqueue(UiSemanticEvent::new(
            kind,
            node.id,
            value.len() as i32,
            self.text_cursor(index) as i32,
            &text[..value.len()],
        ));
    }

    fn paint_clip(
        &self,
        painter: &mut impl Painter,
        theme: &Theme,
        clip: UiRect,
    ) -> Result<(), PaintError> {
        for node in self.nodes() {
            if !self.node_effectively_visible(*node) {
                continue;
            }
            let id = WidgetId::new(node.id);
            let bounds = node_bounds(*node);
            let text = self.text(node);
            let focused = self.focused_id == node.id;
            let mut context = UiContext::<1>::new(UiRect::new(0, 0, 320, 320), *theme);
            context.clear_damage();
            let mut text_painter = AbiTextPainter {
                inner: painter,
                overflow: (node.flags >> 4) & 3,
            };
            match node.kind {
                1 => {
                    let align = match node.args[0] {
                        1 => TextAlign::Center,
                        2 => TextAlign::End,
                        _ => TextAlign::Start,
                    };
                    Label::new(id, bounds, text).with_alignment(align).paint(
                        &mut text_painter,
                        clip,
                        theme,
                    )?;
                }
                2 => {
                    let mut button = Button::new(id, bounds, text);
                    button.set_enabled(node.enabled(), &mut context);
                    button.set_focused(focused, &mut context);
                    button.paint(&mut text_painter, clip, theme)?;
                }
                3 => {
                    let mut checkbox = Checkbox::new(id, bounds, text)
                        .with_mark_offset(node.args[0] as i16, node.args[1] as i16);
                    checkbox.set_enabled(node.enabled(), &mut context);
                    checkbox.set_checked(node.state_flags & 1 != 0, &mut context);
                    checkbox.set_focused(focused, &mut context);
                    checkbox.paint(&mut text_painter, clip, theme)?;
                }
                4 => {
                    let model = AbiListModel {
                        blob: self.value(node),
                        rows: node.args[0] as usize,
                    };
                    let mut list = List::new(id, bounds, node.args[2]);
                    list.set_enabled(node.enabled(), &mut context);
                    let _ = list.sync_model(&model, &mut context);
                    if node.args[1] >= 0 {
                        let _ = list.set_selected(node.args[1] as usize, &model, &mut context);
                    }
                    list.set_focused(focused, &mut context);
                    list.paint(&model, &mut text_painter, clip, theme)?;
                }
                5 => {
                    let value = self.value(node);
                    let mut storage = [0u8; 256];
                    storage[..value.len()].copy_from_slice(value);
                    let buffer = Utf8Buffer::from_initialized(&mut storage, value.len())
                        .map_err(|_| PaintError::InvalidGeometry)?;
                    let mut field = TextField::new(id, bounds, text);
                    field.set_enabled(node.enabled(), &mut context);
                    field.set_focused(focused, &mut context);
                    field.set_editing(self.editing_id == node.id, &mut context);
                    let cursor = if node.args[0] < 0 {
                        value.len()
                    } else {
                        node.args[0] as usize
                    };
                    field
                        .set_cursor(&buffer, cursor, &mut context)
                        .map_err(|_| PaintError::InvalidGeometry)?;
                    let composition = (self.ime_owner_id == node.id
                        && (self.ime_text_len != 0 || self.ime_candidate_len != 0))
                        .then(|| ImeComposition {
                            text: self.ime_text(),
                            candidate: self.ime_candidate(),
                        });
                    field.paint(&mut text_painter, clip, theme, &buffer, composition)?;
                }
                6 => {
                    let mut panel = Panel::new(bounds)
                        .with_padding(node.args[0] as u8)
                        .with_title_height(node.args[1] as u8);
                    if !text.is_empty() {
                        panel = panel.with_title(text);
                    }
                    panel.paint(&mut text_painter, clip, theme)?;
                }
                7 => {
                    let mut panel = Panel::new(bounds)
                        .with_padding(node.args[0] as u8)
                        .with_title_height(node.args[1] as u8)
                        .with_dimmed_backdrop(UiRect::new(0, 0, 320, 320));
                    if !text.is_empty() {
                        panel = panel.with_title(text);
                    }
                    panel.paint(&mut text_painter, clip, theme)?;
                }
                _ => return Err(PaintError::InvalidGeometry),
            }
        }
        Ok(())
    }

    fn node_effectively_visible(&self, node: UiNode) -> bool {
        if !node.visible() || (node.kind == 7 && node.state_flags & 1 == 0) {
            return false;
        }
        let mut parent = node.parent_id;
        for _ in 0..self.nodes().len() {
            if parent == NONE_ID {
                return true;
            }
            let Some(ancestor) = self.nodes().iter().find(|candidate| candidate.id == parent)
            else {
                return false;
            };
            if !ancestor.visible() || (ancestor.kind == 7 && ancestor.state_flags & 1 == 0) {
                return false;
            }
            parent = ancestor.parent_id;
        }
        false
    }
}

/// Enforces the ABI's single-line overflow policy before commands reach a
/// retained backend. Those command models intentionally do not retain a clip.
struct AbiTextPainter<'a, P> {
    inner: &'a mut P,
    overflow: u8,
}

impl<P: Painter> TextMetrics for AbiTextPainter<'_, P> {
    fn measure_text(&mut self, text: &str) -> Result<i32, PaintError> {
        self.inner.measure_text(text)
    }

    fn supports_glyph(&self, ch: char) -> bool {
        self.inner.supports_glyph(ch)
    }

    fn line_height(&self) -> Option<i32> {
        self.inner.line_height()
    }
}

impl<P: Painter> Painter for AbiTextPainter<'_, P> {
    fn fill_rect(&mut self, clip: UiRect, rect: UiRect, color: Rgb565) -> Result<(), PaintError> {
        self.inner.fill_rect(clip, rect, color)
    }

    fn stroke_rect(
        &mut self,
        clip: UiRect,
        rect: UiRect,
        color: Rgb565,
        width: u8,
    ) -> Result<(), PaintError> {
        self.inner.stroke_rect(clip, rect, color, width)
    }

    fn draw_text(
        &mut self,
        clip: UiRect,
        bounds: UiRect,
        run: TextRun<'_>,
    ) -> Result<(), PaintError> {
        if run.text.is_empty() || bounds.w <= 0 {
            return Ok(());
        }
        let measured = self.inner.measure_text(run.text)?;
        if measured <= bounds.w {
            return self.inner.draw_text(clip, bounds, run);
        }

        if self.overflow == 0 {
            let visible = if run.align == TextAlign::End {
                suffix_fitting(self.inner, run.text, bounds.w)?
            } else {
                prefix_fitting(self.inner, run.text, bounds.w)?
            };
            return self.inner.draw_text(
                clip,
                bounds,
                TextRun {
                    text: visible,
                    ..run
                },
            );
        }

        let marker = ellipsis_marker(self.inner, bounds.w)?;
        let marker_width = self.inner.measure_text(marker)?;
        let prefix = prefix_fitting(self.inner, run.text, bounds.w - marker_width)?;
        let prefix_width = self.inner.measure_text(prefix)?;
        let total_width = prefix_width.saturating_add(marker_width);
        let x = aligned_text_x(bounds, total_width, run.align);
        if !prefix.is_empty() {
            self.inner.draw_text(
                clip,
                UiRect::new(x, bounds.y, prefix_width, bounds.h),
                TextRun {
                    text: prefix,
                    color: run.color,
                    align: TextAlign::Start,
                },
            )?;
        }
        if !marker.is_empty() {
            self.inner.draw_text(
                clip,
                UiRect::new(
                    x.saturating_add(prefix_width),
                    bounds.y,
                    marker_width,
                    bounds.h,
                ),
                TextRun {
                    text: marker,
                    color: run.color,
                    align: TextAlign::Start,
                },
            )?;
        }
        Ok(())
    }

    fn draw_glyphs(
        &mut self,
        clip: UiRect,
        bounds: UiRect,
        run: GlyphRun<'_>,
    ) -> Result<(), PaintError> {
        self.inner.draw_glyphs(clip, bounds, run)
    }

    fn draw_focus_mark(
        &mut self,
        clip: UiRect,
        rect: UiRect,
        color: Rgb565,
        width: u8,
    ) -> Result<(), PaintError> {
        self.inner.draw_focus_mark(clip, rect, color, width)
    }
}

fn prefix_fitting<'a>(
    painter: &mut impl TextMetrics,
    text: &'a str,
    width: i32,
) -> Result<&'a str, PaintError> {
    if width <= 0 {
        return Ok("");
    }
    let mut end = 0usize;
    for (index, ch) in text.char_indices() {
        let next = index + ch.len_utf8();
        if painter.measure_text(&text[..next])? > width {
            break;
        }
        end = next;
    }
    Ok(&text[..end])
}

fn suffix_fitting<'a>(
    painter: &mut impl TextMetrics,
    text: &'a str,
    width: i32,
) -> Result<&'a str, PaintError> {
    if width <= 0 {
        return Ok("");
    }
    let mut start = text.len();
    for (index, _) in text.char_indices().rev() {
        if painter.measure_text(&text[index..])? > width {
            break;
        }
        start = index;
    }
    Ok(&text[start..])
}

fn ellipsis_marker<'a>(painter: &mut impl TextMetrics, width: i32) -> Result<&'a str, PaintError> {
    if painter.supports_glyph('…') && painter.measure_text("…")? <= width {
        return Ok("…");
    }
    for marker in ["...", "..", "."] {
        if painter.measure_text(marker)? <= width {
            return Ok(marker);
        }
    }
    Ok("")
}

fn aligned_text_x(bounds: UiRect, width: i32, align: TextAlign) -> i32 {
    match align {
        TextAlign::Start => bounds.x,
        TextAlign::Center => bounds.x.saturating_add((bounds.w - width).max(0) / 2),
        TextAlign::End => bounds.x.saturating_add((bounds.w - width).max(0)),
    }
}

struct AbiListModel<'a> {
    blob: &'a [u8],
    rows: usize,
}

impl ListModel for AbiListModel<'_> {
    fn len(&self) -> usize {
        self.rows
    }
    fn row(&self, index: usize) -> Option<ListRow<'_>> {
        if index >= self.rows {
            return None;
        }
        let record = self.blob.get(index * 12..index * 12 + 12)?;
        let offset = usize::from(u16_at(record, 0));
        let len = usize::from(u16_at(record, 2));
        let label = str::from_utf8(self.blob.get(offset..offset + len)?).ok()?;
        Some(if u16_at(record, 4) & 1 != 0 {
            ListRow::new(label)
        } else {
            ListRow::disabled(label)
        })
    }
}

#[derive(Clone, Copy)]
struct UpdateMeta {
    count: usize,
    data_offset: usize,
}

fn validate_update(session: &UiSession, packet: &[u8]) -> Result<UpdateMeta, UiMountError> {
    if packet.len() < UPDATE_HEADER || packet.len() > UI_MAX_UPDATE_BYTES || &packet[..4] != b"KUP1"
    {
        return Err(UiMountError::BadArgument);
    }
    if u16_at(packet, 4) != 1 || u16_at(packet, 6) != 0 {
        return Err(UiMountError::Unsupported);
    }
    if usize::try_from(u32_at(packet, 8)).ok() != Some(packet.len())
        || usize::from(u16_at(packet, 14)) != UPDATE_STRIDE
        || u32_at(packet, 16) as usize != UPDATE_HEADER
        || u32_at(packet, 28) != 0
    {
        return Err(UiMountError::BadArgument);
    }
    let count = usize::from(u16_at(packet, 12));
    if count == 0 {
        return Err(UiMountError::BadArgument);
    }
    if count > UI_MAX_UPDATES {
        return Err(UiMountError::NoMemory);
    }
    let records_end = UPDATE_HEADER
        .checked_add(
            count
                .checked_mul(UPDATE_STRIDE)
                .ok_or(UiMountError::BadArgument)?,
        )
        .ok_or(UiMountError::BadArgument)?;
    let data_offset = usize::try_from(u32_at(packet, 20)).map_err(|_| UiMountError::BadArgument)?;
    let data_len = usize::try_from(u32_at(packet, 24)).map_err(|_| UiMountError::BadArgument)?;
    if data_offset < records_end
        || data_offset.checked_add(data_len) != Some(packet.len())
        || packet
            .get(records_end..data_offset)
            .is_none_or(|padding| padding.iter().any(|b| *b != 0))
    {
        return Err(UiMountError::BadArgument);
    }

    let mut focus_requests = 0usize;
    for index in 0..count {
        let r = update_record(packet, index);
        let widget = u16_at(r, 0);
        let property = r[2];
        if !(1..=10).contains(&property)
            || r[3] != 0
            || u16_at(r, 10) != 0
            || u32_at(r, 28) != 0
            || (0..index).any(|prior| {
                let p = update_record(packet, prior);
                u16_at(p, 0) == widget && p[2] == property
            })
        {
            return Err(UiMountError::BadArgument);
        }
        let node_index = session
            .nodes()
            .iter()
            .position(|node| node.id == widget)
            .ok_or(UiMountError::BadArgument)?;
        let node = session.nodes[node_index];
        let len = usize::from(u16_at(r, 8));
        let src = update_source(packet, data_offset, data_len, u32_at(r, 4), len)?;
        let args = [i32_at(r, 12), i32_at(r, 16), i32_at(r, 20), i32_at(r, 24)];
        match property {
            1 => {
                if len > usize::from(node.text_capacity)
                    || str::from_utf8(src).is_err()
                    || args != [0; 4]
                {
                    return Err(UiMountError::BadArgument);
                }
            }
            2 | 3 => {
                if !src.is_empty()
                    || !matches!(args[0], 0 | 1)
                    || args[1..] != [0; 3]
                    || (node.id == session.root_id && args[0] == 0)
                {
                    return Err(UiMountError::BadArgument);
                }
            }
            4 => {
                if node.kind != 3
                    || !src.is_empty()
                    || !matches!(args[0], 0 | 1)
                    || args[1..] != [0; 3]
                {
                    return Err(UiMountError::BadArgument);
                }
            }
            5 => {
                if node.kind != 4
                    || !src.is_empty()
                    || args[1..] != [0; 3]
                    || !valid_selection(session.value(&node), node.args[0], args[0])
                {
                    return Err(UiMountError::BadArgument);
                }
            }
            6 => {
                if node.kind != 5 || len > usize::from(node.value_capacity) || args[1..] != [0; 3] {
                    return Err(UiMountError::BadArgument);
                }
                let text = str::from_utf8(src).map_err(|_| UiMountError::BadArgument)?;
                if args[0] < -1 || (args[0] >= 0 && !text.is_char_boundary(args[0] as usize)) {
                    return Err(UiMountError::BadArgument);
                }
            }
            7 => {
                if node.id == session.root_id
                    || !src.is_empty()
                    || i16::try_from(args[0]).is_err()
                    || i16::try_from(args[1]).is_err()
                    || !(1..=32767).contains(&args[2])
                    || !(1..=32767).contains(&args[3])
                {
                    return Err(UiMountError::BadArgument);
                }
            }
            8 => {
                if node.kind != 4
                    || len > usize::from(node.value_capacity)
                    || args[2..] != [0; 2]
                    || validate_row_blob(src, args[0], args[1]).is_err()
                {
                    return Err(UiMountError::BadArgument);
                }
            }
            9 => {
                if node.kind != 7
                    || !src.is_empty()
                    || !matches!(args[0], 0 | 1)
                    || args[1..] != [0; 3]
                {
                    return Err(UiMountError::BadArgument);
                }
            }
            10 => {
                focus_requests += 1;
                if !src.is_empty() || args != [0; 4] || !matches!(node.kind, 2..=5) {
                    return Err(UiMountError::BadArgument);
                }
            }
            _ => unreachable!(),
        }
    }
    if focus_requests > 1 {
        return Err(UiMountError::BadArgument);
    }

    let mut rows = 0usize;
    let mut open_dialogs = 0usize;
    for node in session.nodes() {
        let bounds = effective_bounds(*node, packet, count);
        if node.id != session.root_id {
            let parent = session
                .nodes()
                .iter()
                .find(|parent| parent.id == node.parent_id)
                .ok_or(UiMountError::BadArgument)?;
            if !rect_contained(bounds, effective_bounds(*parent, packet, count)) {
                return Err(UiMountError::BadArgument);
            }
        }
        if node.kind == 4 {
            rows += effective_arg(packet, count, node.id, 8, 0).unwrap_or(node.args[0]) as usize;
        }
        if node.kind == 7 {
            open_dialogs += effective_arg(packet, count, node.id, 9, 0)
                .unwrap_or(i32::from(node.state_flags & 1)) as usize;
        }
    }
    if rows > 32 || open_dialogs > 1 {
        return Err(UiMountError::NoMemory);
    }
    let requested_focus = (0..count).find_map(|i| {
        let r = update_record(packet, i);
        (r[2] == 10).then(|| u16_at(r, 0))
    });
    let final_focus = requested_focus.unwrap_or(session.focused_id);
    if final_focus != NONE_ID {
        let node = session
            .nodes()
            .iter()
            .find(|node| node.id == final_focus)
            .ok_or(UiMountError::BadArgument)?;
        let visible =
            effective_arg(packet, count, final_focus, 3, 0).unwrap_or(i32::from(node.visible()));
        let enabled =
            effective_arg(packet, count, final_focus, 2, 0).unwrap_or(i32::from(node.enabled()));
        let open_dialog = session.nodes().iter().find(|candidate| {
            candidate.kind == 7
                && effective_arg(packet, count, candidate.id, 9, 0)
                    .unwrap_or(i32::from(candidate.state_flags & 1))
                    != 0
        });
        let wrong_scope = if let Some(dialog) = open_dialog {
            !descends_from_nodes(session.nodes(), final_focus, dialog.id)
        } else {
            session.nodes().iter().any(|dialog| {
                dialog.kind == 7 && descends_from_nodes(session.nodes(), final_focus, dialog.id)
            })
        };
        if visible == 0 || enabled == 0 || wrong_scope {
            return Err(UiMountError::BadArgument);
        }
    }
    Ok(UpdateMeta { count, data_offset })
}

fn update_record(packet: &[u8], index: usize) -> &[u8] {
    &packet[UPDATE_HEADER + index * UPDATE_STRIDE..UPDATE_HEADER + (index + 1) * UPDATE_STRIDE]
}
fn update_source(
    packet: &[u8],
    base: usize,
    total: usize,
    offset: u32,
    len: usize,
) -> Result<&[u8], UiMountError> {
    let offset = usize::try_from(offset).map_err(|_| UiMountError::BadArgument)?;
    if offset.checked_add(len).is_none_or(|end| end > total) {
        return Err(UiMountError::BadArgument);
    }
    Ok(&packet[base + offset..base + offset + len])
}
fn effective_arg(
    packet: &[u8],
    count: usize,
    widget: u16,
    property: u8,
    arg: usize,
) -> Option<i32> {
    (0..count).find_map(|index| {
        let r = update_record(packet, index);
        (u16_at(r, 0) == widget && r[2] == property).then(|| i32_at(r, 12 + arg * 4))
    })
}
fn effective_bounds(node: UiNode, packet: &[u8], count: usize) -> UiRect {
    if let Some(x) = effective_arg(packet, count, node.id, 7, 0) {
        UiRect::new(
            x,
            effective_arg(packet, count, node.id, 7, 1).unwrap(),
            effective_arg(packet, count, node.id, 7, 2).unwrap(),
            effective_arg(packet, count, node.id, 7, 3).unwrap(),
        )
    } else {
        node_bounds(node)
    }
}
fn node_bounds(node: UiNode) -> UiRect {
    UiRect::new(
        i32::from(node.x),
        i32::from(node.y),
        i32::from(node.width),
        i32::from(node.height),
    )
}
fn rect_contained(child: UiRect, parent: UiRect) -> bool {
    i64::from(child.x) >= i64::from(parent.x)
        && i64::from(child.y) >= i64::from(parent.y)
        && i64::from(child.x) + i64::from(child.w) <= i64::from(parent.x) + i64::from(parent.w)
        && i64::from(child.y) + i64::from(child.h) <= i64::from(parent.y) + i64::from(parent.h)
}
fn valid_selection(blob: &[u8], rows: i32, selection: i32) -> bool {
    if selection == -1 {
        return true;
    }
    if selection < 0 || selection >= rows {
        return false;
    }
    let start = selection as usize * 12;
    blob.get(start..start + 12)
        .is_some_and(|row| u16_at(row, 4) & 1 != 0)
}
fn validate_row_blob(blob: &[u8], rows: i32, selection: i32) -> Result<(), UiMountError> {
    let rows = usize::try_from(rows).map_err(|_| UiMountError::BadArgument)?;
    if rows.checked_mul(12).is_none_or(|len| len > blob.len())
        || !valid_selection(blob, rows as i32, selection)
    {
        return Err(UiMountError::BadArgument);
    }
    for index in 0..rows {
        let row = &blob[index * 12..index * 12 + 12];
        if usize::from(u16_at(row, 0)) < rows * 12
            || u16_at(row, 4) & !1 != 0
            || u16_at(row, 6) != 0
        {
            return Err(UiMountError::BadArgument);
        }
        str::from_utf8(source(
            blob,
            0,
            blob.len(),
            u32::from(u16_at(row, 0)),
            usize::from(u16_at(row, 2)),
        )?)
        .map_err(|_| UiMountError::BadArgument)?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct MountMeta {
    count: usize,
    data_offset: usize,
    root_id: u16,
    focus_id: u16,
}

fn validate(packet: &[u8]) -> Result<MountMeta, UiMountError> {
    if packet.len() < HEADER || packet.len() > UI_MAX_MOUNT_BYTES || &packet[..4] != b"KUI1" {
        return Err(UiMountError::BadArgument);
    }
    if u16_at(packet, 4) != 1 {
        return Err(UiMountError::Unsupported);
    }
    if u16_at(packet, 6) != 0 {
        return Err(UiMountError::Unsupported);
    }
    if usize::try_from(u32_at(packet, 8)).ok() != Some(packet.len())
        || u16_at(packet, 14) as usize != STRIDE
        || u32_at(packet, 16) as usize != HEADER
        || u32_at(packet, 32) != 0
        || u32_at(packet, 36) != 0
    {
        return Err(UiMountError::BadArgument);
    }
    let count = usize::from(u16_at(packet, 12));
    if count == 0 {
        return Err(UiMountError::BadArgument);
    }
    if count > UI_MAX_NODES {
        return Err(UiMountError::NoMemory);
    }
    let nodes_end = HEADER
        .checked_add(count.checked_mul(STRIDE).ok_or(UiMountError::BadArgument)?)
        .ok_or(UiMountError::BadArgument)?;
    let data_offset = usize::try_from(u32_at(packet, 20)).map_err(|_| UiMountError::BadArgument)?;
    let data_len = usize::try_from(u32_at(packet, 24)).map_err(|_| UiMountError::BadArgument)?;
    if data_offset < nodes_end
        || data_offset.checked_add(data_len) != Some(packet.len())
        || packet
            .get(nodes_end..data_offset)
            .is_none_or(|padding| padding.iter().any(|b| *b != 0))
    {
        return Err(UiMountError::BadArgument);
    }

    let root_id = u16_at(packet, 28);
    let focus_id = u16_at(packet, 30);
    let mut arena = 0usize;
    let mut roots = 0usize;
    let mut text_fields = 0usize;
    let mut text_field_capacity = 0usize;
    let mut list_rows = 0usize;
    let mut dialogs = 0usize;
    let mut open_dialogs = 0usize;
    for index in 0..count {
        let r = record(packet, index);
        let id = u16_at(r, 0);
        let parent = u16_at(r, 2);
        let kind = r[4];
        let flags = r[5];
        let state = u16_at(r, 6);
        if id == 0 || id == NONE_ID || kind == 0 || kind > 7 || flags & 0xc0 != 0 {
            return Err(UiMountError::BadArgument);
        }
        if (flags >> 2) & 3 >= 2 || (flags >> 4) & 3 >= 2 {
            return Err(UiMountError::Unsupported);
        }
        if (0..index).any(|prior| u16_at(record(packet, prior), 0) == id) {
            return Err(UiMountError::BadArgument);
        }
        let allowed_state = matches!(kind, 2 | 3 | 4 | 5 | 7) as u16;
        if state & !allowed_state != 0 {
            return Err(UiMountError::BadArgument);
        }
        let width = u16_at(r, 12);
        let height = u16_at(r, 14);
        if width == 0 || height == 0 || width > 32767 || height > 32767 {
            return Err(UiMountError::BadArgument);
        }
        if parent == NONE_ID {
            roots += 1;
            if id != root_id
                || kind != 6
                || flags & 3 != 3
                || i16_at(r, 8) != 0
                || i16_at(r, 10) != 0
                || width != 320
                || height != 320
            {
                return Err(UiMountError::BadArgument);
            }
        } else {
            let parent_index = (0..index)
                .find(|prior| u16_at(record(packet, *prior), 0) == parent)
                .ok_or(UiMountError::BadArgument)?;
            let pr = record(packet, parent_index);
            if !matches!(pr[4], 6 | 7) || !contained(r, pr) {
                return Err(UiMountError::BadArgument);
            }
            if kind == 7 && u16_at(pr, 0) != root_id {
                return Err(UiMountError::BadArgument);
            }
            let mut depth = 2usize;
            let mut ancestor = u16_at(pr, 2);
            while ancestor != NONE_ID {
                depth += 1;
                if depth > 4 {
                    return Err(UiMountError::NoMemory);
                }
                ancestor = (0..parent_index)
                    .find_map(|p| {
                        let candidate = record(packet, p);
                        (u16_at(candidate, 0) == ancestor).then(|| u16_at(candidate, 2))
                    })
                    .ok_or(UiMountError::BadArgument)?;
            }
        }
        let text_len = usize::from(u16_at(r, 20));
        let text_cap = usize::from(u16_at(r, 22));
        let value_len = usize::from(u16_at(r, 28));
        let value_cap = usize::from(u16_at(r, 30));
        if text_len > text_cap || value_len > value_cap {
            return Err(UiMountError::BadArgument);
        }
        arena = arena
            .checked_add(text_cap)
            .and_then(|n| n.checked_add(value_cap))
            .ok_or(UiMountError::NoMemory)?;
        if arena > UI_DATA_CAPACITY {
            return Err(UiMountError::NoMemory);
        }
        let text = source(packet, data_offset, data_len, u32_at(r, 16), text_len)?;
        str::from_utf8(text).map_err(|_| UiMountError::BadArgument)?;
        let value = source(packet, data_offset, data_len, u32_at(r, 24), value_len)?;
        validate_kind(
            kind,
            state,
            &mut text_fields,
            &mut text_field_capacity,
            &mut list_rows,
            &mut dialogs,
            &mut open_dialogs,
            r,
            value,
        )?;
    }
    if roots != 1
        || dialogs > 2
        || open_dialogs > UI_MAX_OPEN_MODALS
        || text_fields > UI_MAX_TEXT_FIELDS
        || text_field_capacity > 512
        || list_rows > UI_MAX_LIST_ROWS
    {
        return Err(UiMountError::NoMemory);
    }
    if focus_id != NONE_ID {
        let focus = (0..count)
            .find_map(|i| {
                let r = record(packet, i);
                (u16_at(r, 0) == focus_id).then_some(r)
            })
            .ok_or(UiMountError::BadArgument)?;
        if !matches!(focus[4], 2..=5) || focus[5] & 3 != 3 {
            return Err(UiMountError::BadArgument);
        }
        if let Some(open) = (0..count).find_map(|i| {
            let r = record(packet, i);
            (r[4] == 7 && u16_at(r, 6) & 1 != 0).then(|| u16_at(r, 0))
        }) {
            if !descends_from(packet, count, u16_at(focus, 0), open) {
                return Err(UiMountError::BadArgument);
            }
        } else if (0..count).any(|index| {
            let dialog = record(packet, index);
            dialog[4] == 7 && descends_from(packet, count, u16_at(focus, 0), u16_at(dialog, 0))
        }) {
            return Err(UiMountError::BadArgument);
        }
    }
    for index in 0..count {
        let dialog = record(packet, index);
        if dialog[4] != 7 {
            continue;
        }
        let id = u16_at(dialog, 0);
        let children = (0..count)
            .filter(|child| u16_at(record(packet, *child), 2) == id)
            .count();
        let actions = (0..count)
            .filter(|child| {
                let r = record(packet, *child);
                u16_at(r, 2) == id && r[4] == 2 && u16_at(r, 6) & 1 != 0
            })
            .count();
        if children > 8 || actions > 4 {
            return Err(UiMountError::NoMemory);
        }
        for arg_offset in [40usize, 44] {
            let action = i32_at(dialog, arg_offset);
            if action == -1 {
                continue;
            }
            let action = u16::try_from(action).map_err(|_| UiMountError::BadArgument)?;
            let valid = (0..count).any(|child| {
                let r = record(packet, child);
                u16_at(r, 0) == action
                    && u16_at(r, 2) == id
                    && r[4] == 2
                    && r[5] & 3 == 3
                    && u16_at(r, 6) & 1 != 0
            });
            if !valid {
                return Err(UiMountError::BadArgument);
            }
        }
    }
    Ok(MountMeta {
        count,
        data_offset,
        root_id,
        focus_id,
    })
}

#[allow(clippy::too_many_arguments)]
fn validate_kind(
    kind: u8,
    state: u16,
    text_fields: &mut usize,
    text_field_capacity: &mut usize,
    list_rows: &mut usize,
    dialogs: &mut usize,
    open_dialogs: &mut usize,
    r: &[u8],
    value: &[u8],
) -> Result<(), UiMountError> {
    let args = [i32_at(r, 32), i32_at(r, 36), i32_at(r, 40), i32_at(r, 44)];
    match kind {
        1 if !(0..=2).contains(&args[0]) || args[1..] != [0; 3] || !value.is_empty() => {
            Err(UiMountError::BadArgument)
        }
        2 if args != [0; 4] || !value.is_empty() => Err(UiMountError::BadArgument),
        3 => {
            let width = i32::from(u16_at(r, 12));
            let height = i32::from(u16_at(r, 14));
            let side = height.min(12);
            let mark_y = (height - side) / 2 + args[1];
            if args[0] < 0
                || args[0] + side > width
                || mark_y < 0
                || mark_y + side > height
                || args[2..] != [0; 2]
                || !value.is_empty()
            {
                Err(UiMountError::BadArgument)
            } else {
                Ok(())
            }
        }
        4 => {
            let rows = usize::try_from(args[0]).map_err(|_| UiMountError::BadArgument)?;
            *list_rows += rows;
            if args[1] < -1
                || args[1] >= args[0]
                || args[2] <= 0
                || rows.checked_mul(12).is_none_or(|n| n > value.len())
            {
                return Err(UiMountError::BadArgument);
            }
            for row in 0..rows {
                let rr = &value[row * 12..row * 12 + 12];
                if u16_at(rr, 4) & !1 != 0 || u16_at(rr, 6) != 0 {
                    return Err(UiMountError::BadArgument);
                }
                if usize::from(u16_at(rr, 0)) < rows * 12 {
                    return Err(UiMountError::BadArgument);
                }
                str::from_utf8(source(
                    value,
                    0,
                    value.len(),
                    u32::from(u16_at(rr, 0)),
                    usize::from(u16_at(rr, 2)),
                )?)
                .map_err(|_| UiMountError::BadArgument)?;
            }
            if args[1] >= 0 {
                let start = args[1] as usize * 12;
                if u16_at(&value[start..start + 12], 4) & 1 == 0 {
                    return Err(UiMountError::BadArgument);
                }
            }
            if args[3] != 0 {
                return Err(UiMountError::BadArgument);
            }
            Ok(())
        }
        5 => {
            *text_fields += 1;
            let cap = usize::from(u16_at(r, 30));
            *text_field_capacity += cap;
            if cap > UI_MAX_TEXT_FIELD_BYTES
                || args[0] < -1
                || (args[0] != -1 && args[0] as usize > value.len())
                || str::from_utf8(value).is_err()
                || (args[0] >= 0
                    && !str::from_utf8(value)
                        .unwrap_or("")
                        .is_char_boundary(args[0] as usize))
                || args[1..] != [0; 3]
            {
                Err(UiMountError::BadArgument)
            } else {
                Ok(())
            }
        }
        6 | 7
            if !(0..=32).contains(&args[0])
                || !(0..=64).contains(&args[1])
                || !value.is_empty() =>
        {
            Err(UiMountError::BadArgument)
        }
        6 if args[2..] != [0; 2] => Err(UiMountError::BadArgument),
        7 => {
            *dialogs += 1;
            *open_dialogs += usize::from(state & 1);
            Ok(())
        }
        _ => Ok(()),
    }
}

fn descends_from(packet: &[u8], count: usize, mut child: u16, ancestor: u16) -> bool {
    for _ in 0..count {
        let Some(parent) = (0..count).find_map(|index| {
            let r = record(packet, index);
            (u16_at(r, 0) == child).then(|| u16_at(r, 2))
        }) else {
            return false;
        };
        if parent == ancestor {
            return true;
        }
        if parent == NONE_ID {
            return false;
        }
        child = parent;
    }
    false
}

fn descends_from_nodes(nodes: &[UiNode], mut child: u16, ancestor: u16) -> bool {
    for _ in 0..nodes.len() {
        let Some(parent) = nodes
            .iter()
            .find(|node| node.id == child)
            .map(|node| node.parent_id)
        else {
            return false;
        };
        if parent == ancestor {
            return true;
        }
        if parent == NONE_ID {
            return false;
        }
        child = parent;
    }
    false
}

fn contained(child: &[u8], parent: &[u8]) -> bool {
    let x = i64::from(i16_at(child, 8));
    let y = i64::from(i16_at(child, 10));
    let px = i64::from(i16_at(parent, 8));
    let py = i64::from(i16_at(parent, 10));
    x >= px
        && y >= py
        && x + i64::from(u16_at(child, 12)) <= px + i64::from(u16_at(parent, 12))
        && y + i64::from(u16_at(child, 14)) <= py + i64::from(u16_at(parent, 14))
}
fn record(packet: &[u8], index: usize) -> &[u8] {
    &packet[HEADER + index * STRIDE..HEADER + (index + 1) * STRIDE]
}
fn source(
    packet: &[u8],
    base: usize,
    total: usize,
    offset: u32,
    len: usize,
) -> Result<&[u8], UiMountError> {
    let offset = usize::try_from(offset).map_err(|_| UiMountError::BadArgument)?;
    if offset.checked_add(len).is_none_or(|end| end > total) {
        return Err(UiMountError::BadArgument);
    }
    Ok(&packet[base + offset..base + offset + len])
}
fn copy_source(packet: &[u8], base: usize, offset: u32, len: u16, dst: &mut [u8], at: usize) {
    let src = &packet[base + offset as usize..base + offset as usize + len as usize];
    dst[at..at + src.len()].copy_from_slice(src);
}
fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn i16_at(b: &[u8], o: usize) -> i16 {
    i16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn i32_at(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn put_u16(dst: &mut [u8], offset: usize, value: u16) {
    dst[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(dst: &mut [u8], offset: usize, value: u32) {
    dst[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_i32(dst: &mut [u8], offset: usize, value: i32) {
    dst[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn copy_fixed<const N: usize>(dst: &mut [u8; N], len: &mut u8, src: &[u8]) {
    dst.fill(0);
    dst[..src.len()].copy_from_slice(src);
    *len = src.len() as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> [u8; 142] {
        let hex =
            include_str!("../../../harness/fixtures/koto_ui_abi/valid_panel_button_mount.hex")
                .trim()
                .as_bytes();
        let mut out = [0; 142];
        for (index, byte) in out.iter_mut().enumerate() {
            *byte = nibble(hex[index * 2]) << 4 | nibble(hex[index * 2 + 1]);
        }
        out
    }
    fn ime_fixture() -> Vec<u8> {
        let mut out = fixture().to_vec();
        out.resize(174, 0);
        out[8..12].copy_from_slice(&174u32.to_le_bytes());
        out[24..28].copy_from_slice(&38u32.to_le_bytes());
        out[92] = 5;
        out[94..96].copy_from_slice(&1u16.to_le_bytes());
        out[112..116].copy_from_slice(&6u32.to_le_bytes());
        out[118..120].copy_from_slice(&32u16.to_le_bytes());
        out[120..124].copy_from_slice(&(-1i32).to_le_bytes());
        out
    }
    fn ime_focus_fixture() -> Vec<u8> {
        let ime = ime_fixture();
        let mut out = vec![0u8; 222];
        out[..136].copy_from_slice(&ime[..136]);
        out[184..].copy_from_slice(&ime[136..]);
        out[8..12].copy_from_slice(&222u32.to_le_bytes());
        out[12..14].copy_from_slice(&3u16.to_le_bytes());
        out[20..24].copy_from_slice(&184u32.to_le_bytes());
        let button = &mut out[136..184];
        button[0..2].copy_from_slice(&3u16.to_le_bytes());
        button[2..4].copy_from_slice(&1u16.to_le_bytes());
        button[4] = 2;
        button[5] = 3;
        button[8..10].copy_from_slice(&100i16.to_le_bytes());
        button[10..12].copy_from_slice(&40i16.to_le_bytes());
        button[12..14].copy_from_slice(&80u16.to_le_bytes());
        button[14..16].copy_from_slice(&20u16.to_le_bytes());
        out
    }

    struct TestNode<'a> {
        id: u16,
        parent: u16,
        kind: u8,
        flags: u8,
        state: u16,
        bounds: UiRect,
        text: &'a str,
        text_capacity: u16,
        value: &'a [u8],
        value_capacity: u16,
        args: [i32; 4],
    }

    fn mount_packet(nodes: &[TestNode<'_>], focus: u16) -> Vec<u8> {
        let data_offset = HEADER + nodes.len() * STRIDE;
        let data_len: usize = nodes
            .iter()
            .map(|node| node.text.len() + node.value.len())
            .sum();
        let mut packet = vec![0u8; data_offset + data_len];
        packet[..4].copy_from_slice(b"KUI1");
        put_u16(&mut packet, 4, 1);
        put_u32(&mut packet, 8, (data_offset + data_len) as u32);
        put_u16(&mut packet, 12, nodes.len() as u16);
        put_u16(&mut packet, 14, STRIDE as u16);
        put_u32(&mut packet, 16, HEADER as u32);
        put_u32(&mut packet, 20, data_offset as u32);
        put_u32(&mut packet, 24, data_len as u32);
        put_u16(&mut packet, 28, nodes[0].id);
        put_u16(&mut packet, 30, focus);

        let mut cursor = 0usize;
        for (index, node) in nodes.iter().enumerate() {
            let record = HEADER + index * STRIDE;
            put_u16(&mut packet, record, node.id);
            put_u16(&mut packet, record + 2, node.parent);
            packet[record + 4] = node.kind;
            packet[record + 5] = node.flags;
            put_u16(&mut packet, record + 6, node.state);
            packet[record + 8..record + 10].copy_from_slice(&(node.bounds.x as i16).to_le_bytes());
            packet[record + 10..record + 12].copy_from_slice(&(node.bounds.y as i16).to_le_bytes());
            put_u16(&mut packet, record + 12, node.bounds.w as u16);
            put_u16(&mut packet, record + 14, node.bounds.h as u16);
            put_u32(&mut packet, record + 16, cursor as u32);
            put_u16(&mut packet, record + 20, node.text.len() as u16);
            put_u16(&mut packet, record + 22, node.text_capacity);
            packet[data_offset + cursor..data_offset + cursor + node.text.len()]
                .copy_from_slice(node.text.as_bytes());
            cursor += node.text.len();
            put_u32(&mut packet, record + 24, cursor as u32);
            put_u16(&mut packet, record + 28, node.value.len() as u16);
            put_u16(&mut packet, record + 30, node.value_capacity);
            packet[data_offset + cursor..data_offset + cursor + node.value.len()]
                .copy_from_slice(node.value);
            cursor += node.value.len();
            for (arg, value) in node.args.iter().enumerate() {
                put_i32(&mut packet, record + 32 + arg * 4, *value);
            }
        }
        packet
    }

    fn list_blob() -> Vec<u8> {
        let mut blob = vec![0u8; 32];
        put_u16(&mut blob, 0, 24);
        put_u16(&mut blob, 2, 3);
        put_i32(&mut blob, 8, 10);
        put_u16(&mut blob, 12, 27);
        put_u16(&mut blob, 14, 5);
        put_u16(&mut blob, 16, 1);
        put_i32(&mut blob, 20, 20);
        blob[24..].copy_from_slice(b"OffReady");
        blob
    }

    fn all_node_kinds_fixture(dialog_open: bool) -> Vec<u8> {
        let list = list_blob();
        mount_packet(
            &[
                TestNode {
                    id: 1,
                    parent: NONE_ID,
                    kind: 6,
                    flags: 3,
                    state: 0,
                    bounds: UiRect::new(0, 0, 320, 320),
                    text: "Root",
                    text_capacity: 4,
                    value: &[],
                    value_capacity: 0,
                    args: [4, 18, 0, 0],
                },
                TestNode {
                    id: 2,
                    parent: 1,
                    kind: 1,
                    flags: 3,
                    state: 0,
                    bounds: UiRect::new(8, 24, 120, 20),
                    text: "Label",
                    text_capacity: 5,
                    value: &[],
                    value_capacity: 0,
                    args: [0, 0, 0, 0],
                },
                TestNode {
                    id: 3,
                    parent: 1,
                    kind: 2,
                    flags: 3,
                    state: 0,
                    bounds: UiRect::new(8, 48, 80, 24),
                    text: "Button",
                    text_capacity: 6,
                    value: &[],
                    value_capacity: 0,
                    args: [0; 4],
                },
                TestNode {
                    id: 4,
                    parent: 1,
                    kind: 3,
                    flags: 1,
                    state: 1,
                    bounds: UiRect::new(96, 48, 100, 24),
                    text: "Disabled",
                    text_capacity: 8,
                    value: &[],
                    value_capacity: 0,
                    args: [0; 4],
                },
                TestNode {
                    id: 5,
                    parent: 1,
                    kind: 4,
                    flags: 3,
                    state: 1,
                    bounds: UiRect::new(8, 80, 150, 60),
                    text: "List",
                    text_capacity: 4,
                    value: &list,
                    value_capacity: list.len() as u16,
                    args: [2, 1, 24, 0],
                },
                TestNode {
                    id: 6,
                    parent: 1,
                    kind: 5,
                    flags: 3,
                    state: 0,
                    bounds: UiRect::new(8, 144, 180, 28),
                    text: "Input",
                    text_capacity: 5,
                    value: b"abc",
                    value_capacity: 32,
                    args: [3, 0, 0, 0],
                },
                TestNode {
                    id: 7,
                    parent: 1,
                    kind: 7,
                    flags: 3,
                    state: u16::from(dialog_open),
                    bounds: UiRect::new(20, 180, 280, 120),
                    text: "Dialog",
                    text_capacity: 6,
                    value: &[],
                    value_capacity: 0,
                    args: [4, 18, 8, 8],
                },
                TestNode {
                    id: 8,
                    parent: 7,
                    kind: 2,
                    flags: 3,
                    state: 1,
                    bounds: UiRect::new(120, 250, 80, 24),
                    text: "Close",
                    text_capacity: 5,
                    value: &[],
                    value_capacity: 0,
                    args: [0; 4],
                },
            ],
            if dialog_open { 8 } else { 3 },
        )
    }
    fn nibble(b: u8) -> u8 {
        match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            _ => panic!(),
        }
    }

    #[derive(Default)]
    struct Recorder {
        calls: usize,
    }

    #[derive(Default)]
    struct TextRecorder {
        text: Vec<String>,
        ellipsis: bool,
    }
    impl TextMetrics for TextRecorder {
        fn measure_text(&mut self, text: &str) -> Result<i32, PaintError> {
            Ok(text.chars().count() as i32 * 8)
        }
        fn supports_glyph(&self, ch: char) -> bool {
            self.ellipsis && ch == '…'
        }
    }
    impl Painter for TextRecorder {
        fn fill_rect(&mut self, _: UiRect, _: UiRect, _: Rgb565) -> Result<(), PaintError> {
            Ok(())
        }
        fn stroke_rect(
            &mut self,
            _: UiRect,
            _: UiRect,
            _: Rgb565,
            _: u8,
        ) -> Result<(), PaintError> {
            Ok(())
        }
        fn draw_text(&mut self, _: UiRect, _: UiRect, run: TextRun<'_>) -> Result<(), PaintError> {
            self.text.push(run.text.to_string());
            Ok(())
        }
        fn draw_glyphs(&mut self, _: UiRect, _: UiRect, _: GlyphRun<'_>) -> Result<(), PaintError> {
            Ok(())
        }
        fn draw_focus_mark(
            &mut self,
            _: UiRect,
            _: UiRect,
            _: Rgb565,
            _: u8,
        ) -> Result<(), PaintError> {
            Ok(())
        }
    }
    impl koto_ui::TextMetrics for Recorder {
        fn measure_text(&mut self, text: &str) -> Result<i32, PaintError> {
            Ok(text.chars().count() as i32 * 8)
        }
    }
    impl Painter for Recorder {
        fn fill_rect(
            &mut self,
            _: UiRect,
            _: UiRect,
            _: koto_ui::Rgb565,
        ) -> Result<(), PaintError> {
            self.calls += 1;
            Ok(())
        }
        fn stroke_rect(
            &mut self,
            _: UiRect,
            _: UiRect,
            _: koto_ui::Rgb565,
            _: u8,
        ) -> Result<(), PaintError> {
            self.calls += 1;
            Ok(())
        }
        fn draw_text(
            &mut self,
            _: UiRect,
            _: UiRect,
            _: koto_ui::TextRun<'_>,
        ) -> Result<(), PaintError> {
            self.calls += 1;
            Ok(())
        }
        fn draw_glyphs(
            &mut self,
            _: UiRect,
            _: UiRect,
            _: koto_ui::GlyphRun<'_>,
        ) -> Result<(), PaintError> {
            self.calls += 1;
            Ok(())
        }
        fn draw_focus_mark(
            &mut self,
            _: UiRect,
            _: UiRect,
            _: koto_ui::Rgb565,
            _: u8,
        ) -> Result<(), PaintError> {
            self.calls += 1;
            Ok(())
        }
    }

    fn update_packet(widget: u16, property: u8, data: &[u8], args: [i32; 4]) -> Vec<u8> {
        let mut packet = vec![0; UPDATE_HEADER + UPDATE_STRIDE + data.len()];
        packet[0..4].copy_from_slice(b"KUP1");
        packet[4..6].copy_from_slice(&1u16.to_le_bytes());
        let total = packet.len() as u32;
        packet[8..12].copy_from_slice(&total.to_le_bytes());
        packet[12..14].copy_from_slice(&1u16.to_le_bytes());
        packet[14..16].copy_from_slice(&(UPDATE_STRIDE as u16).to_le_bytes());
        packet[16..20].copy_from_slice(&(UPDATE_HEADER as u32).to_le_bytes());
        packet[20..24].copy_from_slice(&((UPDATE_HEADER + UPDATE_STRIDE) as u32).to_le_bytes());
        packet[24..28].copy_from_slice(&(data.len() as u32).to_le_bytes());
        let r = UPDATE_HEADER;
        packet[r..r + 2].copy_from_slice(&widget.to_le_bytes());
        packet[r + 2] = property;
        packet[r + 8..r + 10].copy_from_slice(&(data.len() as u16).to_le_bytes());
        for (index, arg) in args.iter().enumerate() {
            packet[r + 12 + index * 4..r + 16 + index * 4].copy_from_slice(&arg.to_le_bytes());
        }
        packet[UPDATE_HEADER + UPDATE_STRIDE..].copy_from_slice(data);
        packet
    }

    #[test]
    fn mounts_canonical_fixture_into_owned_storage() {
        let packet = fixture();
        let mut session = UiSession::new();
        session.mount(&packet).unwrap();
        assert_eq!(session.root_id(), Some(1));
        assert_eq!(session.initial_focus_id(), Some(2));
        assert_eq!(session.nodes().len(), 2);
        assert_eq!(session.text(&session.nodes()[0]), "Demo");
        assert_eq!(session.text(&session.nodes()[1]), "OK");
    }

    #[test]
    fn full_node_capacity_mounts_and_overflow_is_atomic() {
        let mut nodes = Vec::with_capacity(UI_MAX_NODES);
        nodes.push(TestNode {
            id: 1,
            parent: NONE_ID,
            kind: 6,
            flags: 3,
            state: 0,
            bounds: UiRect::new(0, 0, 320, 320),
            text: "",
            text_capacity: 0,
            value: &[],
            value_capacity: 0,
            args: [0; 4],
        });
        for id in 2..=UI_MAX_NODES as u16 {
            nodes.push(TestNode {
                id,
                parent: 1,
                kind: 1,
                flags: 3,
                state: 0,
                bounds: UiRect::new(i32::from(id), i32::from(id), 1, 1),
                text: "",
                text_capacity: 0,
                value: &[],
                value_capacity: 0,
                args: [0; 4],
            });
        }
        let packet = mount_packet(&nodes, NONE_ID);
        assert!(packet.len() <= UI_MAX_MOUNT_BYTES);
        let mut session = UiSession::new();
        session.mount(&packet).unwrap();
        assert_eq!(session.nodes().len(), UI_MAX_NODES);

        let before = session.clone();
        let mut overflow = packet;
        put_u16(&mut overflow, 12, (UI_MAX_NODES + 1) as u16);
        assert_eq!(session.mount(&overflow), Err(UiMountError::NoMemory));
        assert_eq!(session, before);
    }

    #[test]
    fn invalid_mount_is_atomic_and_canonical_mutations_are_stable() {
        let valid = fixture();
        let mut session = UiSession::new();
        session.mount(&valid).unwrap();
        let before = session.clone();
        let mut bad = valid;
        bad[88..90].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(session.mount(&bad), Err(UiMountError::BadArgument));
        assert_eq!(session, before);
        assert_eq!(session.mount(&valid[..141]), Err(UiMountError::BadArgument));
        assert_eq!(session, before);

        let mut unsupported = valid;
        unsupported[4..6].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(session.mount(&unsupported), Err(UiMountError::Unsupported));

        let mut too_many = valid;
        too_many[12..14].copy_from_slice(&33u16.to_le_bytes());
        assert_eq!(session.mount(&too_many), Err(UiMountError::NoMemory));

        let mut bad_parent = valid;
        bad_parent[90..92].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(session.mount(&bad_parent), Err(UiMountError::BadArgument));

        let mut bad_utf8 = valid;
        bad_utf8[141] = 0xff;
        assert_eq!(session.mount(&bad_utf8), Err(UiMountError::BadArgument));

        let mut reserved = valid;
        reserved[36] = 1;
        assert_eq!(session.mount(&reserved), Err(UiMountError::BadArgument));
        assert_eq!(session, before);

        let mut bad_kind = valid;
        bad_kind[92] = 8;
        assert_eq!(session.mount(&bad_kind), Err(UiMountError::BadArgument));
        assert_eq!(session, before);

        let mut zero_width = valid;
        zero_width[100..102].copy_from_slice(&0u16.to_le_bytes());
        assert_eq!(session.mount(&zero_width), Err(UiMountError::BadArgument));
        assert_eq!(session, before);

        let mut outside_parent = valid;
        outside_parent[96..98].copy_from_slice(&300i16.to_le_bytes());
        assert_eq!(
            session.mount(&outside_parent),
            Err(UiMountError::BadArgument)
        );
        assert_eq!(session, before);
    }

    #[test]
    fn direction_and_overflow_flags_follow_v1_support_boundary() {
        let mut session = UiSession::new();
        let mut explicit_ltr = fixture();
        explicit_ltr[93] |= 1 << 2;
        assert_eq!(session.mount(&explicit_ltr), Ok(()));

        let mut rtl = fixture();
        rtl[93] |= 2 << 2;
        assert_eq!(session.mount(&rtl), Err(UiMountError::Unsupported));

        let mut wrap = fixture();
        wrap[93] |= 2 << 4;
        assert_eq!(session.mount(&wrap), Err(UiMountError::Unsupported));
    }

    #[test]
    fn text_overflow_clips_or_ellipsizes_without_splitting_utf8() {
        let clip = UiRect::new(0, 0, 320, 20);
        let bounds = UiRect::new(0, 0, 40, 20);
        let run = TextRun {
            text: "abcdef疑似ロケール",
            color: Rgb565(1),
            align: TextAlign::Start,
        };

        let mut clipped = TextRecorder::default();
        AbiTextPainter {
            inner: &mut clipped,
            overflow: 0,
        }
        .draw_text(clip, bounds, run)
        .unwrap();
        assert_eq!(clipped.text, ["abcde"]);

        let mut ascii_ellipsis = TextRecorder::default();
        AbiTextPainter {
            inner: &mut ascii_ellipsis,
            overflow: 1,
        }
        .draw_text(clip, bounds, run)
        .unwrap();
        assert_eq!(ascii_ellipsis.text.concat(), "ab...");

        let mut unicode_ellipsis = TextRecorder {
            ellipsis: true,
            ..TextRecorder::default()
        };
        AbiTextPainter {
            inner: &mut unicode_ellipsis,
            overflow: 1,
        }
        .draw_text(clip, bounds, run)
        .unwrap();
        assert_eq!(unicode_ellipsis.text.concat(), "abcd…");

        let mut long_english = TextRecorder::default();
        AbiTextPainter {
            inner: &mut long_english,
            overflow: 1,
        }
        .draw_text(
            clip,
            bounds,
            TextRun {
                text: "Settings and wireless configuration",
                color: Rgb565(1),
                align: TextAlign::Start,
            },
        )
        .unwrap();
        assert_eq!(long_english.text.concat(), "Se...");
    }

    #[test]
    fn session_stays_within_the_frozen_sram_ceiling() {
        // The size is compile-time constant; keep the runtime read so the
        // ceiling shows up as a normal failing test instead of a build error.
        let session_bytes = core::hint::black_box(UI_SESSION_SRAM_BYTES);
        assert!(session_bytes <= 8192);
    }

    #[test]
    fn update_changes_owned_text_and_records_targeted_damage() {
        let mut session = UiSession::new();
        session.mount(&fixture()).unwrap();
        session.clear_damage();

        session.update(&update_packet(2, 1, b"Go", [0; 4])).unwrap();

        assert_eq!(session.text(&session.nodes()[1]), "Go");
        assert_eq!(
            session.damaged_rects().collect::<Vec<_>>(),
            [UiRect::new(10, 40, 80, 20)]
        );
        session.clear_damage();
        assert_eq!(session.damaged_rects().count(), 0);

        session.update(&update_packet(2, 1, b"Go", [0; 4])).unwrap();
        assert_eq!(session.damaged_rects().count(), 0);
    }

    #[test]
    fn invalid_update_rolls_back_without_damage() {
        let mut session = UiSession::new();
        session.mount(&fixture()).unwrap();
        session.clear_damage();
        let before = session.clone();

        assert_eq!(
            session.update(&update_packet(2, 1, b"Long", [0; 4])),
            Err(UiMountError::BadArgument)
        );
        assert_eq!(session, before);
        assert_eq!(session.damaged_rects().count(), 0);
    }

    #[test]
    fn representative_panel_button_trace_has_bounded_commands_and_zero_idle_work() {
        let mut session = UiSession::new();
        session.mount(&fixture()).unwrap();
        let mut painter = Recorder::default();

        assert_eq!(
            session.damaged_rects().collect::<Vec<_>>(),
            [UiRect::new(0, 0, 320, 320)]
        );
        assert_eq!(
            session.paint_pending(&mut painter, &Theme::DARK).unwrap(),
            1
        );
        // Panel: fill/stroke/title/rule; focused Button:
        // fill/stroke/text/focus. Backends may expand each stroke into 4 rects.
        assert_eq!(painter.calls, 8);
        assert_eq!(session.damaged_rects().count(), 1);
        session.complete_present();
        let calls = painter.calls;
        assert_eq!(
            session.paint_pending(&mut painter, &Theme::DARK).unwrap(),
            0
        );
        assert_eq!(painter.calls, calls);
    }

    #[test]
    fn all_seven_node_kinds_mount_and_paint_through_the_shared_adapter() {
        let mut session = UiSession::new();
        session.mount(&all_node_kinds_fixture(true)).unwrap();
        assert_eq!(
            session
                .nodes()
                .iter()
                .map(|node| node.kind)
                .collect::<Vec<_>>(),
            [6, 1, 2, 3, 4, 5, 7, 2]
        );

        let mut painter = TextRecorder::default();
        session.paint_pending(&mut painter, &Theme::DARK).unwrap();
        for expected in [
            "Root", "Label", "Button", "Disabled", "Ready", "abc", "Dialog", "Close",
        ] {
            assert!(
                painter.text.iter().any(|text| text == expected),
                "{expected}: {:?}",
                painter.text
            );
        }
    }

    #[test]
    fn closed_dialog_and_its_children_are_not_painted() {
        let mut session = UiSession::new();
        session.mount(&all_node_kinds_fixture(false)).unwrap();

        let mut painter = TextRecorder::default();
        session.paint_pending(&mut painter, &Theme::DARK).unwrap();

        assert!(!painter.text.iter().any(|text| text == "Dialog"));
        assert!(!painter.text.iter().any(|text| text == "Close"));
        assert!(painter.text.iter().any(|text| text == "Button"));
    }

    #[test]
    fn checkbox_mark_offsets_are_bounded_by_the_control() {
        let mut packet = all_node_kinds_fixture(false);
        let checkbox = HEADER + 3 * STRIDE;
        put_i32(&mut packet, checkbox + 32, 4);
        put_i32(&mut packet, checkbox + 36, -2);
        assert!(UiSession::new().mount(&packet).is_ok());

        put_i32(&mut packet, checkbox + 32, 100);
        assert_eq!(
            UiSession::new().mount(&packet),
            Err(UiMountError::BadArgument)
        );
    }

    #[test]
    fn open_dialog_traps_focus_and_routes_activation_to_its_action() {
        let mut session = UiSession::new();
        session.mount(&all_node_kinds_fixture(true)).unwrap();
        session.complete_present();
        let config = crate::ConfigService::new().snapshot();

        session.frame_begin(
            VmInputSnapshot {
                pressed_bits: 1 << 3,
                ..VmInputSnapshot::empty()
            },
            config,
        );
        assert_eq!(session.focused_id(), Some(8));
        assert_eq!(session.damaged_rects().count(), 0);

        session.frame_begin(
            VmInputSnapshot {
                pressed_bits: 1 << 4,
                ..VmInputSnapshot::empty()
            },
            config,
        );
        let mut event = [0u8; 64];
        assert_eq!(session.poll_event(&mut event), Ok(32));
        assert_eq!((event[12], u16_at(&event, 14)), (1, 8));
    }

    #[test]
    fn focus_skips_disabled_control_and_list_activation_keeps_app_value() {
        let mut session = UiSession::new();
        session.mount(&all_node_kinds_fixture(false)).unwrap();
        session.complete_present();
        let config = crate::ConfigService::new().snapshot();

        session.frame_begin(
            VmInputSnapshot {
                pressed_bits: 1 << 1,
                ..VmInputSnapshot::empty()
            },
            config,
        );
        assert_eq!(session.focused_id(), Some(5));
        let mut event = [0u8; 64];
        assert_eq!(session.poll_event(&mut event), Ok(32));
        assert_eq!((event[12], u16_at(&event, 14)), (8, 5));
        assert_eq!(i32_at(&event, 16), 3);

        session.frame_begin(
            VmInputSnapshot {
                pressed_bits: 1 << 4,
                ..VmInputSnapshot::empty()
            },
            config,
        );
        assert_eq!(session.poll_event(&mut event), Ok(32));
        assert_eq!((event[12], u16_at(&event, 14)), (5, 5));
        assert_eq!((i32_at(&event, 16), i32_at(&event, 20)), (1, 20));
    }

    #[test]
    fn arrows_use_spatial_focus_and_tab_uses_widget_id_order() {
        let mut packet = all_node_kinds_fixture(false);
        packet[HEADER + 3 * STRIDE + 5] = 3;
        let list_record = HEADER + 4 * STRIDE;
        let list_blob = u32_at(&packet, 20) as usize + u32_at(&packet, list_record + 24) as usize;
        put_u16(&mut packet, list_blob + 4, 1);
        let mut session = UiSession::new();
        session.mount(&packet).unwrap();
        session.complete_present();
        let config = crate::ConfigService::new().snapshot();
        let mut event = [0u8; 64];

        session.frame_begin(
            VmInputSnapshot {
                intent_bits: crate::runtime::text_intent::RIGHT,
                ..VmInputSnapshot::empty()
            },
            config,
        );
        assert_eq!(session.focused_id(), Some(4));
        assert_eq!(session.poll_event(&mut event), Ok(32));
        assert_eq!((event[12], u16_at(&event, 14)), (8, 4));

        session.frame_begin(
            VmInputSnapshot {
                intent_bits: crate::runtime::text_intent::CONVERT,
                ..VmInputSnapshot::empty()
            },
            config,
        );
        assert_eq!(session.focused_id(), Some(5));
        assert_eq!(session.poll_event(&mut event), Ok(32));
        assert_eq!((event[12], u16_at(&event, 14)), (8, 5));

        session.frame_begin(
            VmInputSnapshot {
                intent_bits: crate::runtime::text_intent::UP,
                ..VmInputSnapshot::empty()
            },
            config,
        );
        assert_eq!(session.focused_id(), Some(5));
        assert_eq!(session.poll_event(&mut event), Ok(32));
        assert_eq!((event[12], u16_at(&event, 14)), (4, 5));
        assert_eq!(i32_at(&event, 16), 0);

        session.frame_begin(
            VmInputSnapshot {
                intent_bits: crate::runtime::text_intent::UP,
                ..VmInputSnapshot::empty()
            },
            config,
        );
        assert_eq!(session.focused_id(), Some(3));
        assert_eq!(session.poll_event(&mut event), Ok(32));
        assert_eq!((event[12], u16_at(&event, 14)), (8, 3));
    }

    #[test]
    fn enabled_checkbox_activation_toggles_and_emits_value_change() {
        let mut packet = all_node_kinds_fixture(false);
        put_u16(&mut packet, 30, 4);
        packet[HEADER + 3 * STRIDE + 5] = 3;
        let mut session = UiSession::new();
        session.mount(&packet).unwrap();
        session.complete_present();
        session.frame_begin(
            VmInputSnapshot {
                pressed_bits: 1 << 4,
                ..VmInputSnapshot::empty()
            },
            crate::ConfigService::new().snapshot(),
        );

        let mut event = [0u8; 64];
        assert_eq!(session.poll_event(&mut event), Ok(32));
        assert_eq!(
            (event[12], u16_at(&event, 14), i32_at(&event, 16)),
            (2, 4, 0)
        );
        assert_eq!(
            session.damaged_rects().collect::<Vec<_>>(),
            [UiRect::new(96, 48, 100, 24)]
        );
    }

    #[test]
    fn space_activates_checkbox_once_and_remains_text_for_text_fields() {
        let mut packet = all_node_kinds_fixture(false);
        put_u16(&mut packet, 30, 4);
        packet[HEADER + 3 * STRIDE + 5] = 3;
        let mut session = UiSession::new();
        session.mount(&packet).unwrap();
        session.complete_present();
        let config = crate::ConfigService::new().snapshot();

        session.frame_begin(
            VmInputSnapshot {
                text_codepoint: ' ' as u32,
                ..VmInputSnapshot::empty()
            },
            config,
        );
        let mut event = [0u8; 64];
        assert_eq!(session.poll_event(&mut event), Ok(32));
        assert_eq!(
            (event[12], u16_at(&event, 14), i32_at(&event, 16)),
            (2, 4, 0)
        );

        // Hosts that already pair Space with button A must not toggle twice.
        session.frame_begin(
            VmInputSnapshot {
                pressed_bits: 1 << 4,
                text_codepoint: ' ' as u32,
                ..VmInputSnapshot::empty()
            },
            config,
        );
        assert_eq!(session.poll_event(&mut event), Ok(32));
        assert_eq!(
            (event[12], u16_at(&event, 14), i32_at(&event, 16)),
            (2, 4, 1)
        );
        assert_eq!(session.poll_event(&mut event), Ok(0));

        let mut text_session = UiSession::new();
        text_session.mount(&ime_fixture()).unwrap();
        text_session.complete_present();
        text_session.frame_begin(
            VmInputSnapshot {
                pressed_bits: 1 << 4,
                ..VmInputSnapshot::empty()
            },
            config,
        );
        assert_eq!(text_session.editing_id(), Some(2));
        text_session.frame_begin(
            VmInputSnapshot {
                text_codepoint: ' ' as u32,
                ..VmInputSnapshot::empty()
            },
            config,
        );
        assert_eq!(text_session.poll_event(&mut event), Ok(33));
        assert_eq!(&event[32..33], b" ");
    }

    #[test]
    fn text_field_events_use_koto_ui_utf8_editing_and_cursor_boundaries() {
        let mut session = UiSession::new();
        session.mount(&ime_fixture()).unwrap();
        session.complete_present();
        let config = crate::ConfigService::new().snapshot();
        let mut event = [0u8; 64];
        session.frame_begin(
            VmInputSnapshot {
                pressed_bits: 1 << 4,
                intent_bits: crate::runtime::text_intent::NEWLINE,
                ..VmInputSnapshot::empty()
            },
            config,
        );
        assert_eq!(session.editing_id(), Some(2));
        assert_eq!(session.poll_event(&mut event), Ok(0));
        for character in ['日', 'A'] {
            session.frame_begin(
                VmInputSnapshot {
                    text_codepoint: character as u32,
                    ..VmInputSnapshot::empty()
                },
                config,
            );
        }
        assert_eq!(session.poll_event(&mut event), Ok(35));
        assert_eq!(&event[32..35], "日".as_bytes());
        assert_eq!(session.poll_event(&mut event), Ok(36));
        assert_eq!(&event[32..36], "日A".as_bytes());

        session.frame_begin(
            VmInputSnapshot {
                intent_bits: crate::runtime::text_intent::LEFT,
                ..VmInputSnapshot::empty()
            },
            config,
        );
        assert_eq!(session.focused_id(), Some(2));
        assert_eq!(session.editing_id(), Some(2));
        session.frame_begin(
            VmInputSnapshot {
                intent_bits: crate::runtime::text_intent::BACKSPACE,
                ..VmInputSnapshot::empty()
            },
            config,
        );
        assert_eq!(session.poll_event(&mut event), Ok(33));
        assert_eq!(&event[32..33], b"A");
        assert_eq!(i32_at(&event, 20), 0);
    }

    #[test]
    fn frame_input_event_is_pollable_with_zero_additional_frame_latency() {
        let mut session = UiSession::new();
        session.mount(&fixture()).unwrap();
        session.complete_present();
        session.frame_begin(
            VmInputSnapshot {
                pressed_bits: 1 << 4,
                ..VmInputSnapshot::empty()
            },
            crate::ConfigService::new().snapshot(),
        );

        let mut short = [0xaa; 31];
        assert_eq!(session.poll_event(&mut short), Err(UiPollError::NoMemory));
        assert_eq!(short, [0xaa; 31]);

        let mut event = [0u8; 64];
        assert_eq!(session.poll_event(&mut event), Ok(32));
        assert_eq!(&event[..4], b"KUE1");
        assert_eq!(event[12], 1);
        assert_eq!(u16_at(&event, 14), 2);
        assert_eq!(session.poll_event(&mut event), Ok(0));
        assert_eq!(session.damaged_rects().count(), 0);
    }

    #[test]
    fn event_overflow_preserves_queue_then_reports_cumulative_drop() {
        let mut session = UiSession::new();
        session.mount(&fixture()).unwrap();
        let config = crate::ConfigService::new().snapshot();
        for _ in 0..10 {
            session.frame_begin(
                VmInputSnapshot {
                    pressed_bits: 1 << 4,
                    ..VmInputSnapshot::empty()
                },
                config,
            );
        }
        let mut event = [0u8; 64];
        for _ in 0..UI_EVENT_QUEUE_CAPACITY {
            assert_eq!(session.poll_event(&mut event), Ok(32));
            assert_eq!(event[12], 1);
        }
        assert_eq!(session.poll_event(&mut event), Ok(32));
        assert_eq!(event[12], 9);
        assert_eq!(i32_at(&event, 16), 0);
        assert_eq!(i32_at(&event, 20), 2);
        assert_eq!(session.poll_event(&mut event), Ok(0));
    }

    #[test]
    fn locale_change_is_deduplicated_to_the_latest_snapshot() {
        let mut session = UiSession::new();
        session.mount(&fixture()).unwrap();
        let mut config = crate::ConfigService::new();
        session.frame_begin(VmInputSnapshot::empty(), config.snapshot());
        assert!(config.set_locale(crate::Locale::JaJp));
        session.frame_begin(VmInputSnapshot::empty(), config.snapshot());
        assert!(config.set_locale(crate::Locale::QpsPloc));
        session.frame_begin(VmInputSnapshot::empty(), config.snapshot());

        let mut event = [0u8; 64];
        assert_eq!(session.poll_event(&mut event), Ok(40));
        assert_eq!(event[12], 10);
        assert_eq!(i32_at(&event, 16), config.locale_generation() as i32);
        assert_eq!(&event[32..40], b"qps-ploc");
        assert_eq!(session.poll_event(&mut event), Ok(0));
    }

    #[test]
    fn ime_snapshot_is_owned_damages_and_queues_normal_text_change() {
        let mut session = UiSession::new();
        session.mount(&ime_fixture()).unwrap();
        assert!(session.ime_target().is_none());
        assert!(session.begin_text_editing());
        let target = session.ime_target().unwrap();
        assert_eq!((target.widget_id, target.value, target.cursor), (2, "", 0));
        session.clear_damage();

        session
            .apply_ime_snapshot(2, "か", "か".len(), "n", Some("名"))
            .unwrap();
        assert_eq!(session.ime_target().unwrap().value, "か");
        assert_eq!(session.damaged_rects().count(), 1);

        let mut event = [0u8; 64];
        assert_eq!(session.poll_event(&mut event), Ok(35));
        assert_eq!(event[12], 3);
        assert_eq!(&event[32..35], "か".as_bytes());
        session.reset();
        assert!(session.ime_target().is_none());
        assert_eq!(session.poll_event(&mut event), Err(UiPollError::NotMounted));
    }

    #[test]
    fn ime_composition_clears_when_focus_moves_to_another_control() {
        let mut session = UiSession::new();
        session.mount(&ime_focus_fixture()).unwrap();
        assert!(session.begin_text_editing());
        session.apply_ime_snapshot(2, "", 0, "k", None).unwrap();
        assert_eq!(session.ime_owner_id, 2);

        session.update(&update_packet(3, 10, &[], [0; 4])).unwrap();
        assert_eq!(session.focused_id(), Some(3));
        assert_eq!(session.ime_owner_id, NONE_ID);
        assert_eq!(session.ime_text_len, 0);
        assert!(session.ime_target().is_none());
    }
}
