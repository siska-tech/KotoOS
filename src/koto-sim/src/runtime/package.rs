use super::*;

pub fn load_packages(root: impl AsRef<Path>) -> Result<PackageList, SimError> {
    let root = root.as_ref();
    let mut fs = HostFs::mounted(root).map_err(|_| SimError::Io)?;
    let mut paths = manifest_paths(&fs)?;
    paths.sort();

    let mut packages = PackageList::new();
    for path in paths {
        let text = read_to_string(&mut fs, &path)?;
        let mut package = parse_manifest(&text)?;
        if let Some(icon_path) = package.icon_path().map(str::to_string) {
            let icon_bytes = read_bytes(&mut fs, &icon_path)?;
            let icon =
                PackageIcon::from_kicon_text(&icon_bytes).map_err(|_| SimError::InvalidManifest)?;
            package.set_icon(icon);
        }
        package.set_save_data_present(save_data_present(root, package.app_id())?);
        if !packages.push(package) {
            return Err(SimError::PackageListFull);
        }
    }

    Ok(packages)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchReport {
    pub app_id: String,
    pub runtime: String,
    pub entry: String,
    pub result: VmRunResult,
    pub draw_rects: Vec<(i32, i32, i32, i32, i32)>,
    pub text: Vec<(i32, i32, String)>,
}

pub fn load_launch_manifest(
    root: impl AsRef<Path>,
    app_id: &str,
) -> Result<PackageLaunch, SimError> {
    let mut fs = HostFs::mounted(&root).map_err(|_| SimError::Io)?;
    let mut paths = manifest_paths(&fs)?;
    paths.sort();

    for path in paths {
        let text = read_to_string(&mut fs, &path)?;
        let launch = parse_launch_manifest(&text)?;
        if launch.package.app_id() == app_id {
            return Ok(launch);
        }
    }

    Err(SimError::Io)
}

pub fn launch_package(
    root: impl AsRef<Path>,
    package: &PackageInfo,
) -> Result<LaunchReport, SimError> {
    let launch = load_launch_manifest(&root, package.app_id())?;
    if launch.runtime() != KOTORUNTIME_BYTECODE {
        return Err(SimError::InvalidRuntime);
    }

    let mut fs = HostFs::mounted(&root).map_err(|_| SimError::Io)?;
    let bytecode = read_bytes(&mut fs, launch.entry())?;
    let program = verify_kbc(&bytecode, RuntimeLimits::simulator_default())
        .map_err(|_| SimError::RuntimeVerifyFailed)?;
    let mut vm = BytecodeVm::<SIM_VM_STACK_SLOTS, SIM_VM_CALL_DEPTH>::new(&program)
        .map_err(|_| SimError::RuntimeExecutionFailed)?;
    // Const heap image (KOTO-0139): rodata initializes heap[0..rodata_size].
    let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
    if let Some((start, end)) = program.rodata_range() {
        heap[..end - start].copy_from_slice(&bytecode[start..end]);
    }
    let mut host = SimRuntimeHost::with_audio_and_assets(
        HostFs::mounted(&root).map_err(|_| SimError::Io)?,
        launch.package.app_id(),
        Arc::new(Mutex::new(SimAudio::new(DEFAULT_SAMPLE_RATE))),
        launch.asset_paths().to_vec(),
    )
    .map_err(|_| SimError::RuntimeExecutionFailed)?;
    let result = vm
        .execute_frame(
            &bytecode,
            &program,
            &mut host,
            VmInputSnapshot::empty(),
            SIM_FRAME_FUEL,
            &mut heap,
        )
        .map_err(|_| SimError::RuntimeExecutionFailed)?;

    Ok(LaunchReport {
        app_id: launch.package.app_id().to_string(),
        runtime: launch.runtime,
        entry: launch.entry,
        result,
        draw_rects: host.draw_rects,
        text: host.text,
    })
}

pub fn describe_launch_report(report: &LaunchReport) -> String {
    let result = match report.result {
        VmRunResult::Yielded => "yielded".to_string(),
        VmRunResult::Exited(code) => format!("exited({code})"),
        VmRunResult::FuelExhausted => "fuel-exhausted".to_string(),
    };
    format!(
        "runtime {} entry {} -> {} draw_rects={} text={}",
        report.runtime,
        report.entry,
        result,
        report.draw_rects.len(),
        report.text.len()
    )
}

fn manifest_paths(fs: &HostFs) -> Result<Vec<String>, SimError> {
    let entries = fs.read_dir("apps").map_err(|_| SimError::Io)?;
    let mut paths = Vec::new();
    for entry in entries {
        if entry.virtual_path().ends_with(".kpa.json") {
            paths.push(entry.virtual_path().to_string());
        }
    }
    Ok(paths)
}

fn read_to_string(fs: &mut HostFs, path: &str) -> Result<String, SimError> {
    let bytes = read_bytes(fs, path)?;
    String::from_utf8(bytes).map_err(|_| SimError::InvalidManifest)
}

pub(super) fn read_bytes(fs: &mut HostFs, path: &str) -> Result<Vec<u8>, SimError> {
    let mut file = fs.open(path, FileMode::Read).map_err(|_| SimError::Io)?;
    let mut bytes = Vec::new();
    let mut buffer = [0; 256];

    loop {
        let len = file.read(&mut buffer).map_err(|_| SimError::Io)?;
        if len == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..len]);
    }

    Ok(bytes)
}
