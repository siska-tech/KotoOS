use super::*;

/// A readable runtime diagnostic for a running or trapped app.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppDiagnostic {
    pub app_id: String,
    pub frame: usize,
    pub pc: u32,
    pub vm_error: Option<koto_core::VmError>,
    pub source: Option<AppSourceLocation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppSourceLocation {
    pub pc: u32,
    pub file: String,
    pub line: u32,
    pub col: u32,
}

impl AppDiagnostic {
    pub fn describe(&self) -> String {
        let source = self
            .source
            .as_ref()
            .map(|source| {
                format!(
                    " (source {}:{}:{} pc {})",
                    source.file, source.line, source.col, source.pc
                )
            })
            .unwrap_or_default();
        match self.vm_error {
            Some(error) => format!(
                "app {} trapped at frame {} pc {}{}: {:?}",
                self.app_id, self.frame, self.pc, source, error
            ),
            None => format!(
                "app {} fault at frame {} pc {}{}",
                self.app_id, self.frame, self.pc, source
            ),
        }
    }
}

/// User-facing failure category for a bytecode app run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppFailureKind {
    LaunchFailed,
    VerificationFailed,
    RuntimeTrap,
    RuntimeInitFailed,
}

impl AppFailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AppFailureKind::LaunchFailed => "launch-failed",
            AppFailureKind::VerificationFailed => "verification-failed",
            AppFailureKind::RuntimeTrap => "runtime-trap",
            AppFailureKind::RuntimeInitFailed => "runtime-init-failed",
        }
    }
}

/// A shell/window/CLI-readable app failure. It keeps the app identity and the
/// failure category separate from diagnostic text so frontends can render a
/// controlled recovery screen without parsing arbitrary error strings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppFailureSummary {
    pub app_id: String,
    pub kind: AppFailureKind,
    pub detail: String,
    pub diagnostic: Option<AppDiagnostic>,
}

impl AppFailureSummary {
    pub fn launch(app_id: &str, error: SimError) -> Self {
        let kind = match error {
            SimError::RuntimeVerifyFailed => AppFailureKind::VerificationFailed,
            SimError::RuntimeExecutionFailed => AppFailureKind::RuntimeInitFailed,
            _ => AppFailureKind::LaunchFailed,
        };
        Self {
            app_id: app_id.to_string(),
            kind,
            detail: format!("{error:?}"),
            diagnostic: None,
        }
    }

    pub fn trap(diagnostic: AppDiagnostic) -> Self {
        let detail = diagnostic.describe();
        Self {
            app_id: diagnostic.app_id.clone(),
            kind: AppFailureKind::RuntimeTrap,
            detail,
            diagnostic: Some(diagnostic),
        }
    }

    pub fn describe(&self) -> String {
        format!(
            "app {} failure kind={} detail={}",
            self.app_id,
            self.kind.as_str(),
            self.detail
        )
    }
}

/// Failure to run a scripted app scenario: either the launch failed or the VM
/// trapped mid-scenario, both represented as shell-visible summaries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppRunError {
    Launch(Box<AppFailureSummary>),
    Trap(Box<AppFailureSummary>),
}

impl AppRunError {
    pub fn describe(&self) -> String {
        match self {
            AppRunError::Launch(summary) | AppRunError::Trap(summary) => summary.describe(),
        }
    }

