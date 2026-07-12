use super::*;

pub fn golden_frame_trace(root: impl AsRef<Path>) -> Result<String, SimError> {
    let root = root.as_ref();
    let packages = load_packages(root)?;
    let mut shell = ShellState::new(packages);
    let mut recorder = RenderRecorder::new();
    recorder.record_shell_list(&shell)?;

    let mut lines = Vec::new();
    lines.push("golden-frame-trace v1".to_string());
    lines.push("[shell:list]".to_string());
    lines.push(format!(
        "selected={} packages={}",
        shell.selected_index(),
        shell.packages().len()
    ));
    for command in recorder.commands() {
        lines.push(describe_render_command(command));
    }
    let list_commands = recorder.commands().len();
    let previous = shell.selected_index();
    shell.update(&input_state_for(HostInput::Right));
    recorder.record_shell_selection_change(&shell, previous)?;
    lines.push("[shell:selection-feedback]".to_string());
    lines.push(format!(
        "selected={} selection_frames={} page_frames={} sound={:?}",
        shell.selected_index(),
        shell.selection_feedback_frames(),
        shell.page_feedback_frames(),
        shell.take_pending_sound()
    ));
    for command in &recorder.commands()[list_commands..] {
        lines.push(describe_render_command(command));
    }

    let mut session = BytecodeAppSession::launch(root, "dev.koto.samples.hello-text")?;
    let result = session.step_frame(VmInputSnapshot::empty())?;
    lines.push("[app:dev.koto.samples.hello-text:frame1]".to_string());
    lines.push(format!("result={}", describe_vm_result(result)));
    for &(x, y, w, h, rgb565) in session.draw_rects() {
        lines.push(format!("draw_rect x={x} y={y} w={w} h={h} rgb565={rgb565}"));
    }
    for (x, y, text) in session.text() {
        lines.push(format!("draw_text x={x} y={y} text={text:?}"));
    }
    lines.push(String::new());
    Ok(lines.join("\n"))
}

fn describe_vm_result(result: VmRunResult) -> String {
    match result {
        VmRunResult::Yielded => "yielded".to_string(),
        VmRunResult::Exited(code) => format!("exited({code})"),
        VmRunResult::FuelExhausted => "fuel-exhausted".to_string(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostInput {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Cancel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShellScriptEvent {
    pub input: HostInput,
    pub selected_index: usize,
    pub action: ShellAction,
}

pub fn parse_input_script(text: &str) -> Result<Vec<HostInput>, SimError> {
    let mut inputs = Vec::new();
    for line in text.lines() {
        let script = line.split('#').next().unwrap_or("");
        for token in script.split_whitespace() {
            inputs.push(parse_input_token(token)?);
        }
    }
    Ok(inputs)
}

pub fn run_shell_script(shell: &mut ShellState, inputs: &[HostInput]) -> Vec<ShellScriptEvent> {
    inputs
        .iter()
        .map(|input| {
            let action = shell.update(&input_state_for(*input));
            ShellScriptEvent {
                input: *input,
                selected_index: shell.selected_index(),
                action,
            }
        })
        .collect()
}

pub fn describe_host_input(input: HostInput) -> &'static str {
    match input {
        HostInput::Up => "up",
        HostInput::Down => "down",
        HostInput::Left => "left",
        HostInput::Right => "right",
        HostInput::Confirm => "confirm",
        HostInput::Cancel => "cancel",
    }
}

pub fn describe_shell_action(action: ShellAction) -> String {
    match action {
        ShellAction::None => "no action".to_string(),
        ShellAction::Launch(package) => {
            format!(
                "launch requested: {} ({})",
                package.name(),
                package.app_id()
            )
        }
    }
}

fn parse_input_token(token: &str) -> Result<HostInput, SimError> {
    match token {
        "up" => Ok(HostInput::Up),
        "down" => Ok(HostInput::Down),
        "left" => Ok(HostInput::Left),
        "right" => Ok(HostInput::Right),
        "confirm" => Ok(HostInput::Confirm),
        "cancel" => Ok(HostInput::Cancel),
        _ => Err(SimError::InvalidInputScript),
    }
}

fn input_state_for(input: HostInput) -> InputState {
    let mut pressed = Buttons::default();
    match input {
        HostInput::Up => pressed.up = true,
        HostInput::Down => pressed.down = true,
        HostInput::Left => pressed.left = true,
        HostInput::Right => pressed.right = true,
        HostInput::Confirm => pressed.confirm = true,
        HostInput::Cancel => pressed.cancel = true,
    }

    InputState {
        pressed,
        ..InputState::default()
    }
}
