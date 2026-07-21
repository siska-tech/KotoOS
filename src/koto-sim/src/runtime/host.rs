use super::*;

use crate::audio::{RUNTIME_BGM_EVENTS_PER_TRACK, RUNTIME_SFX_EVENTS_PER_TRACK};
use koto_audio::RuntimeCue;
use koto_core::JsonHostSession;

use super::fake_fetch::FakeFetchBackend;
use super::fake_mqtt::{ScriptedMessage, SimMqttBackend, SimMqttTerminal};
use super::sim_vault::SimVault;

#[derive(Debug)]
pub(super) struct SkkSession {
    dict: Vec<u8>,
    index: SkkLeadingIndex,
    /// Scan buffer for the windowed lookup path. The host could pass the whole
    /// `dict` slice, but converting through `WindowedDict` keeps the sim on the
    /// exact code path the firmware's SD reader uses (hardware parity).
    window: [u8; SKK_LOOKUP_WINDOW_BYTES],
}

#[derive(Debug)]
pub(super) struct SimRuntimeHost {
    pub(super) fs: HostFs,
    pub(super) sandbox: Sandbox,
    /// Copied system configuration view exposed through KUC1. The host does not
    /// retain a reference to mutable configuration storage.
    config_snapshot: ConfigSnapshot,
    ui_session: UiSession,
    pub(super) ui_ime_owner: u16,
    pub(super) draw_rects: Vec<(i32, i32, i32, i32, i32)>,
    /// Recorded `draw_pixels` blits: `(x, y, w, h, little-endian RGB565 bytes)`.
    pub(super) draw_pixels: Vec<(i32, i32, i32, i32, Vec<u8>)>,
    /// Session-persistent 320x320 RGB565 LCD-GRAM image used by streamed art.
    pub(super) persistent_pixels: Vec<u8>,
    pub(super) text: Vec<(i32, i32, String)>,
    /// Colour for each `text` entry, index-aligned. [`TEXT_COLOR_DEFAULT`] marks a
    /// colourless `draw_text`; any other value is the RGB565 colour (held as a
    /// sign-extended `i16`, recovered with `as u16` at paint time).
    pub(super) text_colors: Vec<i32>,
    pub(super) files: Vec<Option<HostFile>>,
    pub(super) editor: SimMemoEditor,
    pub(super) ime: KotoMemoIme,
    pub(super) skk: Option<SkkSession>,
    /// Shared host audio engine: BGM/SFX triggers and submitted PCM feed it, and the
    /// cpal callback (window mode) or the headless capture path renders from it.
    pub(super) audio: Arc<Mutex<SimAudio>>,
    /// Read-only package asset paths declared by the launching manifest.
    pub(super) asset_paths: Vec<String>,
    /// The authoritative KPA1 archive for this session. Tests for the legacy
    /// loose-file host seam may leave this unset.
    pub(super) package_archive: Option<Arc<Vec<u8>>>,
    /// Deterministic app-facing Fetch service. It never touches the host
    /// network; the active manifest allowlist is copied at launch.
    fetch: AppFetchService<FakeFetchBackend>,
    fetch_allowlist: FetchAllowlist,
    fetch_app: AppContext,
    fetch_now_ms: u64,
    fetch_poll_cache: Option<(i32, u64, i32, i32)>,
    /// Scripted advisory UTC for the `time_query` host call (KOTO-0247): the
    /// synchronized UTC seconds at frame-clock zero, or `None` while the
    /// scenario runs "time unknown". The clock advances with the same
    /// deterministic per-frame tick as `fetch_now_ms`, never host wall time.
    time_utc_base: Option<i64>,
    /// Host-owned bounded JSON decoder session behind the `json_*` host calls
    /// (KOTO-0246). The same `koto_core::JsonHostSession` backs the device
    /// host, so both runtimes expose byte-identical ABI behavior.
    json: JsonHostSession,
    /// Deterministic application credential vault behind the `vault_handle` /
    /// `fetch_start_authenticated` host calls (KOTO-0248). Synthetic handles and
    /// fake secrets; it never reads any host credential store.
    vault: SimVault,
    /// Deterministic app-facing MQTT subscribe service (KOTO-0249). It never
    /// touches the host network; the manifest broker/topic allowlists are copied
    /// at launch and it shares the deterministic `fetch_now_ms` frame clock.
    mqtt: AppMqttService<SimMqttBackend>,
    mqtt_brokers: BrokerAllowlist,
    mqtt_topics: TopicFilterSet,
    mqtt_app: AppContext,
    /// Audio actions issued by the VM this frame, for deterministic inspection
    /// (mirrors the recorded draw lists).
    pub(super) audio_events: Vec<AudioEvent>,
    /// Retained Game2D board tilemap layer (KOTO-0135): one `tile_ref` per cell,
    /// `-1` empty. Unlike the per-frame draw lists this persists across frames;
    /// `game2d_present` re-emits its non-empty cells into `draw_pixels`.
    pub(super) tilemap: SimTilemap,
    /// Retained Game2D static/background command layer (KOTO-0136): draw calls
    /// captured between `game2d_static_begin` and `game2d_static_end`. Like the
    /// tilemap these persist across the per-frame draw clear; the paint pipeline
    /// composites them *beneath* the immediate lists (page/well/grid/panel chrome
    /// the app no longer re-emits every frame). Parallel to the immediate lists:
    /// rects, pixel blits, and text (+per-text colour).
    pub(super) static_rects: Vec<(i32, i32, i32, i32, i32)>,
    pub(super) static_pixels: Vec<(i32, i32, i32, i32, Vec<u8>)>,
    pub(super) static_text: Vec<(i32, i32, String)>,
    pub(super) static_text_colors: Vec<i32>,
    /// Retained command image produced by `ui_present`; replaced atomically on
    /// UI damage and preserved across per-frame immediate-list clears.
    pub(super) ui_rects: Vec<(i32, i32, i32, i32, i32)>,
    pub(super) ui_text: Vec<(i32, i32, String)>,
    pub(super) ui_text_colors: Vec<i32>,
    pub(super) ui_text_layouts: Vec<SimUiTextLayout>,
    pub(super) ui_commands: Vec<SimUiCommand>,
    /// Per-frame retained-UI trace. These are observations only; the command
    /// image above remains session-persistent while these vectors are cleared.
    pub(super) ui_present_calls: usize,
    pub(super) ui_paint_count: usize,
    pub(super) ui_presented_damage: Vec<(i32, i32, i32, i32)>,
    pub(super) ui_polled_events: Vec<(u8, u16, i32, i32)>,
    /// While `true`, draw calls are captured into the static layer above instead
    /// of the per-frame immediate lists.
    pub(super) capturing_static: bool,
    /// Retained Game2D sprite/stamp layer (KOTO-0140). Stamps are reusable cell
    /// patterns (defined once, session-stable); sprites are retained placed
    /// instances diffed by stable `inst_id`. Both persist across the per-frame
    /// draw clear; `game2d_present` re-emits every visible sprite's cells into
    /// `draw_pixels` (after the board cells, before text) so the existing paint
    /// pipeline composites them in the fixed sprite z-order.
    pub(super) stamps: [Option<Game2dStamp>; GAME2D_MAX_STAMPS],
    pub(super) sprites: [Option<Game2dSprite>; GAME2D_MAX_SPRITES],
    /// Retained Game2D text layer (KOTO-0141). Each item is a string pinned at a
    /// pixel position with a colour, keyed by a stable `id` (the array index).
    /// Like the sprite/tilemap layers these persist across the per-frame draw clear
    /// and are composited in fixed z-order *above* the sprite layer and *below* the
    /// per-frame immediate text (see `render::paint_app_session`). `None` is an
    /// empty/hidden slot.
    pub(super) text_items: [Option<Game2dText>; GAME2D_MAX_TEXT_ITEMS],
}

/// A retained sprite stamp: `count` cells at app-heap byte offset `cells_off`.
/// v1 supports only `format 0` (packed `(dcol,drow)` nibbles, the KOTO-0138
/// layout), validated at define time, so the format is not stored.
#[derive(Clone, Copy, Debug)]
pub(super) struct Game2dStamp {
    cells_off: u32,
    count: u8,
}

/// A retained placed sprite: an instance of `stamp_id` at pixel `(x, y)` drawing
/// the 16x16 tile at app-heap byte offset `tile_ref`.
#[derive(Clone, Copy, Debug)]
pub(super) struct Game2dSprite {
    stamp_id: u8,
    x: i32,
    y: i32,
    tile_ref: i32,
}

/// A retained text item (KOTO-0141): a string pinned at pixel `(x, y)` with a
/// colour. `rgb565` matches the per-text colour convention of the immediate text
/// lists ([`TEXT_COLOR_DEFAULT`] would mark a colourless draw; `game2d_text_set`
/// always carries an explicit colour).
#[derive(Clone, Debug)]
pub(super) struct Game2dText {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) rgb565: i32,
    pub(super) text: String,
}

/// KotoBlocks board geometry the Game2D tilemap layer composites against
/// by default; KOTO-0199 permits any active shape up to 20x20 and any i16 origin.
pub(super) const GAME2D_TILEMAP_MAX_COLS: usize = 20;
pub(super) const GAME2D_TILEMAP_MAX_ROWS: usize = 20;
pub(super) const GAME2D_TILEMAP_MAX_CELLS: usize =
    GAME2D_TILEMAP_MAX_COLS * GAME2D_TILEMAP_MAX_ROWS;
const GAME2D_TILE: i32 = 16;
/// Bytes of one 16x16 little-endian RGB565 tile.
const GAME2D_TILE_BYTES: usize = (GAME2D_TILE * GAME2D_TILE) as usize * 2;

#[derive(Debug)]
pub(super) struct SimTilemap {
    cells: [i32; GAME2D_TILEMAP_MAX_CELLS],
    columns: usize,
    rows: usize,
    origin_x: i16,
    origin_y: i16,
}

impl SimTilemap {
    fn legacy() -> Self {
        Self {
            cells: [-1; GAME2D_TILEMAP_MAX_CELLS],
            columns: 10,
            rows: 20,
            origin_x: 8,
            origin_y: 0,
        }
    }

