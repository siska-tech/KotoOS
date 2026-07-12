use super::*;

/// Frames per second the simulator presents at; audio capture renders one frame's
/// worth of samples per stepped frame so the captured timeline tracks gameplay.
pub const SIM_FPS: u32 = 60;

/// Run `app_id` through `inputs` (or to idle/exit when empty) and render its host
/// audio engine into a single mono i16 timeline, one frame's worth of samples per
/// stepped frame. Deterministic — the synth has no device dependency — so it backs
/// scripted/golden audio capture without a real device. Returns `(sample_rate,
/// samples)`.
pub fn capture_app_audio(
    root: impl AsRef<Path>,
    app_id: &str,
    inputs: &[VmInputSnapshot],
) -> Result<(u32, Vec<i16>), AppRunError> {
    let mut session = BytecodeAppSession::launch(root, app_id)
        .map_err(|error| AppRunError::Launch(Box::new(AppFailureSummary::launch(app_id, error))))?;
    let sample_rate = session
        .audio_handle()
        .lock()
        .map(|audio| audio.sample_rate())
        .unwrap_or(DEFAULT_SAMPLE_RATE);
    let trap = |session: &BytecodeAppSession| {
        AppRunError::Trap(Box::new(AppFailureSummary::trap(session.diagnostic())))
    };

    let mut samples = Vec::new();
    let mut render_through_frame = |session: &BytecodeAppSession, frame: usize| {
        // Floor-divide so per-frame counts sum exactly to `frames * rate / fps`,
        // keeping the timeline drift-free.
        let start = (frame as u64 * sample_rate as u64 / SIM_FPS as u64) as usize;
        let end = ((frame as u64 + 1) * sample_rate as u64 / SIM_FPS as u64) as usize;
        let mut chunk = vec![0i16; end - start];
        session.render_audio(&mut chunk);
        samples.extend_from_slice(&chunk);
    };

    if inputs.is_empty() {
        while !session.has_exited() && session.frame() < SIM_APP_IDLE_FRAME_CAP {
            let frame = session.frame();
            if session.step_frame(VmInputSnapshot::empty()).is_err() {
                return Err(trap(&session));
            }
            render_through_frame(&session, frame);
        }
    } else {
        for input in inputs {
            if session.has_exited() {
                break;
            }
            let frame = session.frame();
            if session.step_frame(*input).is_err() {
                return Err(trap(&session));
            }
            render_through_frame(&session, frame);
        }
    }
    Ok((sample_rate, samples))
}
