//! Bounded launcher-preference persistence to a root-level 8.3 file.

use embassy_rp::uart::UartTx;
use embedded_sdmmc::{BlockDevice, Mode, VolumeIdx, VolumeManager};
use koto_core::shell::{SaveStatus, SortMode};
use koto_core::ShellState;

use crate::firmware::config::{
    FirmwareClock, MAX_SHELL_PREFS_BYTES, SHELL_PREFS_COMPLETE, SHELL_PREFS_FILE,
    SHELL_PREFS_VERSION,
};
use crate::firmware::diag::uart_log;

/// Restore a complete, bounded preference snapshot. A truncated write lacks the
/// final marker and is ignored, leaving safe launcher defaults.
pub fn apply_shell_prefs<D>(
    volume_mgr: &VolumeManager<D, FirmwareClock>,
    shell: &mut ShellState,
    scratch: &mut [u8; MAX_SHELL_PREFS_BYTES],
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
{
    let Ok(volume) = volume_mgr.open_volume(VolumeIdx(0)) else {
        uart_log(uart, "phase=182 prefs-volume-open-error\r\n");
        return;
    };
    let Ok(root) = volume.open_root_dir() else {
        uart_log(uart, "phase=183 prefs-root-open-error\r\n");
        return;
    };
    let Ok(file) = root.open_file_in_dir(SHELL_PREFS_FILE, Mode::ReadOnly) else {
        uart_log(uart, "phase=141 prefs-missing\r\n");
        return;
    };
    if file.length() as usize > scratch.len() {
        uart_log(uart, "phase=184 prefs-oversize\r\n");
        return;
    }
    let mut length = 0usize;
    while !file.is_eof() && length < scratch.len() {
        match file.read(&mut scratch[length..]) {
            Ok(0) => break,
            Ok(count) => length += count,
            Err(_) => {
                uart_log(uart, "phase=185 prefs-read-error\r\n");
                return;
            }
        }
    }
    let Ok(text) = core::str::from_utf8(&scratch[..length]) else {
        uart_log(uart, "phase=186 prefs-invalid-utf8\r\n");
        return;
    };
    let mut lines = text.lines();
    if lines.next() != Some(SHELL_PREFS_VERSION)
        || text.lines().last() != Some(SHELL_PREFS_COMPLETE)
    {
        uart_log(uart, "phase=187 prefs-incomplete\r\n");
        return;
    }
    for line in lines {
        if line == SHELL_PREFS_COMPLETE {
            break;
        }
        if let Some(value) = line.strip_prefix("sort=") {
            shell.set_sort_mode(match value {
                "Name" => SortMode::Name,
                "Favorite" => SortMode::Favorite,
                _ => SortMode::Default,
            });
        } else if let Some(value) = line.strip_prefix("category=") {
            shell.set_category_filter(if value.is_empty() { None } else { Some(value) });
        } else if let Some(value) = line.strip_prefix("pane=") {
            shell.set_detail_pane_visible(value == "1");
        } else if let Some(app_id) = line.strip_prefix("fav=") {
            shell.set_favorite_by_app_id(app_id, true);
        }
    }
    shell.set_save_status(SaveStatus::Saved);
    uart_log(uart, "phase=142 prefs-applied\r\n");
}

/// Serialize and flush one complete preference snapshot. The final `end=1`
/// line lets boot distinguish a complete file from interrupted/truncated data.
pub fn save_shell_prefs<D>(
    volume_mgr: &VolumeManager<D, FirmwareClock>,
    shell: &ShellState,
    scratch: &mut [u8; MAX_SHELL_PREFS_BYTES],
    uart: &mut UartTx<'_, embassy_rp::uart::Blocking>,
) -> bool
where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
{
    let Some(length) = encode_shell_prefs(shell, scratch) else {
        uart_log(uart, "phase=188 prefs-encode-overflow\r\n");
        return false;
    };
    let Ok(volume) = volume_mgr.open_volume(VolumeIdx(0)) else {
        uart_log(uart, "phase=182 prefs-volume-open-error\r\n");
        return false;
    };
    let Ok(root) = volume.open_root_dir() else {
        uart_log(uart, "phase=183 prefs-root-open-error\r\n");
        return false;
    };
    let Ok(file) = root.open_file_in_dir(SHELL_PREFS_FILE, Mode::ReadWriteCreateOrTruncate) else {
        uart_log(uart, "phase=189 prefs-open-write-error\r\n");
        return false;
    };
    if file.write(&scratch[..length]).is_err() || file.flush().is_err() {
        uart_log(uart, "phase=190 prefs-write-error\r\n");
        return false;
    }
    uart_log(uart, "phase=143 prefs-saved\r\n");
    true
}

fn encode_shell_prefs(
    shell: &ShellState,
    output: &mut [u8; MAX_SHELL_PREFS_BYTES],
) -> Option<usize> {
    let mut cursor = 0usize;
    append_pref_line(output, &mut cursor, SHELL_PREFS_VERSION)?;
    append_pref_line(
        output,
        &mut cursor,
        match shell.sort_mode() {
            SortMode::Default => "sort=Default",
            SortMode::Name => "sort=Name",
            SortMode::Favorite => "sort=Favorite",
        },
    )?;
    append_pref_pair(
        output,
        &mut cursor,
        "pane=",
        if shell.detail_pane_visible() {
            "1"
        } else {
            "0"
        },
    )?;
    if let Some(category) = shell.category_filter() {
        append_pref_pair(output, &mut cursor, "category=", category)?;
    }
    for package in shell.packages().iter() {
        if package.is_favorite() {
            append_pref_pair(output, &mut cursor, "fav=", package.app_id())?;
        }
    }
    append_pref_line(output, &mut cursor, SHELL_PREFS_COMPLETE)?;
    Some(cursor)
}

fn append_pref_pair(output: &mut [u8], cursor: &mut usize, key: &str, value: &str) -> Option<()> {
    append_pref_bytes(output, cursor, key.as_bytes())?;
    append_pref_bytes(output, cursor, value.as_bytes())?;
    append_pref_bytes(output, cursor, b"\n")
}

fn append_pref_line(output: &mut [u8], cursor: &mut usize, line: &str) -> Option<()> {
    append_pref_bytes(output, cursor, line.as_bytes())?;
    append_pref_bytes(output, cursor, b"\n")
}

fn append_pref_bytes(output: &mut [u8], cursor: &mut usize, bytes: &[u8]) -> Option<()> {
    let end = cursor.checked_add(bytes.len())?;
    if end > output.len() {
        return None;
    }
    output[*cursor..end].copy_from_slice(bytes);
    *cursor = end;
    Some(())
}