    fn index(column: usize, row: usize) -> usize {
        row * GAME2D_TILEMAP_MAX_COLS + column
    }
}
/// Retained sprite/stamp table sizes (KOTO-0140), mirroring the device budget in
/// `koto-pico` config. 32 stamps cover KotoBlocks' 28 piece orientations with
/// headroom; 16 sprites cover its active/ghost/NEXTx3/HOLD instances.
pub(super) const GAME2D_MAX_STAMPS: usize = 32;
pub(super) const GAME2D_MAX_SPRITES: usize = 16;
/// Retained text-item table size (KOTO-0141), mirroring the device budget in
/// `koto-pico` config. 12 items cover KotoBlocks' status text (badge, score,
/// level, lines, hold hint) with headroom.
pub(super) const GAME2D_MAX_TEXT_ITEMS: usize = 12;

impl SimRuntimeHost {
    #[cfg(test)]
    pub(super) fn new(fs: HostFs, app_id: &str) -> Result<Self, SimError> {
        Self::with_audio_and_assets(
            fs,
            app_id,
            Arc::new(Mutex::new(SimAudio::new(DEFAULT_SAMPLE_RATE))),
            Vec::new(),
        )
    }

    pub(super) fn with_audio_and_assets(
        fs: HostFs,
        app_id: &str,
        audio: Arc<Mutex<SimAudio>>,
        asset_paths: Vec<String>,
    ) -> Result<Self, SimError> {
        let mut fs = fs;
        let skk = load_skk_session(&mut fs);
        Ok(Self {
            fs,
            sandbox: Sandbox::new(app_id).map_err(|_| SimError::RuntimeExecutionFailed)?,
            config_snapshot: ConfigService::new().snapshot(),
            ui_session: UiSession::new(),
            ui_ime_owner: u16::MAX,
            draw_rects: Vec::new(),
            draw_pixels: Vec::new(),
            persistent_pixels: vec![0; 320 * 320 * 2],
            text: Vec::new(),
            text_colors: Vec::new(),
            files: Vec::new(),
            editor: text_editor(40, 20).ok_or(SimError::RuntimeExecutionFailed)?,
            ime: KotoMemoIme::new(),
            skk,
            audio,
            asset_paths,
            package_archive: None,
            // The scripted body exercises the KOTO-0246 sample's selection
            // path: an unknown nested object/array to skip, a null, and the
            // two named fields the app extracts.
            fetch: AppFetchService::new(FakeFetchBackend::response(
                200,
                br#"{"station":{"id":"KOTO-1","samples":[3,7]},"location":"Tokyo","wind":null,"temperature_c":21,"ok":true}"#,
                2,
            )),
            fetch_allowlist: FetchAllowlist::empty(),
            fetch_app: AppContext {
                app_id: stable_app_id(app_id),
                generation: 1,
            },
            fetch_now_ms: 0,
            fetch_poll_cache: None,
            time_utc_base: None,
            json: JsonHostSession::new(),
            vault: SimVault::seeded(app_id),
            // Default deterministic telemetry: a retained value delivered first,
            // then two live samples on the sample app's topic. Scenario tests and
            // the launch path re-script this without a host network.
            mqtt: AppMqttService::new(SimMqttBackend::scenario(
                1,
                vec![
                    ScriptedMessage::new(b"sensors/room/temp", b"21.4", true),
                    ScriptedMessage::new(b"sensors/room/temp", b"21.7", false),
                    ScriptedMessage::new(b"sensors/room/temp", b"22.1", false),
                ],
                SimMqttTerminal::Idle,
            )),
            mqtt_brokers: BrokerAllowlist::empty(),
            mqtt_topics: TopicFilterSet::empty(),
            mqtt_app: AppContext {
                app_id: stable_app_id(app_id),
                generation: 1,
            },
            audio_events: Vec::new(),
            tilemap: SimTilemap::legacy(),
            static_rects: Vec::new(),
            static_pixels: Vec::new(),
            static_text: Vec::new(),
            static_text_colors: Vec::new(),
            ui_rects: Vec::new(),
            ui_text: Vec::new(),
            ui_text_colors: Vec::new(),
            ui_text_layouts: Vec::new(),
            ui_commands: Vec::new(),
            ui_present_calls: 0,
            ui_paint_count: 0,
            ui_presented_damage: Vec::new(),
            ui_polled_events: Vec::new(),
            capturing_static: false,
            stamps: [None; GAME2D_MAX_STAMPS],
            sprites: [None; GAME2D_MAX_SPRITES],
            text_items: core::array::from_fn(|_| None),
        })
    }

    pub(super) fn with_audio_and_package(
        fs: HostFs,
        app_id: &str,
        audio: Arc<Mutex<SimAudio>>,
        package_archive: Arc<Vec<u8>>,
    ) -> Result<Self, SimError> {
        let reader =
            KpaReader::new(package_archive.as_slice()).map_err(|_| SimError::InvalidManifest)?;
        let mut asset_paths = Vec::new();
        for index in 0..reader.entry_count() {
            asset_paths.push(
                reader
                    .entry(index)
                    .map_err(|_| SimError::InvalidManifest)?
                    .path
                    .to_string(),
            );
        }
        let mut host = Self::with_audio_and_assets(fs, app_id, audio, asset_paths)?;
        host.package_archive = Some(package_archive);
        Ok(host)
    }

    pub(super) fn clear_frame_draw(&mut self) {
        self.draw_rects.clear();
        self.draw_pixels.clear();
        self.text.clear();
        self.text_colors.clear();
        self.audio_events.clear();
        self.ui_present_calls = 0;
        self.ui_paint_count = 0;
        self.ui_presented_damage.clear();
        self.ui_polled_events.clear();
    }

    pub(super) fn set_config_snapshot(&mut self, snapshot: ConfigSnapshot) {
        self.config_snapshot = snapshot;
    }

    pub(super) fn set_fetch_allowlist(&mut self, allowlist: FetchAllowlist) {
        self.fetch.teardown();
        self.fetch_allowlist = allowlist;
        self.fetch_app.generation = self.fetch_app.generation.wrapping_add(1).max(1);
        self.fetch_poll_cache = None;
    }

    /// Install the manifest MQTT broker/topic allowlists at launch (KOTO-0249).
    /// Tears down any live session and re-generations the app context so no
    /// handle survives the change.
    pub(super) fn set_mqtt_permission(&mut self, brokers: BrokerAllowlist, topics: TopicFilterSet) {
        self.mqtt.teardown();
        self.mqtt_brokers = brokers;
        self.mqtt_topics = topics;
        self.mqtt_app.generation = self.mqtt_app.generation.wrapping_add(1).max(1);
    }

    /// Script the deterministic MQTT broker as offline, so `mqtt_connect` is
    /// refused with a stable `Unavailable` (KOTO-0249). Tears down any live
    /// session so the change starts clean.
    pub(super) fn script_mqtt_offline(&mut self) {
        self.mqtt.teardown();
        *self.mqtt.backend_mut() = SimMqttBackend::offline();
        self.mqtt_app.generation = self.mqtt_app.generation.wrapping_add(1).max(1);
    }

    /// Re-script the deterministic fetch backend (KOTO-0247 weather scenarios).
    /// The live request is torn down and the app generation bumped so a new
    /// `fetch_start` sees the freshly scripted response, failure, or offline
    /// state. `pending_polls` is how many `Pending` polls precede headers.
    pub(super) fn script_fetch_response(&mut self, status: u16, body: &[u8], pending_polls: u8) {
        self.fetch.teardown();
        self.fetch
            .backend_mut()
            .configure_response(status, body, pending_polls);
        self.fetch_app.generation = self.fetch_app.generation.wrapping_add(1).max(1);
        self.fetch_poll_cache = None;
    }

    pub(super) fn script_fetch_failure(&mut self, error: FetchError, pending_polls: u8) {
        self.fetch.teardown();
        self.fetch
            .backend_mut()
            .configure_failure(error, pending_polls);
        self.fetch_app.generation = self.fetch_app.generation.wrapping_add(1).max(1);
        self.fetch_poll_cache = None;
    }

    pub(super) fn script_fetch_offline(&mut self) {
        self.fetch.teardown();
        self.fetch.backend_mut().configure_offline();
        self.fetch_app.generation = self.fetch_app.generation.wrapping_add(1).max(1);
        self.fetch_poll_cache = None;
    }

    /// Script the advisory clock: `Some(utc_seconds)` anchors synchronized UTC
    /// at the current frame clock; `None` reverts to the unknown-time state.
    pub(super) fn set_time_utc(&mut self, utc_seconds: Option<i64>) {
        self.time_utc_base =
            utc_seconds.map(|seconds| seconds - (self.fetch_now_ms / 1_000) as i64);
    }

    pub(super) fn ui_text(&self, widget_id: u16) -> Option<&str> {
        let node = self
            .ui_session
            .nodes()
            .iter()
            .find(|node| node.id == widget_id)?;
        Some(self.ui_session.text(node))
    }

    pub(super) fn ui_focused_id(&self) -> Option<u16> {
        self.ui_session.focused_id()
    }

    pub(super) fn ui_render_command_count(&self) -> usize {
        self.ui_rects.len() + self.ui_text.len()
    }

    pub(super) fn ui_value(&self, widget_id: u16) -> Option<&str> {
        let node = self
            .ui_session
            .nodes()
            .iter()
            .find(|node| node.id == widget_id)?;
        core::str::from_utf8(self.ui_session.value(node)).ok()
    }

    /// Queue normalized UI input without resuming bytecode. Deterministic
    /// capacity tests use this to exercise the bounded KUE1 queue exactly as a
    /// burst-capable platform input adapter would.
    pub(super) fn inject_ui_input(&mut self, input: VmInputSnapshot) {
        <Self as VmHost>::ui_frame_begin(self, input);
    }

