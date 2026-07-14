use super::*;

/// A persistent, per-frame bytecode app running in KotoSim. Unlike
/// [`launch_package`], which runs a program once to completion, the session keeps
/// the VM, host, and IME/editor state alive across frames so window mode can route
/// input into the live VM and paint the VM's own draw output each frame.
pub struct BytecodeAppSession {
    app_id: String,
    bytecode: Vec<u8>,
    session: BytecodeSession<SIM_VM_STACK_SLOTS, SIM_VM_CALL_DEPTH>,
    /// App heap, sized to the program's KBC header request (per-app profile,
    /// KOTO-0096) and lent to the VM each frame; state persists here across frames.
    heap: Vec<u8>,
    host: SimRuntimeHost,
    /// Shared handle to the host audio engine (also held by `host`), so window mode
    /// can hand it to the cpal callback and the headless capture path can render it.
    audio: Arc<Mutex<SimAudio>>,
    /// Budget diagnostics (KOTO-0101): the program's KBC heap request and the
    /// manifest's declared per-app SRAM working budget (the device ceiling).
    heap_request: u32,
    sram_work_bytes: Option<u32>,
    /// Budget diagnostics: session high-water marks for host-owned working sets,
    /// tracked here because the host clears its per-frame draw lists each frame.
    open_files_peak: usize,
    draw_rects_peak: usize,
    draw_pixels_peak: usize,
    text_draws_peak: usize,
    audio_events_peak: usize,
}

impl BytecodeAppSession {
    /// Verify and load `app_id` from `root` into a fresh, not-yet-run session with a
    /// private, headless audio engine.
    pub fn launch(root: impl AsRef<Path>, app_id: &str) -> Result<Self, SimError> {
        let audio = Arc::new(Mutex::new(SimAudio::new(DEFAULT_SAMPLE_RATE)));
        Self::launch_with_audio(root, app_id, audio)
    }

    /// Like [`Self::launch`] but shares an external audio engine (window mode hands
    /// the same handle to the cpal output stream).
    pub fn launch_with_audio(
        root: impl AsRef<Path>,
        app_id: &str,
        audio: Arc<Mutex<SimAudio>>,
    ) -> Result<Self, SimError> {
        let root = root.as_ref();
        let (launch, archive) = load_launch_archive(root, app_id)?;
        if launch.runtime() != KOTORUNTIME_BYTECODE {
            return Err(SimError::InvalidRuntime);
        }
        let bytecode = if let Some(archive) = &archive {
            KpaReader::new(archive.as_slice())
                .map_err(|_| SimError::InvalidManifest)?
                .payload_for(launch.entry())
                .map_err(|_| SimError::InvalidManifest)?
                .ok_or(SimError::Io)?
                .to_vec()
        } else {
            let mut fs = HostFs::mounted(root).map_err(|_| SimError::Io)?;
            read_bytes(&mut fs, launch.entry())?
        };
        let session = BytecodeSession::new(
            &bytecode,
            RuntimeLimits::simulator_default(),
            SIM_FRAME_FUEL,
        )
        .map_err(|error| match error {
            SessionError::Verify(_) => SimError::RuntimeVerifyFailed,
            SessionError::Vm(_) => SimError::RuntimeExecutionFailed,
        })?;
        let heap_bytes = session.program().header().max_heap_bytes;
        // The manifest's declared SRAM working budget is the per-app device budget:
        // reject an app whose KBC heap request exceeds it (per-app profile, KOTO-0096).
        if let Some(budget) = launch.package.sram_work_bytes() {
            if heap_bytes > budget {
                return Err(SimError::AppExceedsMemoryBudget);
            }
        }
        let host = if let Some(archive) = archive {
            SimRuntimeHost::with_audio_and_package(
                HostFs::mounted(root).map_err(|_| SimError::Io)?,
                launch.package.app_id(),
                Arc::clone(&audio),
                archive,
            )
        } else {
            SimRuntimeHost::with_audio_and_assets(
                HostFs::mounted(root).map_err(|_| SimError::Io)?,
                launch.package.app_id(),
                Arc::clone(&audio),
                launch.asset_paths().to_vec(),
            )
        }?;
        // Initialize the heap with the const heap image (KOTO-0139): rodata becomes
        // heap[0..rodata_size]; the rest stays zeroed. The verifier has bounded
        // rodata_size <= max_heap_bytes, so this copy is in range.
        let mut heap = vec![0u8; heap_bytes as usize];
        if let Some((start, end)) = session.program().rodata_range() {
            heap[..end - start].copy_from_slice(&bytecode[start..end]);
        }
        Ok(Self {
            app_id: launch.package.app_id().to_string(),
            bytecode,
            session,
            heap,
            host,
            audio,
            heap_request: heap_bytes,
            sram_work_bytes: launch.package.sram_work_bytes(),
            open_files_peak: 0,
            draw_rects_peak: 0,
            draw_pixels_peak: 0,
            text_draws_peak: 0,
            audio_events_peak: 0,
        })
    }

