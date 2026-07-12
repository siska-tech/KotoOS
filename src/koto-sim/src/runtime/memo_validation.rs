use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoValidationReport {
    pub shell_launched: bool,
    pub ime_before_commit: MemoImeSnapshot,
    pub document_after_commit: String,
    pub saved_document: String,
    pub reloaded_document: String,
    pub saved_path: String,
    pub sandbox_escape_found: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoImeSnapshot {
    pub mode: MemoImeMode,
    pub pending_romaji: String,
    pub reading: String,
    pub candidate: Option<String>,
    pub sticky_shift_armed: bool,
}

/// End-to-end validation of the bytecode memo app: it launches from KotoShell,
/// then a scripted scenario drives the live VM through romaji/kana, Sticky Shift,
/// SKK conversion and commit, cursor movement, deletion, save, and exit, before a
/// fresh launch reloads the saved file. Editing and IME run entirely through the
/// VM's host calls — nothing here drives the Rust editor/IME directly.
pub fn run_memo_validation(root: impl AsRef<Path>) -> Result<MemoValidationReport, SimError> {
    let root = root.as_ref();

    // 1. Prove the app launches from KotoShell.
    let packages = load_packages(root)?;
    let mut shell = ShellState::new(packages);
    let memo_index = shell
        .packages()
        .iter()
        .position(|package| package.app_id() == MEMO_APP_ID)
        .ok_or(SimError::MemoValidationFailed)?;
    let mut nav = Vec::new();
    nav.extend(std::iter::repeat_n(HostInput::Right, memo_index));
    nav.push(HostInput::Confirm);
    let shell_launched = run_shell_script(&mut shell, &nav).iter().any(|event| {
        matches!(&event.action, ShellAction::Launch(package) if package.app_id() == MEMO_APP_ID)
    });
    if !shell_launched {
        return Err(SimError::MemoValidationFailed);
    }

    // 2. Drive the bytecode app session frame by frame.
    let mut session = BytecodeAppSession::launch(root, MEMO_APP_ID)
        .map_err(|_| SimError::MemoValidationFailed)?;
    let char_frame = |session: &mut BytecodeAppSession, ch: char| -> Result<(), SimError> {
        let input = VmInputSnapshot {
            text_codepoint: ch as u32,
            ..VmInputSnapshot::empty()
        };
        session
            .step_frame(input)
            .map(|_| ())
            .map_err(|_| SimError::MemoValidationFailed)
    };
    let intent_frame = |session: &mut BytecodeAppSession, bits: u32| -> Result<(), SimError> {
        let input = VmInputSnapshot {
            intent_bits: bits,
            ..VmInputSnapshot::empty()
        };
        session
            .step_frame(input)
            .map(|_| ())
            .map_err(|_| SimError::MemoValidationFailed)
    };
    use koto_core::runtime::text_intent as ti;

    // Start in ASCII mode, then explicitly enable IME for Japanese entry.
    intent_frame(&mut session, ti::IME_TOGGLE)?;
    // Romaji/kana straight into the document: k, a -> か.
    char_frame(&mut session, 'k')?;
    char_frame(&mut session, 'a')?;
    // Sticky Shift starts SKK conversion: Shift, then k, a, s, a -> reading かさ.
    intent_frame(&mut session, ti::SHIFT)?;
    for ch in ['k', 'a', 's', 'a'] {
        char_frame(&mut session, ch)?;
    }
    // Convert and capture the candidate before commit.
    intent_frame(&mut session, ti::CONVERT)?;
    let ime_before_commit = MemoImeSnapshot::from_line(session.ime_line());
    if ime_before_commit.candidate.as_deref() != Some("傘") {
        return Err(SimError::MemoValidationFailed);
    }
    // Commit the candidate into the document: か傘.
    intent_frame(&mut session, ti::COMMIT)?;
    let document_after_commit = session.document().to_string();
    // Cursor movement and deletion: move left over 傘, backspace か -> 傘.
    intent_frame(&mut session, ti::LEFT)?;
    intent_frame(&mut session, ti::BACKSPACE)?;
    // Save and exit.
    intent_frame(&mut session, ti::EXIT)?;
    if !session.has_exited() {
        return Err(SimError::MemoValidationFailed);
    }

    // 3. The save must land only inside the app sandbox.
    let saved_path = sandboxed_data_path(MEMO_APP_ID, MEMO_FILE_PATH)?;
    let saved_host_path = root.join(&saved_path);
    let saved_document =
        fs::read_to_string(&saved_host_path).map_err(|_| SimError::MemoValidationFailed)?;
    let sandbox_escape_found =
        root.join(MEMO_FILE_PATH).exists() || root.join("data").join(MEMO_FILE_PATH).exists();

    // 4. Relaunch: a fresh session reloads the saved file on its first frame.
    let mut relaunch = BytecodeAppSession::launch(root, MEMO_APP_ID)
        .map_err(|_| SimError::MemoValidationFailed)?;
    relaunch
        .step_frame(VmInputSnapshot::empty())
        .map_err(|_| SimError::MemoValidationFailed)?;
    let reloaded_document = relaunch.document().to_string();

    Ok(MemoValidationReport {
        shell_launched,
        ime_before_commit,
        document_after_commit,
        saved_document,
        reloaded_document,
        saved_path,
        sandbox_escape_found,
    })
}

pub fn describe_memo_validation_report(report: &MemoValidationReport) -> String {
    format!(
        "memo validation shell_launched={} ime={:?} reading={} candidate={} after_commit={} saved={:?} reloaded={:?} path={} sandbox_escape={}",
        report.shell_launched,
        report.ime_before_commit.mode,
        report.ime_before_commit.reading,
        report.ime_before_commit.candidate.as_deref().unwrap_or("<none>"),
        report.document_after_commit,
        report.saved_document,
        report.reloaded_document,
        report.saved_path,
        report.sandbox_escape_found
    )
}

impl MemoImeSnapshot {
    pub(super) fn from_line(line: MemoImeLine<'_>) -> Self {
        Self {
            mode: line.mode,
            pending_romaji: line.pending_romaji.to_string(),
            reading: line.reading.to_string(),
            candidate: line.candidate.map(str::to_string),
            sticky_shift_armed: line.sticky_shift_armed,
        }
    }
}

pub(super) fn text_editor(
    content_cols: u16,
    content_rows: u16,
) -> Option<MemoEditor<MEMO_DOC_CAPACITY>> {
    let cell_width = 6u16;
    let cell_height = 13u16;
    let height = content_rows.checked_add(3)?.checked_mul(cell_height)?;
    let width = content_cols.checked_mul(cell_width)?;
    let layout = koto_core::TextLayout::new(
        RenderSurface::new(width, height, PixelFormat::Rgb565),
        CellMetrics {
            cell_width,
            cell_height,
        },
        2,
    )
    .ok()?;
    Some(MemoEditor::new(layout))
}

fn sandboxed_data_path(app_id: &str, path: &str) -> Result<String, SimError> {
    let sandbox = Sandbox::new(app_id).map_err(|_| SimError::MemoValidationFailed)?;
    let path = sandbox
        .resolve(path)
        .map_err(|_| SimError::MemoValidationFailed)?;
    Ok(format!(
        "{SAVE_DATA_ROOT}/{}/{}",
        sandbox.app_id(),
        path.as_str()
    ))
}

pub(super) fn sandboxed_app_data_path(app_id: &str) -> Result<String, SimError> {
    validate_app_id(app_id).map_err(|_| SimError::InvalidManifest)?;
    let sandbox = Sandbox::new(app_id).map_err(|_| SimError::InvalidManifest)?;
    Ok(format!("{SAVE_DATA_ROOT}/{}", sandbox.app_id()))
}