    fn sandbox_path(&self, path: &str) -> Result<String, HostErrorKind> {
        let path = self
            .sandbox
            .resolve(path)
            .map_err(|_| HostErrorKind::PermissionDenied)?;
        Ok(format!("data/{}/{}", self.sandbox.app_id(), path.as_str()))
    }

    /// Sorted filenames in the app's save-data sandbox directory. A missing
    /// directory (app has never saved) is an empty listing. Names are the
    /// sandbox-relative basenames only; host paths never cross the boundary.
    fn sandbox_entry_names(&self) -> Vec<String> {
        let dir = format!("{SAVE_DATA_ROOT}/{}", self.sandbox.app_id());
        let mut names: Vec<String> = match self.fs.read_dir(&dir) {
            Ok(entries) => entries
                .iter()
                .filter_map(|entry| entry.virtual_path().rsplit('/').next())
                .map(str::to_string)
                .collect(),
            Err(_) => Vec::new(),
        };
        names.sort();
        names
    }

    fn allocate_handle(&mut self, file: HostFile) -> HostCallOutcome {
        if let Some((index, slot)) = self
            .files
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.is_none())
        {
            *slot = Some(file);
            return HostCallOutcome::Ok1(index as i32);
        }
        if self.files.len() >= SIM_MAX_OPEN_FILES {
            return HostCallOutcome::Err(koto_core::HostErrorCode::NO_MEMORY);
        }
        self.files.push(Some(file));
        HostCallOutcome::Ok1((self.files.len() - 1) as i32)
    }

    /// The number of currently open sandboxed file handles. Reports occupancy
    /// only — handles index the per-app sandbox, never host paths.
    pub(super) fn open_file_count(&self) -> usize {
        self.files.iter().filter(|slot| slot.is_some()).count()
    }

    fn file_mut(&mut self, handle: i32) -> Result<&mut HostFile, HostErrorKind> {
        let handle = usize::try_from(handle).map_err(|_| HostErrorKind::BadArgument)?;
        self.files
            .get_mut(handle)
            .and_then(Option::as_mut)
            .ok_or(HostErrorKind::BadArgument)
    }

    /// Read a read-only package asset fully into memory, capped at `cap` bytes.
    fn read_asset_bytes(&mut self, path: &str, cap: usize) -> Result<Vec<u8>, HostCallOutcome> {
        if let Some(archive) = &self.package_archive {
            let reader = KpaReader::new(archive.as_slice())
                .map_err(|_| HostCallOutcome::Err(koto_core::HostErrorCode::IO_ERROR))?;
            let bytes = reader
                .payload_for(path)
                .map_err(|_| HostCallOutcome::Err(koto_core::HostErrorCode::IO_ERROR))?
                .ok_or(HostCallOutcome::Err(
                    koto_core::HostErrorCode::PERMISSION_DENIED,
                ))?;
            if bytes.len() > cap {
                return Err(HostCallOutcome::Err(koto_core::HostErrorCode::NO_MEMORY));
            }
            return Ok(bytes.to_vec());
        }
        let mut file = match self.fs.open(path, FileMode::Read) {
            Ok(file) => file,
            Err(koto_core::HalError::InvalidArgument) => {
                return Err(HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT))
            }
            Err(_) => return Err(HostCallOutcome::Err(koto_core::HostErrorCode::IO_ERROR)),
        };
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 256];
        loop {
            let len = match file.read(&mut chunk) {
                Ok(len) => len,
                Err(_) => return Err(HostCallOutcome::Err(koto_core::HostErrorCode::IO_ERROR)),
            };
            if len == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..len]);
            if bytes.len() > cap {
                return Err(HostCallOutcome::Err(koto_core::HostErrorCode::NO_MEMORY));
            }
        }
        Ok(bytes)
    }

    fn present_ui(&mut self) -> HostCallOutcome {
        let damage: Vec<_> = self
            .ui_session
            .damaged_rects()
            .map(|rect| (rect.x, rect.y, rect.w, rect.h))
            .collect();
        let mut rects = Vec::new();
        let mut text = Vec::new();
        let mut colors = Vec::new();
        let mut layouts = Vec::new();
        let mut commands = Vec::new();
        let result = {
            let mut painter = SimUiPainter {
                rects: &mut rects,
                text: &mut text,
                colors: &mut colors,
                layouts: &mut layouts,
                commands: &mut commands,
            };
            self.ui_session
                .paint_full_if_damaged(&mut painter, &koto_ui::Theme::DARK)
        };
        match result {
            Ok(false) => HostCallOutcome::Ok0,
            Ok(true) => {
                self.ui_paint_count += 1;
                self.ui_presented_damage.extend(damage);
                self.ui_rects = rects;
                self.ui_text = text;
                self.ui_text_colors = colors;
                self.ui_text_layouts = layouts;
                self.ui_commands = commands;
                self.ui_session.complete_present();
                HostCallOutcome::Ok0
            }
            Err(_) => HostCallOutcome::Err(koto_core::HostErrorCode::NO_MEMORY),
        }
    }
}

impl VmHost for SimRuntimeHost {
    fn ui_frame_begin(&mut self, input: VmInputSnapshot) {
        self.fetch_now_ms = self.fetch_now_ms.saturating_add(16);
        self.fetch_poll_cache = None;
        let Some(target) = self.ui_session.ime_target() else {
            if self.ui_ime_owner != u16::MAX {
                let _ = self.ime.process_key(MemoImeKey::Cancel, &mut self.editor);
                self.ui_session.clear_ime_composition();
                self.ui_ime_owner = u16::MAX;
            }
            self.ui_session.frame_begin(input, self.config_snapshot);
            return;
        };
        let widget_id = target.widget_id;
        let cursor = target.cursor;
        let mut initial = [0u8; 256];
        let initial_len = target.value.len();
        initial[..initial_len].copy_from_slice(target.value.as_bytes());
        if self.ui_ime_owner != widget_id {
            let _ = self.ime.process_key(MemoImeKey::Cancel, &mut self.editor);
            self.ui_session.clear_ime_composition();
            self.ui_ime_owner = widget_id;
        }
        let initial_text = core::str::from_utf8(&initial[..initial_len]).unwrap_or("");
        let _ = self.editor.load_str(initial_text);
        let _ = self.editor.set_cursor(cursor);

        let mut forwarded = input;
        let intents = input.intent_bits;
        if intents & koto_core::runtime::text_intent::IME_TOGGLE != 0 {
            let _ = self.ime.process_key(MemoImeKey::Toggle, &mut self.editor);
            forwarded.intent_bits &= !koto_core::runtime::text_intent::IME_TOGGLE;
        }
        if intents & koto_core::runtime::text_intent::SHIFT != 0 {
            let _ = self.ime.process_key(MemoImeKey::Shift, &mut self.editor);
            forwarded.intent_bits &= !koto_core::runtime::text_intent::SHIFT;
        }
        if intents & koto_core::runtime::text_intent::NEWLINE != 0
            && self.ime.line().mode != MemoImeMode::Empty
        {
            let _ = self.ime.process_key(MemoImeKey::Commit, &mut self.editor);
            forwarded.intent_bits &= !koto_core::runtime::text_intent::NEWLINE;
            forwarded.pressed_bits &= !(1 << 4);
        }
        if intents & koto_core::runtime::text_intent::CONVERT != 0 {
            let _ = self.ime_convert();
            forwarded.intent_bits &= !koto_core::runtime::text_intent::CONVERT;
        }
        if intents & koto_core::runtime::text_intent::COMMIT != 0 {
            let _ = self.ime.process_key(MemoImeKey::Commit, &mut self.editor);
            forwarded.intent_bits &= !koto_core::runtime::text_intent::COMMIT;
        }
        if intents & koto_core::runtime::text_intent::CANCEL != 0
            && self.ime.line().mode != MemoImeMode::Empty
        {
            let _ = self.ime.process_key(MemoImeKey::Cancel, &mut self.editor);
            forwarded.intent_bits &= !koto_core::runtime::text_intent::CANCEL;
            forwarded.pressed_bits &= !(1 << 5);
            forwarded.text_codepoint = 0;
        }
        if intents & koto_core::runtime::text_intent::BACKSPACE != 0
            && self.ime.line().mode != MemoImeMode::Empty
        {
            let _ = self
                .ime
                .process_key(MemoImeKey::Backspace, &mut self.editor);
            forwarded.intent_bits &= !koto_core::runtime::text_intent::BACKSPACE;
        }
        if let Some(ch) = char::from_u32(forwarded.text_codepoint).filter(|ch| *ch != '\0') {
            let _ = self
                .ime
                .process_key(MemoImeKey::Character(ch), &mut self.editor);
            forwarded.text_codepoint = 0;
        }

        let mut value = [0u8; 256];
        let value_len = self.editor.as_str().len().min(value.len());
        value[..value_len].copy_from_slice(&self.editor.as_str().as_bytes()[..value_len]);
        let mut composition = [0u8; 128];
        let mut composition_len = 0usize;
        let mut candidate = [0u8; 64];
        let mut candidate_len = 0usize;
        let line = self.ime.line();
        for part in [line.reading, line.pending_romaji] {
            let len = part.len().min(composition.len() - composition_len);
            composition[composition_len..composition_len + len]
                .copy_from_slice(&part.as_bytes()[..len]);
            composition_len += len;
        }
        if let Some(text) = line.candidate {
            candidate_len = text.len().min(candidate.len());
            candidate[..candidate_len].copy_from_slice(&text.as_bytes()[..candidate_len]);
        }
        let value = core::str::from_utf8(&value[..value_len]).unwrap_or("");
        let composition = core::str::from_utf8(&composition[..composition_len]).unwrap_or("");
        let candidate = (candidate_len != 0)
            .then(|| core::str::from_utf8(&candidate[..candidate_len]).unwrap_or(""));
        let value_changed = initial_text != value;
        let ime_changed = self
            .ui_session
            .apply_ime_snapshot(
                widget_id,
                value,
                self.editor.cursor(),
                composition,
                candidate,
            )
            .unwrap_or(false);
        self.ui_session.frame_begin(forwarded, self.config_snapshot);
        // Composition-only changes do not create an App semantic event, so the
        // host presents their retained TextField damage without requiring an
        // otherwise-idle `ui_present` call from bytecode.
        if ime_changed && !value_changed {
            let _ = self.present_ui();
        }
    }

