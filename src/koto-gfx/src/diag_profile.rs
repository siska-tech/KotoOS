//! Runtime diagnostic verbosity profiles (DIAG-0001 Stage 1).
//!
//! The firmware's per-frame UART diagnostics grew, issue by issue (GFX-0008 →
//! GFX-0011), into up to nine distinct lines on the same throttled render cadence.
//! Together their transmit cost perturbs the very `phase=160` timing they exist to
//! measure. This module factors the *verbosity* axis out into a small compile-time
//! constant so a perf run can quiet the render/audio/CodeWindow chatter without
//! deleting any diagnostic and without touching PSRAM backend selection.
//!
//! It lives in KotoGFX (beside the diagnostics *model* it gates —
//! [`DirtyRectGeometry`](crate::DirtyRectGeometry),
//! [`CoalescePressure`](crate::CoalescePressure)) so the pure profile logic is
//! host-testable; the firmware re-exports it from `config.rs` and pins the active
//! [`DiagProfile`] there. Everything here is `const fn` / POD, so a disabled branch
//! is dead-code-eliminated in the firmware (zero RAM; quieting logs shrinks `.text`).

/// A diagnostic class: the smallest verbosity band a per-cadence emit site belongs
/// to. A [`DiagProfile`] is a *set* of classes; a line emits only when its class is
/// enabled by the active profile.
///
/// The always-on boot / launch / app-exit / fault / panic spine is **not** a class —
/// it is structural and emits under every profile (including [`DiagProfile::Quiet`]),
/// so no profile can hide a boot fault or a crash.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiagClass {
    /// The regression-detecting population: the `phase=160` headline plus the
    /// thinned `phase=168` budget verdict. Enough to spot a perf/budget regression.
    PerfDefault,
    /// Dirty-rect / coalesce / count-shift geometry (`phase=164/169/171/174`, and
    /// the `phase=170` static rebuild). Only meaningful while investigating a
    /// repaint decision.
    Gfx,
    /// Audio hostcall trace + aggregate summary (`phase=173`; the per-call
    /// `phase=172` joins in Stage 2). Only while investigating drops/underruns.
    Audio,
    /// PSRAM CodeWindow refill histogram + fast-read counters (`phase=163/167`).
    /// Only while investigating PSRAM refill cost.
    CodeWindow,
    /// The bring-up firehose (first-frame command dump, heartbeat, draw-usage —
    /// routed in Stage 2). Present now so [`DiagProfile::Verbose`] is a faithful
    /// A/B baseline for today's dev emit set.
    Verbose,
}

/// A named diagnostic verbosity profile (DIAG-0001 §4). Selecting a profile emits
/// the always-on spine plus that profile's classes — profiles are *additive* over
/// the spine, so the "did it boot / did it crash" signal is never silenced.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiagProfile {
    /// No cadence classes: pure timing runs / demos. Only the always-on spine.
    Quiet,
    /// **Default.** `perf-default` only — a clean `phase=160` (+ thinned `phase=168`)
    /// for normal development and performance smoke, with no per-cadence chatter.
    Perf,
    /// `perf-default` + `gfx-debug`: dirty-rect / coalescing / count-shift work.
    Gfx,
    /// `perf-default` + `audio-debug`: drops / underruns / event investigations.
    Audio,
    /// `perf-default` + `codewindow-debug`: PSRAM refill-cost investigations.
    CodeWindow,
    /// All classes: the bring-up firehose. Reproduces today's dev emit set and the
    /// 30-frame cadence, so a Verbose build is a byte-identical A/B against `main`.
    Verbose,
}

impl DiagProfile {
    /// Whether this profile emits the given diagnostic [`DiagClass`].
    ///
    /// `Quiet` enables nothing; `Perf` enables only `PerfDefault`; each debug
    /// profile adds its one class to `PerfDefault`; `Verbose` enables everything.
    pub const fn enables(self, class: DiagClass) -> bool {
        match self {
            DiagProfile::Quiet => false,
            DiagProfile::Perf => matches!(class, DiagClass::PerfDefault),
            DiagProfile::Gfx => matches!(class, DiagClass::PerfDefault | DiagClass::Gfx),
            DiagProfile::Audio => matches!(class, DiagClass::PerfDefault | DiagClass::Audio),
            DiagProfile::CodeWindow => {
                matches!(class, DiagClass::PerfDefault | DiagClass::CodeWindow)
            }
            DiagProfile::Verbose => true,
        }
    }