    /// Run one cooperative frame: clear the per-frame draw log, then execute up to
    /// [`SIM_FRAME_FUEL`] instructions until the VM yields, exits, or exhausts fuel.
    /// On a VM trap the error and faulting program counter are retained for
    /// diagnostics (see [`Self::diagnostic`]).
    pub fn step_frame(&mut self, input: VmInputSnapshot) -> Result<VmRunResult, SimError> {
        self.host.clear_frame_draw();
        let result = self
            .session
            .step_frame(&self.bytecode, &mut self.host, input, &mut self.heap)
            .map_err(|_| SimError::RuntimeExecutionFailed)?;
        // Track host working-set high-water marks: the host clears its per-frame
        // draw lists at the start of the next frame, so capture them now (budget
        // diagnostics, KOTO-0101).
        self.open_files_peak = self.open_files_peak.max(self.host.open_file_count());
        self.draw_rects_peak = self.draw_rects_peak.max(self.host.draw_rects.len());
        self.draw_pixels_peak = self.draw_pixels_peak.max(self.host.draw_pixels.len());
        self.text_draws_peak = self.text_draws_peak.max(self.host.text.len());
        self.audio_events_peak = self.audio_events_peak.max(self.host.audio_events.len());
        Ok(result)
    }

    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// The VM program counter (code-word index) — the faulting PC after a trap.
    pub fn pc(&self) -> u32 {
        self.session.pc()
    }

    /// The number of frames stepped so far.
    pub fn frame(&self) -> usize {
        self.session.frame() as usize
    }

    /// A readable diagnostic for the current state (app ID, frame, PC, VM error).
    pub fn diagnostic(&self) -> AppDiagnostic {
        AppDiagnostic {
            app_id: self.app_id.clone(),
            frame: self.frame(),
            pc: self.pc(),
            vm_error: self.session.last_error(),
            source: self.source_location(),
        }
    }

    fn source_location(&self) -> Option<AppSourceLocation> {
        let map = koto_core::debug_map(&self.bytecode).ok().flatten()?;
        let pc = self.pc();
        let location = if pc > 0 {
            map.lookup_pc(pc - 1).or_else(|| map.lookup_pc(pc))
        } else {
            map.lookup_pc(pc)
        }?;
        Some(AppSourceLocation {
            pc: location.pc,
            file: location.file.to_string(),
            line: location.line,
            col: location.col,
        })
    }

    pub fn result(&self) -> VmRunResult {
        self.session.result()
    }

    pub fn has_exited(&self) -> bool {
        self.session.has_exited()
    }

    /// Draw rectangles recorded by the VM during the last frame.
    pub fn draw_rects(&self) -> &[(i32, i32, i32, i32, i32)] {
        &self.host.draw_rects
    }

    /// Pixel blits recorded by the VM during the last frame:
    /// `(x, y, w, h, little-endian RGB565 bytes)`.
    pub fn draw_pixels(&self) -> &[(i32, i32, i32, i32, Vec<u8>)] {
        &self.host.draw_pixels
    }

    pub fn persistent_pixels(&self) -> &[u8] {
        &self.host.persistent_pixels
    }

    /// Audio actions the VM issued during the last frame (sfx/bgm/submit), for
    /// deterministic inspection of scripted runs.
    pub fn audio_events(&self) -> &[AudioEvent] {
        &self.host.audio_events
    }

    /// The shared host audio engine handle, for the cpal output stream (window mode).
    pub fn audio_handle(&self) -> Arc<Mutex<SimAudio>> {
        Arc::clone(&self.audio)
    }

    /// Render `out.len()` mono samples from the host audio engine (headless capture).
    /// Deterministic: the synth has no device dependency.
    pub fn render_audio(&self, out: &mut [i16]) {
        if let Ok(mut audio) = self.audio.lock() {
            audio.render(out);
        }
    }

    /// Text draws recorded by the VM during the last frame.
    pub fn text(&self) -> &[(i32, i32, String)] {
        &self.host.text
    }

    /// Colour per text draw (index-aligned with [`Self::text`]);
    /// [`TEXT_COLOR_DEFAULT`] marks a colourless `draw_text`.
    pub fn text_colors(&self) -> &[i32] {
        &self.host.text_colors
    }