    fn ui_capabilities(&mut self, dst: &mut [u8]) -> HostCallOutcome {
        match UiCapabilities::from_config(self.config_snapshot).encode(dst) {
            Ok(written) => HostCallOutcome::Ok1(written as i32),
            Err(_) => HostCallOutcome::Err(koto_core::HostErrorCode::NO_MEMORY),
        }
    }

    fn ui_mount(&mut self, src: &[u8]) -> HostCallOutcome {
        match self.ui_session.mount(src) {
            Ok(()) => {
                self.ime = KotoMemoIme::new();
                self.ui_ime_owner = u16::MAX;
                HostCallOutcome::Ok0
            }
            Err(UiMountError::BadArgument) => {
                HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT)
            }
            Err(UiMountError::Unsupported) => {
                HostCallOutcome::Err(koto_core::HostErrorCode::UNSUPPORTED)
            }
            Err(UiMountError::NoMemory) => {
                HostCallOutcome::Err(koto_core::HostErrorCode::NO_MEMORY)
            }
        }
    }

    fn ui_update(&mut self, src: &[u8]) -> HostCallOutcome {
        if !self.ui_session.is_mounted() {
            return HostCallOutcome::Err(koto_core::HostErrorCode::NOT_FOUND);
        }
        match self.ui_session.update(src) {
            Ok(()) => HostCallOutcome::Ok0,
            Err(UiMountError::BadArgument) => {
                HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT)
            }
            Err(UiMountError::Unsupported) => {
                HostCallOutcome::Err(koto_core::HostErrorCode::UNSUPPORTED)
            }
            Err(UiMountError::NoMemory) => {
                HostCallOutcome::Err(koto_core::HostErrorCode::NO_MEMORY)
            }
        }
    }

    fn ui_present(&mut self) -> HostCallOutcome {
        if !self.ui_session.is_mounted() {
            return HostCallOutcome::Err(koto_core::HostErrorCode::NOT_FOUND);
        }
        self.ui_present_calls += 1;
        self.present_ui()
    }

    fn ui_poll_event(&mut self, dst: &mut [u8]) -> HostCallOutcome {
        match self.ui_session.poll_event(dst) {
            Ok(written) => {
                if written >= 32 {
                    self.ui_polled_events.push((
                        dst[12],
                        u16::from_le_bytes([dst[14], dst[15]]),
                        i32::from_le_bytes([dst[16], dst[17], dst[18], dst[19]]),
                        i32::from_le_bytes([dst[20], dst[21], dst[22], dst[23]]),
                    ));
                }
                HostCallOutcome::Ok1(written as i32)
            }
            Err(UiPollError::NotMounted) => {
                HostCallOutcome::Err(koto_core::HostErrorCode::NOT_FOUND)
            }
            Err(UiPollError::NoMemory) => HostCallOutcome::Err(koto_core::HostErrorCode::NO_MEMORY),
        }
    }

    fn ui_reset(&mut self) -> HostCallOutcome {
        if self.ui_ime_owner != u16::MAX {
            self.ime = KotoMemoIme::new();
        }
        self.ui_ime_owner = u16::MAX;
        self.ui_session.reset();
        self.ui_rects.clear();
        self.ui_text.clear();
        self.ui_text_colors.clear();
        self.ui_text_layouts.clear();
        self.ui_commands.clear();
        HostCallOutcome::Ok0
    }

    fn ui_session_end(&mut self) {
        let _ = self.ui_reset();
    }

    fn draw_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgb565: i32) -> HostCallOutcome {
        if self.capturing_static {
            self.static_rects.push((x, y, w, h, rgb565));
        } else {
            self.draw_rects.push((x, y, w, h, rgb565));
        }
        HostCallOutcome::Ok0
    }

    fn draw_pixels_rgb565(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        pixels: &[u8],
    ) -> HostCallOutcome {
        if self.capturing_static {
            self.static_pixels.push((x, y, w, h, pixels.to_vec()));
        } else {
            self.draw_pixels.push((x, y, w, h, pixels.to_vec()));
        }
        HostCallOutcome::Ok0
    }

    fn draw_pixels_persistent_rgb565(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        pixels: &[u8],
    ) -> HostCallOutcome {
        if w <= 0 || h <= 0 || pixels.len() != w as usize * h as usize * 2 {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        }
        for row in 0..h {
            let dy = y + row;
            if !(0..320).contains(&dy) {
                continue;
            }
            let left = x.max(0);
            let right = (x + w).min(320);
            if left >= right {
                continue;
            }
            let src = ((row * w + (left - x)) * 2) as usize;
            let dst = ((dy * 320 + left) * 2) as usize;
            let len = ((right - left) * 2) as usize;
            self.persistent_pixels[dst..dst + len].copy_from_slice(&pixels[src..src + len]);
        }
        HostCallOutcome::Ok0
    }

    fn game2d_set_tile(&mut self, layer: i32, x: i32, y: i32, tile_ref: i32) -> HostCallOutcome {
        if layer != 0 {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        }
        let (Ok(cx), Ok(cy)) = (usize::try_from(x), usize::try_from(y)) else {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        };
        if cx >= self.tilemap.columns || cy >= self.tilemap.rows {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        }
        // `tile_ref` is an app-heap byte offset (validated against the heap at
        // present time, when the heap is in hand); `< 0` clears the cell.
        self.tilemap.cells[SimTilemap::index(cx, cy)] = tile_ref;
        HostCallOutcome::Ok0
    }

    fn game2d_clear_layer(&mut self, layer: i32) -> HostCallOutcome {
        if layer != 0 {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        }
        self.tilemap.cells.fill(-1);
        HostCallOutcome::Ok0
    }

    fn game2d_configure_tilemap(
        &mut self,
        layer: i32,
        columns: i32,
        rows: i32,
        origin_x: i32,
        origin_y: i32,
    ) -> HostCallOutcome {
        let (Ok(columns), Ok(rows), Ok(origin_x), Ok(origin_y)) = (
            usize::try_from(columns),
            usize::try_from(rows),
            i16::try_from(origin_x),
            i16::try_from(origin_y),
        ) else {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        };
        if layer != 0
            || !(1..=GAME2D_TILEMAP_MAX_COLS).contains(&columns)
            || !(1..=GAME2D_TILEMAP_MAX_ROWS).contains(&rows)
        {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        }
        self.tilemap.cells.fill(-1);
        self.tilemap.columns = columns;
        self.tilemap.rows = rows;
        self.tilemap.origin_x = origin_x;
        self.tilemap.origin_y = origin_y;
        HostCallOutcome::Ok0
    }

    fn game2d_present(&mut self, heap: &[u8]) -> HostCallOutcome {
        // Re-emit every non-empty cell into the frame's `draw_pixels` list so the
        // existing paint pipeline renders the tilemap unchanged. The simulator
        // repaints fully each frame, so emitting all cells (not just dirty ones)
        // is correct; dirty tracking is a device concern (KOTO-0135).
        for cy in 0..self.tilemap.rows {
            for cx in 0..self.tilemap.columns {
                let tile_ref = self.tilemap.cells[SimTilemap::index(cx, cy)];
                let Ok(off) = usize::try_from(tile_ref) else {
                    continue; // empty (`-1`) or invalid
                };
                let Some(src) = heap.get(off..off.saturating_add(GAME2D_TILE_BYTES)) else {
                    return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
                };
                self.draw_pixels.push((
                    i32::from(self.tilemap.origin_x) + cx as i32 * GAME2D_TILE,
                    i32::from(self.tilemap.origin_y) + cy as i32 * GAME2D_TILE,
                    GAME2D_TILE,
                    GAME2D_TILE,
                    src.to_vec(),
                ));
            }
        }
        // Sprite layer (KOTO-0140): composite each visible sprite's cells over the
        // board tilemap and beneath the per-frame text, in the fixed z-order. Each
        // cell blits the sprite's 16x16 tile at `(x + dcol*16, y + drow*16)`; the
        // stamp supplies the `(dcol,drow)` offsets from the app heap by byte offset.
        for index in 0..GAME2D_MAX_SPRITES {
            let Some(sprite) = self.sprites[index] else {
                continue;
            };
            let Some(stamp) = self
                .stamps
                .get(usize::from(sprite.stamp_id))
                .copied()
                .flatten()
            else {
                continue;
            };
            let Ok(tile_off) = usize::try_from(sprite.tile_ref) else {
                continue;
            };
            let Some(tile) = heap.get(tile_off..tile_off.saturating_add(GAME2D_TILE_BYTES)) else {
                return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
            };
            let tile = tile.to_vec();
            for cell in 0..stamp.count as usize {
                let Some((dcol, drow)) = stamp_cell(heap, &stamp, cell) else {
                    return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
                };
                self.draw_pixels.push((
                    sprite.x + dcol * GAME2D_TILE,
                    sprite.y + drow * GAME2D_TILE,
                    GAME2D_TILE,
                    GAME2D_TILE,
                    tile.clone(),
                ));
            }
        }
        HostCallOutcome::Ok0
    }

    fn game2d_stamp_define(
        &mut self,
        stamp_id: i32,
        cells_off: i32,
        count: i32,
        format: i32,
    ) -> HostCallOutcome {
        let (Ok(id), Ok(off), Ok(count)) = (
            usize::try_from(stamp_id),
            u32::try_from(cells_off),
            u8::try_from(count),
        ) else {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        };
        if id >= GAME2D_MAX_STAMPS || format != 0 || count == 0 {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        }
        self.stamps[id] = Some(Game2dStamp {
            cells_off: off,
            count,
        });
        HostCallOutcome::Ok0
    }

    fn game2d_sprite_set(
        &mut self,
        inst_id: i32,
        stamp_id: i32,
        x: i32,
        y: i32,
        tile_ref: i32,
    ) -> HostCallOutcome {
        let (Ok(id), Ok(stamp)) = (usize::try_from(inst_id), u8::try_from(stamp_id)) else {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        };
        if id >= GAME2D_MAX_SPRITES {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        }
        self.sprites[id] = Some(Game2dSprite {
            stamp_id: stamp,
            x,
            y,
            tile_ref,
        });
        HostCallOutcome::Ok0
    }

    fn game2d_sprite_hide(&mut self, inst_id: i32) -> HostCallOutcome {
        let Ok(id) = usize::try_from(inst_id) else {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        };
        if id >= GAME2D_MAX_SPRITES {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        }
        self.sprites[id] = None;
        HostCallOutcome::Ok0
    }

    fn game2d_sprite_clear_all(&mut self) -> HostCallOutcome {
        self.sprites = [None; GAME2D_MAX_SPRITES];
        HostCallOutcome::Ok0
    }

    fn game2d_text_set(
        &mut self,
        id: i32,
        x: i32,
        y: i32,
        text: &str,
        rgb565: i32,
    ) -> HostCallOutcome {
        let Ok(id) = usize::try_from(id) else {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        };
        if id >= GAME2D_MAX_TEXT_ITEMS {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        }
        self.text_items[id] = Some(Game2dText {
            x,
            y,
            rgb565,
            text: text.to_string(),
        });
        HostCallOutcome::Ok0
    }

    fn game2d_text_hide(&mut self, id: i32) -> HostCallOutcome {
        let Ok(id) = usize::try_from(id) else {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        };
        if id >= GAME2D_MAX_TEXT_ITEMS {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        }
        self.text_items[id] = None;
        HostCallOutcome::Ok0
    }

    fn game2d_text_clear_all(&mut self) -> HostCallOutcome {
        self.text_items = core::array::from_fn(|_| None);
        HostCallOutcome::Ok0
    }

    fn game2d_static_begin(&mut self) -> HostCallOutcome {
        // Clear the retained static layer and route subsequent draw calls into it
        // (KOTO-0136). The layer persists across the per-frame draw clear, so a
        // rebuild starts from empty rather than appending to last build's commands.
        self.static_rects.clear();
        self.static_pixels.clear();
        self.static_text.clear();
        self.static_text_colors.clear();
        self.capturing_static = true;
        HostCallOutcome::Ok0
    }

    fn game2d_static_end(&mut self) -> HostCallOutcome {
        self.capturing_static = false;
        HostCallOutcome::Ok0
    }

    fn draw_text(&mut self, x: i32, y: i32, text: &str) -> HostCallOutcome {
        // Sentinel outside the RGB565 range a `draw_text_color` colour can take
        // (app colours arrive as sign-extended `i16`, so `65535` lands as `-1` and
        // must not be mistaken for "use the default colour").
        if self.capturing_static {
            self.static_text.push((x, y, text.to_string()));
            self.static_text_colors.push(TEXT_COLOR_DEFAULT);
        } else {
            self.text.push((x, y, text.to_string()));
            self.text_colors.push(TEXT_COLOR_DEFAULT);
        }
        HostCallOutcome::Ok0
    }

    fn draw_text_color(&mut self, x: i32, y: i32, text: &str, rgb565: i32) -> HostCallOutcome {
        if self.capturing_static {
            self.static_text.push((x, y, text.to_string()));
            self.static_text_colors.push(rgb565);
        } else {
            self.text.push((x, y, text.to_string()));
            self.text_colors.push(rgb565);
        }
        HostCallOutcome::Ok0
    }

    fn input_snapshot(&mut self, input: VmInputSnapshot) -> HostCallOutcome {
        HostCallOutcome::Ok2(input.held_bits as i32, input.pressed_bits as i32)
    }

    fn audio_submit_i16(&mut self, frames: i32, channels: i32, samples: &[u8]) -> HostCallOutcome {
        self.audio_events
            .push(AudioEvent::SubmitPcm { frames, channels });
        let accepted = self
            .audio
            .lock()
            .map(|mut audio| audio.submit_pcm(channels, samples))
            .unwrap_or(0);
        HostCallOutcome::Ok1(accepted)
    }

    fn play_sfx(&mut self, id: i32) -> HostCallOutcome {
        self.audio_events.push(AudioEvent::Sfx(id));
        if let Ok(mut audio) = self.audio.lock() {
            audio.play_sfx(id);
        }
        HostCallOutcome::Ok0
    }

    fn play_bgm(&mut self, id: i32) -> HostCallOutcome {
        self.audio_events.push(AudioEvent::Bgm(id));
        HostCallOutcome::Err(koto_core::HostErrorCode::UNSUPPORTED)
    }

    fn play_bgm_asset(&mut self, path: &str) -> HostCallOutcome {
        if !path.starts_with("audio/") || !(path.ends_with(".kmml") || path.ends_with(".kacl")) {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        }
        if !self.asset_paths.iter().any(|asset| asset == path) {
            return HostCallOutcome::Err(koto_core::HostErrorCode::PERMISSION_DENIED);
        }
        let bytes = match self.read_asset_bytes(
            path,
            if path.ends_with(".kacl") {
                usize::MAX
            } else {
                4096
            },
        ) {
            Ok(bytes) => bytes,
            Err(outcome) => return outcome,
        };
        if path.ends_with(".kacl") {
            let ok = self
                .audio
                .lock()
                .map(|mut audio| audio.play_runtime_bgm_clip(&bytes))
                .unwrap_or(false);
            if !ok {
                return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
            }
            self.audio_events.push(AudioEvent::BgmAsset);
            return HostCallOutcome::Ok0;
        }
        let cue = match if bytes.starts_with(b"KAQ1") {
            RuntimeCue::<RUNTIME_BGM_EVENTS_PER_TRACK>::decode(&bytes)
        } else {
            core::str::from_utf8(&bytes)
                .map_err(|_| koto_audio::RuntimeCueError::InvalidText)
                .and_then(RuntimeCue::<RUNTIME_BGM_EVENTS_PER_TRACK>::compile_kmml)
        } {
            Ok(cue) => cue,
            Err(_) => return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT),
        };
        if let Ok(mut audio) = self.audio.lock() {
            audio.play_runtime_bgm(cue);
        }
        self.audio_events.push(AudioEvent::BgmAsset);
        HostCallOutcome::Ok0
    }

    fn play_sfx_asset(&mut self, path: &str) -> HostCallOutcome {
        if !path.starts_with("audio/") || !(path.ends_with(".kmml") || path.ends_with(".kacl")) {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        }
        if !self.asset_paths.iter().any(|asset| asset == path) {
            return HostCallOutcome::Err(koto_core::HostErrorCode::PERMISSION_DENIED);
        }
        let bytes = match self.read_asset_bytes(path, usize::MAX) {
            Ok(bytes) => bytes,
            Err(outcome) => return outcome,
        };
        if path.ends_with(".kacl") {
            let ok = self
                .audio
                .lock()
                .map(|mut audio| audio.play_runtime_clip(&bytes))
                .unwrap_or(false);
            if !ok {
                return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
            }
            self.audio_events.push(AudioEvent::SfxAsset);
            return HostCallOutcome::Ok0;
        }
        let cue = match if bytes.starts_with(b"KAQ1") {
            RuntimeCue::<RUNTIME_SFX_EVENTS_PER_TRACK>::decode(&bytes)
        } else {
            core::str::from_utf8(&bytes)
                .map_err(|_| koto_audio::RuntimeCueError::InvalidText)
                .and_then(RuntimeCue::<RUNTIME_SFX_EVENTS_PER_TRACK>::compile_kmml)
        } {
            Ok(cue) => cue,
            Err(_) => return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT),
        };
        if let Ok(mut audio) = self.audio.lock() {
            audio.play_runtime_sfx(cue);
        }
        self.audio_events.push(AudioEvent::SfxAsset);
        HostCallOutcome::Ok0
    }

    fn stop_bgm(&mut self) -> HostCallOutcome {
        self.audio_events.push(AudioEvent::StopBgm);
        if let Ok(mut audio) = self.audio.lock() {
            // Native KotoAudio BGM stops without affecting one-shot SFX.
            audio.seq_stop_bgm();
        }
        HostCallOutcome::Ok0
    }

    fn file_open(&mut self, path: &str, mode: i32) -> HostCallOutcome {
        let mode = match mode {
            0 => FileMode::Read,
            1 => FileMode::Write,
            2 => FileMode::ReadWrite,
            _ => return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT),
        };
        let path = match self.sandbox_path(path) {
            Ok(path) => path,
            Err(error) => return HostCallOutcome::Err(error.code()),
        };
        if matches!(mode, FileMode::Write | FileMode::ReadWrite) {
            if let Some(root) = self.fs.root() {
                if let Some(parent) = root.join(&path).parent() {
                    if fs::create_dir_all(parent).is_err() {
                        return HostCallOutcome::Err(koto_core::HostErrorCode::IO_ERROR);
                    }
                }
            }
        }
        match self.fs.open(&path, mode) {
            Ok(file) => self.allocate_handle(file),
            Err(_) => HostCallOutcome::Err(koto_core::HostErrorCode::IO_ERROR),
        }
    }

    fn file_read(&mut self, handle: i32, dst: &mut [u8]) -> HostCallOutcome {
        match self.file_mut(handle).and_then(|file| {
            file.read(dst)
                .map_err(|_| HostErrorKind::Io)
                .map(|len| len as i32)
        }) {
            Ok(len) => HostCallOutcome::Ok1(len),
            Err(error) => HostCallOutcome::Err(error.code()),
        }
    }

    fn file_write(&mut self, handle: i32, src: &[u8]) -> HostCallOutcome {
        match self.file_mut(handle).and_then(|file| {
            file.write(src)
                .map_err(|_| HostErrorKind::Io)
                .map(|len| len as i32)
        }) {
            Ok(len) => HostCallOutcome::Ok1(len),
            Err(error) => HostCallOutcome::Err(error.code()),
        }
    }

    fn file_close(&mut self, handle: i32) -> HostCallOutcome {
        let handle = match usize::try_from(handle) {
            Ok(handle) => handle,
            Err(_) => return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT),
        };
        match self.files.get_mut(handle) {
            Some(slot @ Some(_)) => {
                *slot = None;
                HostCallOutcome::Ok0
            }
            _ => HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT),
        }
    }

    fn asset_load(&mut self, path: &str, dst: &mut [u8]) -> HostCallOutcome {
        // Only manifest-declared package assets are readable, mirroring how BGM/SFX
        // assets are gated. The asset is read-only and never touches
        // the save sandbox.
        if !self.asset_paths.iter().any(|asset| asset == path) {
            return HostCallOutcome::Err(koto_core::HostErrorCode::PERMISSION_DENIED);
        }
        let bytes = match self.read_asset_bytes(path, dst.len()) {
            Ok(bytes) => bytes,
            Err(outcome) => return outcome,
        };
        let n = bytes.len().min(dst.len());
        dst[..n].copy_from_slice(&bytes[..n]);
        HostCallOutcome::Ok1(n as i32)
    }

    fn asset_load_range(&mut self, path: &str, offset: usize, dst: &mut [u8]) -> HostCallOutcome {
        if !self.asset_paths.iter().any(|asset| asset == path) {
            return HostCallOutcome::Err(koto_core::HostErrorCode::PERMISSION_DENIED);
        }
        let bytes = match self.read_asset_bytes(path, usize::MAX) {
            Ok(bytes) => bytes,
            Err(outcome) => return outcome,
        };
        if offset > bytes.len() {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        }
        let n = dst.len().min(bytes.len() - offset);
        dst[..n].copy_from_slice(&bytes[offset..offset + n]);
        HostCallOutcome::Ok1(n as i32)
    }

    fn fetch_start(&mut self, url: &str) -> HostCallOutcome {
        self.fetch_poll_cache = None;
        match self.fetch.start(
            self.fetch_app,
            &self.fetch_allowlist,
            url,
            self.fetch_now_ms,
        ) {
            Ok(id) => HostCallOutcome::Ok1(id.raw() as i32),
            Err(error) => HostCallOutcome::Err(fetch_host_error(error)),
        }
    }

    fn fetch_poll(&mut self, request_id: i32) -> HostCallOutcome {
        if let Some((cached_id, cached_at, state, metadata)) = self.fetch_poll_cache {
            if cached_id == request_id && cached_at == self.fetch_now_ms {
                return HostCallOutcome::Ok2(state, metadata);
            }
        }
        let id = FetchRequestId::from_raw(request_id as u32);
        let (state, metadata) = match self.fetch.poll(self.fetch_app, id, self.fetch_now_ms) {
            Ok(FetchPoll::Pending) => (0, 0),
            Ok(FetchPoll::Headers { status }) => (1, i32::from(status)),
            Ok(FetchPoll::Body) => (2, 0),
            Ok(FetchPoll::Complete) => (3, 0),
            Ok(FetchPoll::Failed(error)) | Err(error) => (4, error as i32),
        };
        self.fetch_poll_cache = Some((request_id, self.fetch_now_ms, state, metadata));
        HostCallOutcome::Ok2(state, metadata)
    }

    fn fetch_read(&mut self, request_id: i32, dst: &mut [u8]) -> HostCallOutcome {
        self.fetch_poll_cache = None;
        let id = FetchRequestId::from_raw(request_id as u32);
        match self.fetch.read(self.fetch_app, id, dst) {
            Ok(len) => HostCallOutcome::Ok1(len as i32),
            Err(error) => HostCallOutcome::Err(fetch_host_error(error)),
        }
    }

    fn fetch_cancel(&mut self, request_id: i32) -> HostCallOutcome {
        self.fetch_poll_cache = None;
        let id = FetchRequestId::from_raw(request_id as u32);
        match self.fetch.cancel(self.fetch_app, id) {
            Ok(()) => HostCallOutcome::Ok0,
            Err(error) => HostCallOutcome::Err(fetch_host_error(error)),
        }
    }

    // Bounded MQTT subscribe service (Host ABI minor 23, KOTO-0249). The app
    // names a manifest broker/topic by index; the OS owns the socket, TLS, and
    // credential. The service shares the deterministic `fetch_now_ms` clock.

    fn mqtt_connect(&mut self, broker_index: i32) -> HostCallOutcome {
        let Ok(index) = usize::try_from(broker_index) else {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        };
        match self
            .mqtt
            .connect(self.mqtt_app, &self.mqtt_brokers, index, self.fetch_now_ms)
        {
            Ok(id) => HostCallOutcome::Ok1(id.raw() as i32),
            Err(error) => HostCallOutcome::Err(mqtt_host_error(error)),
        }
    }

    fn mqtt_subscribe(&mut self, session: i32, topic_index: i32) -> HostCallOutcome {
        let Ok(index) = usize::try_from(topic_index) else {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        };
        let id = MqttSessionId::from_raw(session as u32);
        match self.mqtt.subscribe(
            self.mqtt_app,
            id,
            &self.mqtt_topics,
            index,
            self.fetch_now_ms,
        ) {
            Ok(()) => HostCallOutcome::Ok1(0),
            Err(error) => HostCallOutcome::Err(mqtt_host_error(error)),
        }
    }

    fn mqtt_poll(&mut self, session: i32) -> HostCallOutcome {
        let id = MqttSessionId::from_raw(session as u32);
        match self.mqtt.poll(self.mqtt_app, id, self.fetch_now_ms) {
            Ok(state) => HostCallOutcome::Ok1(state.state_code()),
            Err(error) => HostCallOutcome::Err(mqtt_host_error(error)),
        }
    }

    fn mqtt_peek(&mut self, session: i32) -> HostCallOutcome {
        let id = MqttSessionId::from_raw(session as u32);
        match self.mqtt.peek(self.mqtt_app, id) {
            Ok(Some((topic_len, payload_len))) => {
                HostCallOutcome::Ok2(topic_len as i32, payload_len as i32)
            }
            Ok(None) => HostCallOutcome::Ok2(0, 0),
            Err(error) => HostCallOutcome::Err(mqtt_host_error(error)),
        }
    }

    fn mqtt_read(&mut self, session: i32, topic: &mut [u8], payload: &mut [u8]) -> HostCallOutcome {
        let id = MqttSessionId::from_raw(session as u32);
        match self.mqtt.read_message(self.mqtt_app, id, topic, payload) {
            Ok(None) => HostCallOutcome::Ok1(koto_core::mqtt::app_mqtt::READ_NONE),
            Ok(Some(message)) => HostCallOutcome::Ok1(if message.retained {
                koto_core::mqtt::app_mqtt::READ_RETAINED
            } else {
                koto_core::mqtt::app_mqtt::READ_MESSAGE
            }),
            Err(error) => HostCallOutcome::Err(mqtt_host_error(error)),
        }
    }

    fn mqtt_disconnect(&mut self, session: i32) -> HostCallOutcome {
        let id = MqttSessionId::from_raw(session as u32);
        match self.mqtt.disconnect(self.mqtt_app, id) {
            Ok(()) => HostCallOutcome::Ok1(0),
            Err(error) => HostCallOutcome::Err(mqtt_host_error(error)),
        }
    }

    fn mqtt_dropped(&mut self, session: i32) -> HostCallOutcome {
        let id = MqttSessionId::from_raw(session as u32);
        match self.mqtt.dropped(self.mqtt_app, id) {
            Ok(count) => HostCallOutcome::Ok1(count as i32),
            Err(error) => HostCallOutcome::Err(mqtt_host_error(error)),
        }
    }

    fn json_reset(&mut self) -> HostCallOutcome {
        self.json.reset();
        HostCallOutcome::Ok0
    }

    fn json_next(&mut self, src: &[u8]) -> HostCallOutcome {
        HostCallOutcome::Ok1(self.json.next(src))
    }

    fn json_finish(&mut self) -> HostCallOutcome {
        HostCallOutcome::Ok1(self.json.finish())
    }

    fn json_token(&mut self, dst: &mut [u8]) -> HostCallOutcome {
        let token = self.json.token();
        if dst.len() < token.len() {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        }
        dst[..token.len()].copy_from_slice(token);
        HostCallOutcome::Ok1(token.len() as i32)
    }

    fn json_error(&mut self) -> HostCallOutcome {
        let (code, offset) = self.json.error();
        HostCallOutcome::Ok2(code, offset)
    }

    fn json_status(&mut self) -> HostCallOutcome {
        let (consumed, depth) = self.json.status();
        HostCallOutcome::Ok2(consumed, depth)
    }

    /// Advisory time (Host ABI minor 21, KOTO-0247), driven entirely by the
    /// deterministic frame clock and the scripted UTC anchor — never the host
    /// wall clock.
    fn time_query(&mut self, kind: i32) -> HostCallOutcome {
        use koto_core::time::app_time_query as time_query;
        match kind {
            time_query::UTC_SECONDS => {
                let utc = self
                    .time_utc_base
                    .map(|base| base + (self.fetch_now_ms / 1_000) as i64);
                match utc {
                    Some(seconds) if (0..=i64::from(i32::MAX)).contains(&seconds) => {
                        HostCallOutcome::Ok1(seconds as i32)
                    }
                    _ => HostCallOutcome::Ok1(-1),
                }
            }
            time_query::OFFSET_MINUTES => {
                HostCallOutcome::Ok1(i32::from(self.config_snapshot.utc_offset.minutes()))
            }
            time_query::MONOTONIC_MS => HostCallOutcome::Ok1(
                (self.fetch_now_ms as i64 & i64::from(time_query::MONOTONIC_MASK)) as i32,
            ),
            _ => HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT),
        }
    }

    /// Resolve the running app's opaque credential handle for `url` (KOTO-0248).
    /// Returns `0` when no grant applies; never exposes a secret.
    fn vault_handle(&mut self, service: i32, url: &str) -> HostCallOutcome {
        HostCallOutcome::Ok1(self.vault.handle(service, url))
    }

    /// Start an allowlisted GET with the granted credential injected by the OS.
    /// The sim validates the handle exactly as the device host would (grant
    /// generation, app id, service, exact endpoint, TLS), then starts the fetch;
    /// a failed validation is a fixed `PERMISSION_DENIED` and no secret is
    /// exposed. The fake secret never rides the sim's fake transport.
    fn fetch_start_authenticated(&mut self, url: &str, handle: i32) -> HostCallOutcome {
        match self.vault.resolve_fetch(url, handle) {
            Ok(()) => self.fetch_start(url),
            Err(_) => HostCallOutcome::Err(koto_core::HostErrorCode::PERMISSION_DENIED),
        }
    }

    fn close_all_files(&mut self) {
        for slot in &mut self.files {
            *slot = None;
        }
        self.fetch.teardown();
        self.fetch_poll_cache = None;
        self.mqtt.teardown();
    }

    fn ime_feed_key(&mut self, kind: i32, codepoint: i32) -> HostCallOutcome {
        use koto_core::runtime::ime_key;
        let key = match kind {
            ime_key::CHARACTER => match u32::try_from(codepoint).ok().and_then(char::from_u32) {
                Some(ch) => MemoImeKey::Character(ch),
                None => return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT),
            },
            ime_key::SHIFT => MemoImeKey::Shift,
            ime_key::CONVERT => MemoImeKey::Convert,
            ime_key::COMMIT => MemoImeKey::Commit,
            ime_key::CANCEL => MemoImeKey::Cancel,
            ime_key::BACKSPACE => MemoImeKey::Backspace,
            ime_key::OTHER => MemoImeKey::Other,
            ime_key::TOGGLE => MemoImeKey::Toggle,
            _ => return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT),
        };
        match self.ime.process_key(key, &mut self.editor) {
            Ok(()) => HostCallOutcome::Ok0,
            Err(_) => HostCallOutcome::Err(koto_core::HostErrorCode::IO_ERROR),
        }
    }

    fn ime_convert(&mut self) -> HostCallOutcome {
        let Some(skk) = self.skk.as_mut() else {
            return HostCallOutcome::Err(koto_core::HostErrorCode::UNSUPPORTED);
        };
        let mut access = WindowedDict {
            index: &skk.index,
            reader: skk.dict.as_slice(),
            window: &mut skk.window,
        };
        match self.ime.convert_with_access(&mut access, &mut self.editor) {
            Ok(()) => HostCallOutcome::Ok0,
            Err(_) => HostCallOutcome::Err(koto_core::HostErrorCode::IO_ERROR),
        }
    }

    fn ime_query_line(&mut self, dst: &mut [u8]) -> HostCallOutcome {
        match serialize_ime_line(&self.ime.line(), dst) {
            Some(len) => HostCallOutcome::Ok1(len as i32),
            None => HostCallOutcome::Err(koto_core::HostErrorCode::NO_MEMORY),
        }
    }

    fn edit_move(&mut self, dir: i32) -> HostCallOutcome {
        use koto_core::runtime::edit_dir;
        let movement = match dir {
            edit_dir::LEFT => MemoMove::Left,
            edit_dir::RIGHT => MemoMove::Right,
            edit_dir::UP => MemoMove::Up,
            edit_dir::DOWN => MemoMove::Down,
            edit_dir::HOME => MemoMove::Home,
            edit_dir::END => MemoMove::End,
            _ => return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT),
        };
        self.editor.move_cursor(movement);
        HostCallOutcome::Ok0
    }

    fn edit_reserve_rows(&mut self, rows: i32) -> HostCallOutcome {
        self.editor
            .set_reserved_bottom_rows(usize::try_from(rows).unwrap_or(0));
        HostCallOutcome::Ok0
    }

    fn edit_configure(&mut self, cols: i32, rows: i32) -> HostCallOutcome {
        if !self.editor.as_str().is_empty() {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        }
        let (Ok(cols), Ok(rows)) = (u16::try_from(cols), u16::try_from(rows)) else {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        };
        if !(8..=80).contains(&cols) || !(4..=30).contains(&rows) {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        }
        let Some(editor) = text_editor(cols, rows) else {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        };
        self.editor = editor;
        HostCallOutcome::Ok0
    }

    fn edit_delete(&mut self, kind: i32) -> HostCallOutcome {
        use koto_core::runtime::edit_delete;
        let removed = match kind {
            edit_delete::BACKSPACE => self.editor.backspace(),
            edit_delete::FORWARD => self.editor.delete(),
            _ => return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT),
        };
        match removed {
            Ok(removed) => HostCallOutcome::Ok1(removed as i32),
            Err(_) => HostCallOutcome::Err(koto_core::HostErrorCode::IO_ERROR),
        }
    }

    fn edit_load(&mut self, src: &[u8]) -> HostCallOutcome {
        let text = match core::str::from_utf8(src) {
            Ok(text) => text,
            Err(_) => return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT),
        };
        match self.editor.load_str(text) {
            Ok(()) => HostCallOutcome::Ok0,
            Err(_) => HostCallOutcome::Err(koto_core::HostErrorCode::NO_MEMORY),
        }
    }

    fn edit_query_text(&mut self, dst: &mut [u8]) -> HostCallOutcome {
        let text = self.editor.as_str().as_bytes();
        let len = text.len().min(dst.len());
        dst[..len].copy_from_slice(&text[..len]);
        HostCallOutcome::Ok2(len as i32, self.editor.cursor() as i32)
    }

    fn ime_display(&mut self, dst: &mut [u8]) -> HostCallOutcome {
        let line = self.ime.line();
        // Prefix active composition text with a stable state label so apps,
        // scripts, and screenshots can tell pending romaji from readings and
        // failed conversions without parsing the structured IME line.
        let mut display = String::new();
        match line.mode {
            MemoImeMode::Empty => {}
            MemoImeMode::Composing => {
                display.push_str("comp:");
                display.push_str(line.pending_romaji);
            }
            MemoImeMode::Converting => {
                display.push_str("read:");
                display.push_str(line.reading);
                display.push_str(line.pending_romaji);
            }
            MemoImeMode::Candidate => {
                display.push_str("cand:");
                if let Some(candidate) = line.candidate {
                    display.push_str(candidate);
                }
            }
            MemoImeMode::MissingCandidate => {
                display.push_str("miss:");
                display.push_str(line.reading);
            }
        }
        // Copy the largest whole-character prefix that fits (never split UTF-8).
        let mut end = display.len().min(dst.len());
        while end > 0 && !display.is_char_boundary(end) {
            end -= 1;
        }
        dst[..end].copy_from_slice(&display.as_bytes()[..end]);
        HostCallOutcome::Ok1(end as i32)
    }

    fn edit_visible_line(&mut self, row: i32, dst: &mut [u8]) -> HostCallOutcome {
        let Ok(row) = u16::try_from(row) else {
            return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT);
        };
        let line = self.editor.visible_line(row).unwrap_or("");
        let mut end = line.len().min(dst.len());
        while end > 0 && !line.is_char_boundary(end) {
            end -= 1;
        }
        dst[..end].copy_from_slice(&line.as_bytes()[..end]);
        HostCallOutcome::Ok1(end as i32)
    }

    fn edit_cursor_view(&mut self) -> HostCallOutcome {
        // Caret position within the wrapped viewport: column within the visual row.
        let col = i32::try_from(self.editor.cursor_display_col()).unwrap_or(i32::MAX);
        let row = self
            .editor
            .cursor_visible_row()
            .map(i32::from)
            .unwrap_or(-1);
        HostCallOutcome::Ok2(col, row)
    }

    fn edit_scroll_row(&mut self) -> HostCallOutcome {
        HostCallOutcome::Ok1(i32::try_from(self.editor.scroll_row()).unwrap_or(i32::MAX))
    }

    fn edit_view_metrics(&mut self) -> HostCallOutcome {
        let cell = self.editor.layout().cell;
        HostCallOutcome::Ok2(i32::from(cell.cell_width), i32::from(cell.cell_height))
    }

    fn edit_cursor_status(&mut self, dst: &mut [u8]) -> HostCallOutcome {
        // Report the true document position (logical line + column), independent of
        // soft wrapping.
        let row = self.editor.cursor_logical_line() + 1;
        let col = self.editor.cursor_column() + 1;
        let status = format!("Ln {row} Col {col}");
        let len = status.len().min(dst.len());
        dst[..len].copy_from_slice(&status.as_bytes()[..len]);
        HostCallOutcome::Ok1(len as i32)
    }

    fn edit_total_lines(&mut self) -> HostCallOutcome {
        // Visual (wrapped) rows, so the vertical scrollbar tracks what is painted.
        HostCallOutcome::Ok1(i32::try_from(self.editor.total_visual_rows()).unwrap_or(i32::MAX))
    }

    fn edit_wrap(&mut self) -> HostCallOutcome {
        HostCallOutcome::Ok1(i32::from(self.editor.is_wrap()))
    }

    fn edit_hscroll_view(&mut self) -> HostCallOutcome {
        let hscroll = i32::try_from(self.editor.hscroll()).unwrap_or(i32::MAX);
        let line_cols = i32::try_from(self.editor.cursor_line_cols()).unwrap_or(i32::MAX);
        HostCallOutcome::Ok2(hscroll, line_cols)
    }

    fn dir_list(&mut self, index: i32, dst: &mut [u8]) -> HostCallOutcome {
        let names = self.sandbox_entry_names();
        let count = i32::try_from(names.len()).unwrap_or(i32::MAX);
        let index = match usize::try_from(index) {
            Ok(index) => index,
            Err(_) => return HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT),
        };
        let Some(name) = names.get(index) else {
            // Out-of-range index is not an error; it reports an empty name so the
            // app can probe the count with a single call.
            return HostCallOutcome::Ok2(count, 0);
        };
        let bytes = name.as_bytes();
        let written = bytes.len().min(dst.len());
        dst[..written].copy_from_slice(&bytes[..written]);
        HostCallOutcome::Ok2(count, written as i32)
    }
}

