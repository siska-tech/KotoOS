use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SaveDataNamespace {
    pub app_id: String,
    pub file_count: usize,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SaveDataClearReport {
    pub app_id: String,
    pub existed: bool,
}

/// List app save-data namespaces under `data/<app_id>` without returning host
/// filesystem paths.
pub fn list_save_data(root: impl AsRef<Path>) -> Result<Vec<SaveDataNamespace>, SimError> {
    let data_root = root.as_ref().join(SAVE_DATA_ROOT);
    if !data_root.exists() {
        return Ok(Vec::new());
    }
    if !data_root.is_dir() {
        return Err(SimError::Io);
    }

    let mut namespaces = Vec::new();
    for entry in fs::read_dir(data_root).map_err(|_| SimError::Io)? {
        let entry = entry.map_err(|_| SimError::Io)?;
        let file_type = entry.file_type().map_err(|_| SimError::Io)?;
        if !file_type.is_dir() {
            continue;
        }
        let app_id = entry
            .file_name()
            .to_str()
            .ok_or(SimError::InvalidManifest)?
            .to_string();
        validate_app_id(&app_id).map_err(|_| SimError::InvalidManifest)?;
        Sandbox::new(&app_id).map_err(|_| SimError::InvalidManifest)?;
        let (file_count, total_bytes) = save_data_usage(&entry.path())?;
        namespaces.push(SaveDataNamespace {
            app_id,
            file_count,
            total_bytes,
        });
    }
    namespaces.sort_by(|left, right| left.app_id.cmp(&right.app_id));
    Ok(namespaces)
}

/// Clear one app's save-data namespace under `data/<app_id>`.
pub fn clear_save_data(
    root: impl AsRef<Path>,
    app_id: &str,
) -> Result<SaveDataClearReport, SimError> {
    let virtual_path = sandboxed_app_data_path(app_id)?;
    let target = root.as_ref().join(virtual_path);
    if !target.exists() {
        return Ok(SaveDataClearReport {
            app_id: app_id.to_string(),
            existed: false,
        });
    }
    if !target.is_dir() {
        return Err(SimError::Io);
    }
    fs::remove_dir_all(&target).map_err(|_| SimError::Io)?;
    Ok(SaveDataClearReport {
        app_id: app_id.to_string(),
        existed: true,
    })
}

pub fn describe_save_data_namespace(namespace: &SaveDataNamespace) -> String {
    format!(
        "save-data {} files={} bytes={}",
        namespace.app_id, namespace.file_count, namespace.total_bytes
    )
}

pub fn describe_save_data_clear_report(report: &SaveDataClearReport) -> String {
    if report.existed {
        format!("cleared save-data {}", report.app_id)
    } else {
        format!("save-data {} already empty", report.app_id)
    }
}

fn save_data_usage(path: &Path) -> Result<(usize, u64), SimError> {
    let mut file_count = 0;
    let mut total_bytes = 0;
    for entry in fs::read_dir(path).map_err(|_| SimError::Io)? {
        let entry = entry.map_err(|_| SimError::Io)?;
        let file_type = entry.file_type().map_err(|_| SimError::Io)?;
        if file_type.is_dir() {
            let (child_count, child_bytes) = save_data_usage(&entry.path())?;
            file_count += child_count;
            total_bytes += child_bytes;
        } else if file_type.is_file() {
            file_count += 1;
            total_bytes += entry.metadata().map_err(|_| SimError::Io)?.len();
        }
    }
    Ok((file_count, total_bytes))
}

pub(super) fn save_data_present(root: &Path, app_id: &str) -> Result<bool, SimError> {
    let virtual_path = sandboxed_app_data_path(app_id)?;
    let target = root.join(virtual_path);
    if !target.exists() {
        return Ok(false);
    }
    if !target.is_dir() {
        return Err(SimError::Io);
    }
    let (file_count, total_bytes) = save_data_usage(&target)?;
    Ok(file_count > 0 || total_bytes > 0)
}