    /// Retained Game2D static/background layer (KOTO-0136): rect, pixel, and text
    /// draws captured between `game2d_static_begin`/`game2d_static_end`. These
    /// persist across frames and composite *beneath* the per-frame immediate
    /// lists. `static_text_colors` is index-aligned with `static_text`.
    pub fn static_rects(&self) -> &[(i32, i32, i32, i32, i32)] {
        &self.host.static_rects
    }

    pub fn static_pixels(&self) -> &[(i32, i32, i32, i32, Vec<u8>)] {
        &self.host.static_pixels
    }

    pub fn static_text(&self) -> &[(i32, i32, String)] {
        &self.host.static_text
    }

    pub fn static_text_colors(&self) -> &[i32] {
        &self.host.static_text_colors
    }

    /// Retained Game2D text layer (KOTO-0141): id-keyed text items composited in
    /// fixed z-order above the sprite layer and below the per-frame immediate text.
    /// `None` slots are hidden/unused.
    pub(super) fn game2d_text(&self) -> &[Option<Game2dText>] {
        &self.host.text_items
    }

    /// The host-side document the VM is editing through the text-buffer host calls.
    pub fn document(&self) -> &str {
        self.host.editor.as_str()
    }

    pub fn editor_scroll_row(&self) -> usize {
        self.host.editor.scroll_row()
    }

    pub fn editor_cursor_visible_row(&self) -> Option<u16> {
        self.host.editor.cursor_visible_row()
    }

    /// Toggle the editor's soft-wrap mode. Line wrapping is a host editor setting
    /// (like a terminal's), so the frontend flips it directly and the app renders
    /// what `edit_wrap`/`edit_hscroll_view` report.
    pub fn toggle_wrap(&mut self) {
        self.host.editor.toggle_wrap();
    }

    /// Whether the editor is currently soft-wrapping.
    pub fn is_wrap(&self) -> bool {
        self.host.editor.is_wrap()
    }

    /// The current IME composition line.
    pub fn ime_line(&self) -> MemoImeLine<'_> {
        self.host.ime.line()
    }

    /// A snapshot of VM and host state for the runtime inspector: run state, PC,
    /// fuel, last host call, last VM error, last input, and sandboxed file/draw
    /// counts. Reports occupancy and counts only — never host paths.
    pub fn inspect(&self) -> InspectorReport {
        InspectorReport {
            app_id: self.app_id.clone(),
            frame: self.frame(),
            run_state: self.session.result(),
            pc: self.session.pc(),
            frame_fuel_used: self.session.last_frame_fuel(),
            last_host_call: self.session.last_host_call(),
            last_vm_error: self.session.last_error(),
            last_input: self.session.last_input(),
            open_files: self.host.open_file_count(),
            draw_rects: self.host.draw_rects.len(),
            draw_pixels: self.host.draw_pixels.len(),
            text_draws: self.host.text.len(),
            audio_events: self.host.audio_events.len(),
        }
    }

    /// A per-app memory/fuel budget report: the VM's session high-water marks
    /// (operand stack, call depth, local slots, addressed heap, frame fuel, host
    /// calls/frame) and host working-set peaks, each paired with the capacity it
    /// is bounded by, so a scripted run can validate SRAM and fuel budgets before
    /// device bring-up (KOTO-0101). Capacities are the simulator's canonical VM
    /// profile; `heap_request` is the program's KBC heap request and `heap_budget`
    /// the manifest's declared per-app SRAM ceiling.
    pub fn budget(&self) -> AppBudgetReport {
        let vm = self.session.budget();
        AppBudgetReport {
            app_id: self.app_id.clone(),
            frames: self.frame(),
            stack_slots_peak: vm.stack_slots_peak,
            stack_slots_cap: SIM_VM_STACK_SLOTS as u16,
            call_depth_peak: vm.call_depth_peak,
            call_depth_cap: SIM_VM_CALL_DEPTH as u16,
            local_slots_peak: vm.local_slots_peak,
            local_slots_cap: koto_core::runtime::VM_LOCAL_SLOTS as u16,
            heap_bytes_peak: vm.heap_bytes_peak,
            heap_request: self.heap_request,
            heap_budget: self.sram_work_bytes,
            frame_fuel_peak: vm.frame_fuel_peak,
            frame_fuel_cap: SIM_FRAME_FUEL,
            host_calls_per_frame_peak: vm.host_calls_per_frame_peak,
            open_files_peak: self.open_files_peak,
            open_files_cap: SIM_MAX_OPEN_FILES,
            draw_rects_peak: self.draw_rects_peak,
            draw_pixels_peak: self.draw_pixels_peak,
            text_draws_peak: self.text_draws_peak,
            audio_events_peak: self.audio_events_peak,
        }
    }
}
