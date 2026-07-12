use super::*;

/// Sandbox-safe namespace used to persist shell preferences under the save-data
/// area (`data/dev.koto.shell/prefs.txt`).
pub const SHELL_PREFS_APP_ID: &str = "dev.koto.shell";

fn shell_prefs_path(root: &Path) -> PathBuf {
    root.join(SAVE_DATA_ROOT)
        .join(SHELL_PREFS_APP_ID)
        .join("prefs.txt")
}

fn sort_mode_tag(mode: SortMode) -> &'static str {
    match mode {
        SortMode::Default => "Default",
        SortMode::Name => "Name",
        SortMode::Favorite => "Favorite",
    }
}

fn sort_mode_from_tag(tag: &str) -> SortMode {
    match tag {
        "Name" => SortMode::Name,
        "Favorite" => SortMode::Favorite,
        _ => SortMode::Default,
    }
}

/// Apply persisted shell preferences (sort, category, favorites) to `shell`.
/// Missing or unreadable preferences leave the shell at its defaults.
pub fn apply_shell_prefs(shell: &mut ShellState, root: impl AsRef<Path>) {
    let Ok(text) = fs::read_to_string(shell_prefs_path(root.as_ref())) else {
        return;
    };
    for line in text.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("sort=") {
            shell.set_sort_mode(sort_mode_from_tag(value));
        } else if let Some(value) = line.strip_prefix("category=") {
            shell.set_category_filter(if value.is_empty() { None } else { Some(value) });
        } else if let Some(app_id) = line.strip_prefix("fav=") {
            shell.set_favorite_by_app_id(app_id, true);
        }
    }
}

/// Persist shell preferences (sort, category, favorites) to the save-data area.
pub fn save_shell_prefs(shell: &ShellState, root: impl AsRef<Path>) -> Result<(), SimError> {
    let path = shell_prefs_path(root.as_ref());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| SimError::Io)?;
    }
    let mut out = String::new();
    out.push_str("sort=");
    out.push_str(sort_mode_tag(shell.sort_mode()));
    out.push('\n');
    if let Some(category) = shell.category_filter() {
        out.push_str("category=");
        out.push_str(category);
        out.push('\n');
    }
    for package in shell.packages().iter() {
        if package.is_favorite() {
            out.push_str("fav=");
            out.push_str(package.app_id());
            out.push('\n');
        }
    }
    fs::write(&path, out).map_err(|_| SimError::Io)?;
    Ok(())
}