fn stable_app_id(app_id: &str) -> u32 {
    app_id.bytes().fold(2_166_136_261u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
    })
}

fn fetch_host_error(error: FetchError) -> koto_core::HostErrorCode {
    match error {
        FetchError::Denied | FetchError::ForeignRequest => {
            koto_core::HostErrorCode::PERMISSION_DENIED
        }
        FetchError::Unavailable => koto_core::HostErrorCode::UNSUPPORTED,
        FetchError::Busy | FetchError::BufferTooLarge | FetchError::MalformedUrl => {
            koto_core::HostErrorCode::BAD_ARGUMENT
        }
        FetchError::StaleRequest => koto_core::HostErrorCode::NOT_FOUND,
        _ => koto_core::HostErrorCode::IO_ERROR,
    }
}

fn mqtt_host_error(error: MqttError) -> koto_core::HostErrorCode {
    match error {
        MqttError::Denied | MqttError::TopicNotAllowed => {
            koto_core::HostErrorCode::PERMISSION_DENIED
        }
        MqttError::Unavailable => koto_core::HostErrorCode::UNSUPPORTED,
        MqttError::Busy
        | MqttError::BufferTooLarge
        | MqttError::MalformedBroker
        | MqttError::MessageTooLarge => koto_core::HostErrorCode::BAD_ARGUMENT,
        MqttError::StaleSession | MqttError::ForeignSession => koto_core::HostErrorCode::NOT_FOUND,
        _ => koto_core::HostErrorCode::IO_ERROR,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SimUiTextLayout {
    pub(super) clip: koto_ui::UiRect,
    pub(super) bounds: koto_ui::UiRect,
    pub(super) align: koto_ui::TextAlign,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SimUiCommand {
    Rect(usize),
    Text(usize),
}

struct SimUiPainter<'a> {
    rects: &'a mut Vec<(i32, i32, i32, i32, i32)>,
    text: &'a mut Vec<(i32, i32, String)>,
    colors: &'a mut Vec<i32>,
    layouts: &'a mut Vec<SimUiTextLayout>,
    commands: &'a mut Vec<SimUiCommand>,
}

// KotoSim and PicoCalc both render retained App UI with mplus12.kfont.
// Command layout must use the font's real advances or UTF-8 text and its caret
// diverge when CanvasUiPainter performs the final rasterization.
const UI_FONT_HALF_WIDTH: i32 = 6;
const UI_FONT_FULL_WIDTH: i32 = 12;
const UI_FONT_LINE_HEIGHT: i32 = 13;

impl koto_ui::TextMetrics for SimUiPainter<'_> {
    fn measure_text(&mut self, text: &str) -> Result<i32, koto_ui::PaintError> {
        Ok(text
            .chars()
            .map(|ch| {
                if ch.is_ascii() {
                    UI_FONT_HALF_WIDTH
                } else {
                    UI_FONT_FULL_WIDTH
                }
            })
            .sum())
    }

    fn line_height(&self) -> Option<i32> {
        Some(UI_FONT_LINE_HEIGHT)
    }
}

impl koto_ui::Painter for SimUiPainter<'_> {
    fn fill_rect(
        &mut self,
        clip: koto_ui::UiRect,
        rect: koto_ui::UiRect,
        color: koto_ui::Rgb565,
    ) -> Result<(), koto_ui::PaintError> {
        if let Some(rect) = rect.intersection(clip) {
            let index = self.rects.len();
            self.rects
                .push((rect.x, rect.y, rect.w, rect.h, i32::from(color.0)));
            self.commands.push(SimUiCommand::Rect(index));
        }
        Ok(())
    }
    fn stroke_rect(
        &mut self,
        clip: koto_ui::UiRect,
        rect: koto_ui::UiRect,
        color: koto_ui::Rgb565,
        width: u8,
    ) -> Result<(), koto_ui::PaintError> {
        let width = i32::from(width).max(1);
        for edge in [
            koto_ui::UiRect::new(rect.x, rect.y, rect.w, width),
            koto_ui::UiRect::new(rect.x, rect.y + rect.h - width, rect.w, width),
            koto_ui::UiRect::new(rect.x, rect.y, width, rect.h),
            koto_ui::UiRect::new(rect.x + rect.w - width, rect.y, width, rect.h),
        ] {
            self.fill_rect(clip, edge, color)?;
        }
        Ok(())
    }
    fn draw_text(
        &mut self,
        clip: koto_ui::UiRect,
        bounds: koto_ui::UiRect,
        run: koto_ui::TextRun<'_>,
    ) -> Result<(), koto_ui::PaintError> {
        if bounds.intersection(clip).is_none() {
            return Ok(());
        }
        let width = <Self as koto_ui::TextMetrics>::measure_text(self, run.text)?;
        let x = match run.align {
            koto_ui::TextAlign::Start => bounds.x,
            koto_ui::TextAlign::Center => bounds.x + (bounds.w - width) / 2,
            koto_ui::TextAlign::End => bounds.x + bounds.w - width,
        };
        let index = self.text.len();
        self.text.push((
            x,
            bounds.y + (bounds.h - UI_FONT_LINE_HEIGHT).max(0) / 2,
            run.text.to_string(),
        ));
        self.colors.push(i32::from(run.color.0));
        self.layouts.push(SimUiTextLayout {
            clip,
            bounds,
            align: run.align,
        });
        self.commands.push(SimUiCommand::Text(index));
        Ok(())
    }
    fn draw_glyphs(
        &mut self,
        clip: koto_ui::UiRect,
        bounds: koto_ui::UiRect,
        run: koto_ui::GlyphRun<'_>,
    ) -> Result<(), koto_ui::PaintError> {
        let text: String = run
            .glyphs
            .iter()
            .filter_map(|id| char::from_u32(u32::from(*id)))
            .collect();
        self.draw_text(
            clip,
            bounds,
            koto_ui::TextRun {
                text: &text,
                color: run.color,
                align: koto_ui::TextAlign::Start,
            },
        )
    }
    fn draw_focus_mark(
        &mut self,
        clip: koto_ui::UiRect,
        rect: koto_ui::UiRect,
        color: koto_ui::Rgb565,
        width: u8,
    ) -> Result<(), koto_ui::PaintError> {
        self.stroke_rect(clip, rect, color, width)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostErrorKind {
    BadArgument,
    PermissionDenied,
    Io,
}

impl HostErrorKind {
    fn code(self) -> koto_core::HostErrorCode {
        match self {
            HostErrorKind::BadArgument => koto_core::HostErrorCode::BAD_ARGUMENT,
            HostErrorKind::PermissionDenied => koto_core::HostErrorCode::PERMISSION_DENIED,
            HostErrorKind::Io => koto_core::HostErrorCode::IO_ERROR,
        }
    }
}

/// Resolve cell `index` of a `format 0` stamp to its `(dcol, drow)` offset. Each
/// cell is one nibble at heap byte `cells_off + index/2` (low nibble for even
/// `index`, high for odd), encoding `nibble = drow*4 + dcol` — the KOTO-0138 cell
/// layout. Returns `None` if the byte lies outside the heap.
fn stamp_cell(heap: &[u8], stamp: &Game2dStamp, index: usize) -> Option<(i32, i32)> {
    let byte = *heap.get(stamp.cells_off as usize + (index >> 1))?;
    let nibble = if index & 1 == 0 {
        byte & 0x0f
    } else {
        byte >> 4
    };
    Some(((nibble & 3) as i32, (nibble >> 2) as i32))
}

/// Load the host IME dictionary from the simulator root. Best-effort: a missing
/// or invalid dictionary leaves SKK conversion unavailable rather than failing the
/// launch, so non-text apps still run.
fn load_skk_session(fs: &mut HostFs) -> Option<SkkSession> {
    let mut file = fs.open(SKK_DICT_PATH, FileMode::Read).ok()?;
    let mut dict = Vec::new();
    let mut buffer = [0u8; 256];
    loop {
        let len = file.read(&mut buffer).ok()?;
        if len == 0 {
            break;
        }
        dict.extend_from_slice(&buffer[..len]);
    }
    // Build through the streaming reader for firmware parity (the sim keeps the
    // dictionary in memory anyway, but exercises the same code path).
    let mut window = [0u8; SKK_LOOKUP_WINDOW_BYTES];
    let index = SkkLeadingIndex::build_from_reader(&mut dict.as_slice(), &mut window).ok()?;
    Some(SkkSession {
        dict,
        index,
        window,
    })
}

fn ime_mode_code(mode: MemoImeMode) -> u8 {
    match mode {
        MemoImeMode::Empty => 0,
        MemoImeMode::Composing => 1,
        MemoImeMode::Converting => 2,
        MemoImeMode::Candidate => 3,
        MemoImeMode::MissingCandidate => 4,
    }
}

/// Serialize the IME composition line into `dst` for `ime_query_line`. Layout:
/// `[mode:u8][sticky:u8]` then three length-prefixed UTF-8 fields (`pending`,
/// `reading`, `candidate`), each `[len:u8][bytes]`, then `[cand_index:u8]`
/// `[cand_count:u8]` (the shown candidate's zero-based position and the total
/// candidate count, both saturating at 255). Returns the byte count, or `None`
/// when `dst` is too small or a field exceeds 255 bytes.
fn serialize_ime_line(line: &MemoImeLine<'_>, dst: &mut [u8]) -> Option<usize> {
    let fields = [
        line.pending_romaji.as_bytes(),
        line.reading.as_bytes(),
        line.candidate.unwrap_or("").as_bytes(),
    ];
    let needed = 2 + fields.iter().map(|field| 1 + field.len()).sum::<usize>() + 2;
    if needed > dst.len() {
        return None;
    }
    let mut at = 0;
    dst[at] = ime_mode_code(line.mode);
    dst[at + 1] = u8::from(line.sticky_shift_armed);
    at += 2;
    for field in fields {
        let len = u8::try_from(field.len()).ok()?;
        dst[at] = len;
        at += 1;
        dst[at..at + field.len()].copy_from_slice(field);
        at += field.len();
    }
    dst[at] = line.candidate_index.min(255) as u8;
    dst[at + 1] = line.candidate_count.min(255) as u8;
    at += 2;
    Some(at)
}