    /// The render sample cadence `C`: the per-cadence lines emit on frame 1 and every
    /// `C` frames thereafter.
    ///
    /// `Quiet`/`Perf` keep the slow `120` (you want minimal chatter on a timing run);
    /// the debug profiles and `Verbose` use the denser `30` (you want more samples
    /// while investigating). `Verbose`'s `30` matches today's dev build. Decoupled
    /// from PSRAM backend selection — this used to be welded to the
    /// `psram_qpi_code_window_prod_profile` cargo feature (DIAG-0001 §"Problem").
    pub const fn sample_period(self) -> u32 {
        match self {
            DiagProfile::Quiet | DiagProfile::Perf => 120,
            DiagProfile::Gfx
            | DiagProfile::Audio
            | DiagProfile::CodeWindow
            | DiagProfile::Verbose => 30,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_CLASSES: [DiagClass; 5] = [
        DiagClass::PerfDefault,
        DiagClass::Gfx,
        DiagClass::Audio,
        DiagClass::CodeWindow,
        DiagClass::Verbose,
    ];

    fn enabled_set(profile: DiagProfile) -> Vec<DiagClass> {
        ALL_CLASSES
            .iter()
            .copied()
            .filter(|&c| profile.enables(c))
            .collect()
    }

    #[test]
    fn quiet_enables_no_class() {
        assert_eq!(enabled_set(DiagProfile::Quiet), Vec::<DiagClass>::new());
    }

    #[test]
    fn perf_enables_only_perf_default() {
        assert_eq!(enabled_set(DiagProfile::Perf), vec![DiagClass::PerfDefault]);
    }

    #[test]
    fn gfx_enables_perf_default_plus_gfx_only() {
        assert_eq!(
            enabled_set(DiagProfile::Gfx),
            vec![DiagClass::PerfDefault, DiagClass::Gfx]
        );
    }

    #[test]
    fn audio_enables_perf_default_plus_audio_only() {
        assert_eq!(
            enabled_set(DiagProfile::Audio),
            vec![DiagClass::PerfDefault, DiagClass::Audio]
        );
    }

    #[test]
    fn codewindow_enables_perf_default_plus_codewindow_only() {
        assert_eq!(
            enabled_set(DiagProfile::CodeWindow),
            vec![DiagClass::PerfDefault, DiagClass::CodeWindow]
        );
    }

    #[test]
    fn verbose_enables_every_class() {
        assert_eq!(enabled_set(DiagProfile::Verbose), ALL_CLASSES.to_vec());
    }

    #[test]
    fn perf_default_is_the_common_spine_of_every_emitting_profile() {
        // Every profile that emits anything at all must carry the regression detector.
        for profile in [
            DiagProfile::Perf,
            DiagProfile::Gfx,
            DiagProfile::Audio,
            DiagProfile::CodeWindow,
            DiagProfile::Verbose,
        ] {
            assert!(
                profile.enables(DiagClass::PerfDefault),
                "{profile:?} must keep phase=160 / perf-default"
            );
        }
    }

    #[test]
    fn debug_profiles_do_not_leak_into_each_others_classes() {
        assert!(!DiagProfile::Gfx.enables(DiagClass::Audio));
        assert!(!DiagProfile::Gfx.enables(DiagClass::CodeWindow));
        assert!(!DiagProfile::Audio.enables(DiagClass::Gfx));
        assert!(!DiagProfile::Audio.enables(DiagClass::CodeWindow));
        assert!(!DiagProfile::CodeWindow.enables(DiagClass::Gfx));
        assert!(!DiagProfile::CodeWindow.enables(DiagClass::Audio));
        // None of the single-debug profiles turn on the verbose firehose.
        assert!(!DiagProfile::Gfx.enables(DiagClass::Verbose));
        assert!(!DiagProfile::Audio.enables(DiagClass::Verbose));
        assert!(!DiagProfile::CodeWindow.enables(DiagClass::Verbose));
    }

    #[test]
    fn sample_period_matches_the_profile_table() {
        assert_eq!(DiagProfile::Quiet.sample_period(), 120);
        assert_eq!(DiagProfile::Perf.sample_period(), 120);
        assert_eq!(DiagProfile::Gfx.sample_period(), 30);
        assert_eq!(DiagProfile::Audio.sample_period(), 30);
        assert_eq!(DiagProfile::CodeWindow.sample_period(), 30);
        // Verbose keeps today's dev cadence so it is a faithful A/B baseline.
        assert_eq!(DiagProfile::Verbose.sample_period(), 30);
    }
}