    pub fn summary(&self) -> &AppFailureSummary {
        match self {
            AppRunError::Launch(summary) | AppRunError::Trap(summary) => summary,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppScenarioReport {
    pub app_id: String,
    pub frames: usize,
    pub result: VmRunResult,
    pub document: String,
    pub ime: MemoImeSnapshot,
    /// Runtime inspector snapshot taken after the final frame.
    pub inspector: InspectorReport,
    /// Per-app memory/fuel budget high-water marks across the whole run (KOTO-0101).
    pub budget: AppBudgetReport,
}

/// Launch `app_id` and drive it through one input snapshot per frame, stopping
/// when the app exits or the script ends. Faithful to the runtime path: input is
/// the same [`VmInputSnapshot`] model window mode feeds the live VM.
pub fn run_app_scenario(
    root: impl AsRef<Path>,
    app_id: &str,
    inputs: &[VmInputSnapshot],
) -> Result<AppScenarioReport, AppRunError> {
    let mut session = BytecodeAppSession::launch(root, app_id)
        .map_err(|error| AppRunError::Launch(Box::new(AppFailureSummary::launch(app_id, error))))?;
    if inputs.is_empty() {
        // No script: run the app to exit (bounded), feeding empty frames.
        while !session.has_exited() && session.frame() < SIM_APP_IDLE_FRAME_CAP {
            if session.step_frame(VmInputSnapshot::empty()).is_err() {
                return Err(AppRunError::Trap(Box::new(AppFailureSummary::trap(
                    session.diagnostic(),
                ))));
            }
        }
    } else {
        // Scripted: one frame per input, exactly as written (include an `exit`
        // intent frame to make the app quit).
        for input in inputs {
            if session.has_exited() {
                break;
            }
            if session.step_frame(*input).is_err() {
                return Err(AppRunError::Trap(Box::new(AppFailureSummary::trap(
                    session.diagnostic(),
                ))));
            }
        }
    }
    Ok(AppScenarioReport {
        app_id: session.app_id().to_string(),
        frames: session.frame(),
        result: session.result(),
        document: session.document().to_string(),
        ime: MemoImeSnapshot::from_line(session.ime_line()),
        inspector: session.inspect(),
        budget: session.budget(),
    })
}

pub fn describe_app_scenario_report(report: &AppScenarioReport) -> String {
    let result = match report.result {
        VmRunResult::Yielded => "yielded".to_string(),
        VmRunResult::Exited(code) => format!("exited({code})"),
        VmRunResult::FuelExhausted => "fuel-exhausted".to_string(),
    };
    format!(
        "app {} frames={} -> {} document_bytes={} ime={:?} candidate={}",
        report.app_id,
        report.frames,
        result,
        report.document.len(),
        report.ime.mode,
        report.ime.candidate.as_deref().unwrap_or("<none>"),
    )
}

/// Parse a per-frame app input script. Each non-empty line is one frame; tokens
/// are a single-quoted character (`'a'`, `'\n'`) and/or intent names (`shift`,
/// `convert`, `commit`, `cancel`, `backspace`, `delete`, `left`, `right`, `up`,
/// `down`, `home`, `end`, `newline`, `save`, `exit`, `ime-toggle`). The bare
/// token `frame` denotes an empty frame. `#` starts a comment.
pub fn parse_app_script(text: &str) -> Result<Vec<VmInputSnapshot>, SimError> {
    let mut frames = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut snapshot = VmInputSnapshot::empty();
        for token in line.split_whitespace() {
            if token == "frame" {
                continue;
            } else if let Some(codepoint) = parse_char_token(token) {
                snapshot.text_codepoint = codepoint;
            } else if let Some(bit) = intent_bit(token) {
                snapshot.intent_bits |= bit;
            } else {
                return Err(SimError::InvalidInputScript);
            }
        }
        frames.push(snapshot);
    }
    Ok(frames)
}

fn parse_char_token(token: &str) -> Option<u32> {
    let inner = token.strip_prefix('\'')?.strip_suffix('\'')?;
    let mut chars = inner.chars();
    let first = chars.next()?;
    let ch = if first == '\\' {
        let escape = chars.next()?;
        match escape {
            'n' => '\n',
            't' => '\t',
            '\\' => '\\',
            '\'' => '\'',
            's' => ' ',
            _ => return None,
        }
    } else {
        first
    };
    if chars.next().is_some() {
        return None;
    }
    Some(ch as u32)
}

fn intent_bit(token: &str) -> Option<u32> {
    use koto_core::runtime::text_intent as ti;
    Some(match token {
        "shift" => ti::SHIFT,
        "convert" => ti::CONVERT,
        "commit" => ti::COMMIT,
        "cancel" => ti::CANCEL,
        "backspace" => ti::BACKSPACE,
        "delete" => ti::DELETE,
        "left" => ti::LEFT,
        "right" => ti::RIGHT,
        "up" => ti::UP,
        "down" => ti::DOWN,
        "home" => ti::HOME,
        "end" => ti::END,
        "newline" => ti::NEWLINE,
        "save" => ti::SAVE,
        "exit" => ti::EXIT,
        "ime-toggle" | "ime" => ti::IME_TOGGLE,
        "open" => ti::OPEN,
        "new" => ti::NEW,
        _ => return None,
    })
}
