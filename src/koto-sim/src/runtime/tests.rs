#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parses_required_manifest_fields() {
        let manifest = r#"{
            "format": "kpa-manifest",
            "version": 1,
            "app_id": "dev.koto.test",
            "name": "Test App",
            "runtime": "kotoruntime-bytecode",
            "entry": "bytecode/main.kbc",
            "icon": "icons/test.kicon"
        }"#;

        let package = parse_manifest(manifest).unwrap();
        assert_eq!(package.app_id(), "dev.koto.test");
        assert_eq!(package.name(), "Test App");
        assert_eq!(package.icon_path(), Some("icons/test.kicon"));
    }

    #[test]
    fn parses_manifest_details_for_shell_view() {
        let manifest = r#"{
            "format": "kpa-manifest",
            "version": 1,
            "app_id": "dev.koto.test",
            "name": "Test App",
            "runtime": "kotoruntime-bytecode",
            "entry": "bytecode/main.kbc",
            "memory": {
                "sram_work_bytes": 24576,
                "psram_cache_bytes": 32768
            },
            "permissions": {
                "fs": "sandbox",
                "network": false
            }
        }"#;

        let package = parse_manifest(manifest).unwrap();

        assert_eq!(package.runtime(), Some("kotoruntime-bytecode"));
        assert_eq!(package.entry(), Some("bytecode/main.kbc"));
        assert_eq!(package.fs_permission(), Some("sandbox"));
        assert_eq!(package.network_permission(), Some(false));
        assert_eq!(package.sram_work_bytes(), Some(24_576));
        assert_eq!(package.psram_cache_bytes(), Some(32_768));
    }

    #[test]
    fn parses_description_and_category_for_shell_pane() {
        let manifest = r#"{
            "format": "kpa-manifest",
            "version": 1,
            "app_id": "dev.koto.test",
            "name": "Test App",
            "runtime": "kotoruntime-bytecode",
            "entry": "bytecode/main.kbc",
            "description": "A test description.",
            "category": "Tools"
        }"#;

        let package = parse_manifest(manifest).unwrap();

        assert_eq!(package.description(), Some("A test description."));
        assert_eq!(package.category(), Some("Tools"));
    }

    #[test]
    fn parses_manifest_without_description_or_category() {
        let manifest = r#"{
            "format": "kpa-manifest",
            "version": 1,
            "app_id": "dev.koto.test",
            "name": "Test App",
            "runtime": "kotoruntime-bytecode",
            "entry": "bytecode/main.kbc"
        }"#;

        let package = parse_manifest(manifest).unwrap();

        assert_eq!(package.description(), None);
        assert_eq!(package.category(), None);
    }

    #[test]
    fn load_packages_marks_save_data_presence() {
        let root = test_root("load_packages_marks_save_data_presence");
        write_manifest_only(&root, KOTORUNTIME_BYTECODE);
        fs::create_dir_all(root.join("data").join("dev.koto.test")).unwrap();
        fs::write(
            root.join("data").join("dev.koto.test").join("settings.txt"),
            b"saved",
        )
        .unwrap();

        let packages = load_packages(&root).unwrap();
        let package = packages.iter().next().unwrap();

        assert!(package.save_data_present());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_wrong_manifest_format() {
        let manifest = r#"{
            "format": "something-else",
            "version": 1,
            "app_id": "dev.koto.test",
            "name": "Test App",
            "runtime": "kotoruntime-bytecode",
            "entry": "bytecode/main.kbc"
        }"#;

        assert_eq!(parse_manifest(manifest), Err(SimError::InvalidManifest));
    }

    #[test]
    fn rejects_missing_manifest_runtime_or_entry() {
        let manifest = r#"{
            "format": "kpa-manifest",
            "version": 1,
            "app_id": "dev.koto.test",
            "name": "Test App"
        }"#;

        assert_eq!(parse_manifest(manifest), Err(SimError::InvalidManifest));
    }

    #[test]
    fn rejects_invalid_manifest_runtime_and_entry_values() {
        let bad_runtime = r#"{
            "format": "kpa-manifest",
            "version": 1,
            "app_id": "dev.koto.test",
            "name": "Test App",
            "runtime": "KotoRuntime",
            "entry": "bytecode/main.kbc"
        }"#;
        let bad_entry = r#"{
            "format": "kpa-manifest",
            "version": 1,
            "app_id": "dev.koto.test",
            "name": "Test App",
            "runtime": "kotoruntime-bytecode",
            "entry": "../main.kbc"
        }"#;

        assert_eq!(parse_manifest(bad_runtime), Err(SimError::InvalidManifest));
        assert_eq!(parse_manifest(bad_entry), Err(SimError::InvalidManifest));
    }

    #[test]
    fn rejects_invalid_manifest_icon_value() {
        let manifest = r#"{
            "format": "kpa-manifest",
            "version": 1,
            "app_id": "dev.koto.test",
            "name": "Test App",
            "runtime": "kotoruntime-bytecode",
            "entry": "bytecode/main.kbc",
            "icon": "../icon.kicon"
        }"#;

        assert_eq!(parse_manifest(manifest), Err(SimError::InvalidManifest));
    }

    #[test]
    fn recorder_captures_shell_redraw_commands() {
        let mut packages = PackageList::new();
        packages.push(PackageInfo::new("dev.koto.one", "One").unwrap());
        packages.push(PackageInfo::new("dev.koto.two", "Two").unwrap());
        let mut shell = ShellState::new(packages);
        let mut recorder = RenderRecorder::new();

        recorder.record_shell_full(&shell).unwrap();
        let previous = shell.selected_index();
        shell.update(&koto_core::InputState {
            pressed: koto_core::Buttons {
                down: true,
                ..koto_core::Buttons::default()
            },
            ..koto_core::InputState::default()
        });
        recorder
            .record_shell_selection_change(&shell, previous)
            .unwrap();

        // Full repaint, then the selection change repaints the previous and
        // current tiles plus the details pane and status strip.
        assert_eq!(recorder.commands().len(), 5);
        assert_eq!(
            describe_render_command(&recorder.commands()[0]),
            "render full 320x320 Rgb565"
        );
    }

    #[test]
    fn recorder_captures_per_row_list_commands() {
        let mut packages = PackageList::new();
        packages.push(PackageInfo::new("dev.koto.one", "One").unwrap());
        packages.push(PackageInfo::new("dev.koto.two", "Two").unwrap());
        let shell = ShellState::new(packages);
        let mut recorder = RenderRecorder::new();

        recorder.record_shell_list(&shell).unwrap();

        // Header, one command per package tile, the page indicator, then the
        // details pane, status strip, and command bar (pane shown by default).
        let log: Vec<String> = recorder
            .commands()
            .iter()
            .map(describe_render_command)
            .collect();
        assert_eq!(
            log,
            [
                "render rect x=0 y=0 w=320 h=20 on 320x320 Rgb565",
                "render rect x=0 y=20 w=65 h=84 on 320x320 Rgb565",
                "render rect x=65 y=20 w=65 h=84 on 320x320 Rgb565",
                "render rect x=0 y=272 w=196 h=16 on 320x320 Rgb565",
                "render rect x=196 y=20 w=124 h=268 on 320x320 Rgb565",
                "render rect x=0 y=288 w=320 h=14 on 320x320 Rgb565",
                "render rect x=0 y=302 w=320 h=18 on 320x320 Rgb565",
            ]
        );
    }

    #[test]
    fn parses_direction_confirm_and_cancel_script_tokens() {
        assert_eq!(
            parse_input_script("down up left right\nconfirm cancel # ignored").unwrap(),
            [
                HostInput::Down,
                HostInput::Up,
                HostInput::Left,
                HostInput::Right,
                HostInput::Confirm,
                HostInput::Cancel
            ]
        );
    }

    #[test]
    fn rejects_unknown_script_tokens() {
        assert_eq!(
            parse_input_script("down nope"),
            Err(SimError::InvalidInputScript)
        );
    }

    #[test]
    fn scripted_input_navigates_multiple_package_entries() {
        let mut packages = PackageList::new();
        packages.push(PackageInfo::new("dev.koto.one", "One").unwrap());
        packages.push(PackageInfo::new("dev.koto.two", "Two").unwrap());
        packages.push(PackageInfo::new("dev.koto.three", "Three").unwrap());
        let mut shell = ShellState::new(packages);
        let inputs = parse_input_script("right right left confirm").unwrap();

        let events = run_shell_script(&mut shell, &inputs);

        assert_eq!(events.len(), 4);
        assert_eq!(events[0].selected_index, 1);
        assert_eq!(events[1].selected_index, 2);
        assert_eq!(events[2].selected_index, 1);
        match events[3].action {
            ShellAction::Launch(package) => assert_eq!(package.name(), "Two"),
            ShellAction::None | ShellAction::OpenConfig => panic!("expected launch action"),
        }
    }

    #[test]
    fn host_fs_reads_files_under_mounted_root() {
        let root = test_root("host_fs_reads_files_under_mounted_root");
        fs::create_dir_all(root.join("apps")).unwrap();
        fs::write(root.join("apps").join("memo.txt"), b"memo").unwrap();

        let mut host_fs = HostFs::mounted(&root).unwrap();
        let mut file = host_fs.open("apps/memo.txt", FileMode::Read).unwrap();
        let mut bytes = [0; 4];

        assert_eq!(file.read(&mut bytes).unwrap(), 4);
        assert_eq!(&bytes, b"memo");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn asset_load_reads_declared_package_asset() {
        let root = test_root("asset_load_reads_declared_package_asset");
        fs::create_dir_all(root.join("sprites")).unwrap();
        fs::write(root.join("sprites").join("tiles.kim"), b"KIM1DATA").unwrap();

        let host_fs = HostFs::mounted(&root).unwrap();
        let audio = Arc::new(Mutex::new(SimAudio::new(DEFAULT_SAMPLE_RATE)));
        let mut host = SimRuntimeHost::with_audio_and_assets(
            host_fs,
            "dev.koto.test",
            audio,
            vec!["sprites/tiles.kim".to_string()],
        )
        .unwrap();

        // A declared asset is copied into the destination slice.
        let mut dst = [0u8; 8];
        assert!(matches!(
            host.asset_load("sprites/tiles.kim", &mut dst),
            HostCallOutcome::Ok1(8)
        ));
        assert_eq!(&dst, b"KIM1DATA");

        // An undeclared path is refused even if it exists on disk.
        let mut other = [0u8; 8];
        assert!(matches!(
            host.asset_load("sprites/other.kim", &mut other),
            HostCallOutcome::Err(_)
        ));

        fs::remove_dir_all(root).unwrap();
    }

    /// KOTO-0236 path identity: the same string literal names the same asset
    /// for compile-time `asset_len` (resolved through the on-disk `app.json`
    /// above the root source) and runtime `asset_load`, and the loaded byte
    /// count equals the folded size for the sized asset.
    #[test]
    fn asset_len_folds_the_size_that_asset_load_reads_at_runtime() {
        let root = test_root("asset_len_folds_the_size_that_asset_load_reads_at_runtime");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("locales")).unwrap();
        let payload = b"Title\nBody line\n";
        fs::write(root.join("locales").join("en-US.txt"), payload).unwrap();
        fs::write(
            root.join("app.json"),
            br#"{"assets":[{"source":"locales/en-US.txt","output":"locales/en-US.txt"}]}"#,
        )
        .unwrap();
        let source = "
const RAW_BYTES = asset_len(\"locales/en-US.txt\");
fn main() {
    buf raw[RAW_BYTES];
    let loaded = asset_load(\"locales/en-US.txt\", 17, raw, len(raw));
    exit(loaded - RAW_BYTES);
}
";
        let source_path = root.join("src").join("main.koto");
        fs::write(&source_path, source).unwrap();

        let bytecode = koto_compiler::compile(source_path.to_str().unwrap(), source).unwrap();
        let program =
            koto_core::verify_kbc(&bytecode, koto_core::RuntimeLimits::simulator_default())
                .unwrap();
        assert!(
            program.header().max_heap_bytes >= payload.len() as u32,
            "folded buffer must reserve the packaged bytes"
        );
        let mut vm = koto_core::BytecodeVm::<16, 4>::new(&program).unwrap();
        let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
        if let Some((start, end)) = program.rodata_range() {
            heap[..end - start].copy_from_slice(&bytecode[start..end]);
        }
        let audio = Arc::new(Mutex::new(SimAudio::new(DEFAULT_SAMPLE_RATE)));
        let mut host = SimRuntimeHost::with_audio_and_assets(
            HostFs::mounted(&root).unwrap(),
            "dev.koto.test",
            audio,
            vec!["locales/en-US.txt".to_string()],
        )
        .unwrap();

        // Exit code is `loaded - folded`: zero exactly when the runtime read
        // the byte count the compiler folded.
        assert_eq!(
            vm.execute_frame(
                &bytecode,
                &program,
                &mut host,
                VmInputSnapshot::empty(),
                SIM_FRAME_FUEL,
                &mut heap,
            )
            .unwrap(),
            koto_core::VmRunResult::Exited(0)
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn host_fs_rejects_paths_that_escape_mounted_root() {
        let root = test_root("host_fs_rejects_paths_that_escape_mounted_root");
        let outside = root.parent().unwrap().join("outside-secret.txt");
        fs::create_dir_all(root.join("apps")).unwrap();
        fs::write(&outside, b"secret").unwrap();

        let mut host_fs = HostFs::mounted(&root).unwrap();

        assert_eq!(
            host_fs.open("../outside-secret.txt", FileMode::Read).err(),
            Some(HalError::InvalidArgument)
        );
        assert_eq!(
            host_fs
                .open("apps/../../outside-secret.txt", FileMode::Read)
                .err(),
            Some(HalError::InvalidArgument)
        );

        fs::remove_file(outside).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn host_fs_rejects_invalid_dir_entry_names_before_exposing_paths() {
        let root = test_root("host_fs_rejects_invalid_dir_entry_names_before_exposing_paths");
        fs::create_dir_all(root.join("apps")).unwrap();
        fs::write(root.join("apps").join("memo.kpa.json"), b"{}").unwrap();

        let host_fs = HostFs::mounted(&root).unwrap();
        let entries = host_fs.read_dir("apps").unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].virtual_path(), "apps/memo.kpa.json");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn save_data_lists_namespaces_without_host_paths() {
        let root = test_root("save_data_lists_namespaces_without_host_paths");
        fs::create_dir_all(root.join("data").join("dev.koto.b").join("nested")).unwrap();
        fs::create_dir_all(root.join("data").join("dev.koto.a")).unwrap();
        fs::write(root.join("data").join("dev.koto.b").join("one.txt"), b"one").unwrap();
        fs::write(
            root.join("data")
                .join("dev.koto.b")
                .join("nested")
                .join("two.txt"),
            b"twenty",
        )
        .unwrap();

        let namespaces = list_save_data(&root).unwrap();

        assert_eq!(
            namespaces,
            [
                SaveDataNamespace {
                    app_id: "dev.koto.a".to_string(),
                    file_count: 0,
                    total_bytes: 0,
                },
                SaveDataNamespace {
                    app_id: "dev.koto.b".to_string(),
                    file_count: 2,
                    total_bytes: 9,
                },
            ]
        );
        let described = describe_save_data_namespace(&namespaces[1]);
        assert_eq!(described, "save-data dev.koto.b files=2 bytes=9");
        assert!(!described.contains(&root.display().to_string()));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn save_data_clear_removes_selected_namespace_only() {
        let root = test_root("save_data_clear_removes_selected_namespace_only");
        fs::create_dir_all(root.join("data").join("dev.koto.memo")).unwrap();
        fs::create_dir_all(root.join("data").join("dev.koto.other")).unwrap();
        fs::write(
            root.join("data").join("dev.koto.memo").join("memo.txt"),
            b"memo",
        )
        .unwrap();
        fs::write(
            root.join("data").join("dev.koto.other").join("note.txt"),
            b"note",
        )
        .unwrap();

        let report = clear_save_data(&root, "dev.koto.memo").unwrap();

        assert_eq!(
            describe_save_data_clear_report(&report),
            "cleared save-data dev.koto.memo"
        );
        assert!(!root.join("data").join("dev.koto.memo").exists());
        assert!(root
            .join("data")
            .join("dev.koto.other")
            .join("note.txt")
            .exists());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn save_data_clear_rejects_escape_like_app_ids() {
        let root = test_root("save_data_clear_rejects_escape_like_app_ids");
        fs::create_dir_all(root.join("data").join("dev.koto.memo")).unwrap();

        assert_eq!(
            clear_save_data(&root, "../dev.koto.memo").err(),
            Some(SimError::InvalidManifest)
        );
        assert_eq!(
            clear_save_data(&root, "dev/koto/memo").err(),
            Some(SimError::InvalidManifest)
        );
        assert!(root.join("data").join("dev.koto.memo").exists());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn writes_bmp_with_expected_header_and_size() {
        let mut framebuffer = Framebuffer::new(4, 2);
        framebuffer.as_canvas().clear(koto_core::Rgb565(0xF800)); // red

        let root = test_root("writes_bmp");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("out.bmp");
        write_bmp(&path, &framebuffer).unwrap();

        let bytes = fs::read(&path).unwrap();
        assert_eq!(&bytes[0..2], b"BM");
        // 4px row * 3 bytes = 12, already 4-aligned; 2 rows => 24 bytes data + 54 header.
        assert_eq!(bytes.len(), 54 + 24);
        assert_eq!(
            u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]),
            54
        );
        // First pixel byte triple is BGR for pure red.
        assert_eq!(&bytes[54..57], &[0x00, 0x00, 0xFF]);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_launch_manifest_runtime_and_entry() {
        let manifest = r#"{
            "format": "kpa-manifest",
            "version": 1,
            "app_id": "dev.koto.test",
            "name": "Test App",
            "runtime": "kotoruntime-bytecode",
            "entry": "bytecode/main.kbc"
        }"#;

        let launch = parse_launch_manifest(manifest).unwrap();

        assert_eq!(launch.package.app_id(), "dev.koto.test");
        assert_eq!(launch.runtime(), KOTORUNTIME_BYTECODE);
        assert_eq!(launch.entry(), "bytecode/main.kbc");
    }

    #[test]
    fn launches_minimal_bytecode_package() {
        let root = test_root("launches_minimal_bytecode_package");
        write_runtime_package(&root, "kotoruntime-bytecode", &minimal_exit_kbc(3));

        let package = load_packages(&root)
            .unwrap()
            .iter()
            .next()
            .copied()
            .unwrap();

        let report = launch_package(&root, &package).unwrap();

        assert_eq!(report.app_id, "dev.koto.test");
        assert_eq!(report.runtime, KOTORUNTIME_BYTECODE);
        assert_eq!(report.entry, "bytecode/main.kbc");
        assert_eq!(report.result, koto_core::VmRunResult::Exited(3));
        assert_eq!(
            describe_launch_report(&report),
            "runtime kotoruntime-bytecode entry bytecode/main.kbc -> exited(3) draw_rects=0 text=0"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn launches_memo_smoke_bytecode_package() {
        let root = test_root("launches_memo_smoke_bytecode_package");
        write_runtime_package(&root, "kotoruntime-bytecode", &memo_smoke_kbc());

        let package = load_packages(&root)
            .unwrap()
            .iter()
            .next()
            .copied()
            .unwrap();
        let report = launch_package(&root, &package).unwrap();

        assert_eq!(report.result, koto_core::VmRunResult::Exited(0));
        assert_eq!(report.text, [(0, 0, String::from("koto memo\n"))]);
        assert_eq!(
            fs::read(root.join("data").join("dev.koto.test").join("memo.txt")).unwrap(),
            b"koto memo\n"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn launch_rejects_app_exceeding_manifest_memory_budget() {
        // Per-app heap profile (KOTO-0096): the manifest's `sram_work_bytes` is the
        // device budget, and an app whose KBC header requests more heap than that is
        // refused at launch.
        let root = test_root("launch_rejects_over_budget_app");
        fs::create_dir_all(root.join("apps")).unwrap();
        fs::create_dir_all(root.join("bytecode")).unwrap();
        let code = [
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 0),
            insn(
                koto_core::runtime::opcode::HOST_CALL,
                koto_core::runtime::host_call::EXIT,
                0,
            ),
        ];
        // Header requests 1024 bytes (within the 16 KB ceiling, so it verifies) but
        // the manifest only budgets 256.
        fs::write(
            root.join("bytecode").join("main.kbc"),
            kbc_with_heap(&code, 1024),
        )
        .unwrap();
        fs::write(
            root.join("apps").join("test.kpa.json"),
            r#"{
                "format": "kpa-manifest",
                "version": 1,
                "app_id": "dev.koto.test",
                "name": "Test App",
                "runtime": "kotoruntime-bytecode",
                "entry": "bytecode/main.kbc",
                "memory": { "sram_work_bytes": 256 }
            }"#,
        )
        .unwrap();

        assert_eq!(
            BytecodeAppSession::launch(&root, "dev.koto.test").err(),
            Some(SimError::AppExceedsMemoryBudget)
        );

        fs::remove_dir_all(root).unwrap();
    }

    /// The committed memo bytecode, compiled from `apps/memo/src/main.koto` and
    /// kept in sync by the app build loop. The validation drives this real app.
    const REAL_MEMO_KBC: &[u8] = include_bytes!("../../../../package_inputs/bytecode/memo.kbc");

    #[test]
    fn memo_validation_drives_bytecode_app_end_to_end() {
        let root = test_root("memo_validation_bytecode_app");
        write_memo_runtime_package(&root, REAL_MEMO_KBC);
        write_skk_dict(&root);

        let report = run_memo_validation(&root).unwrap();

        assert!(report.shell_launched);
        assert_eq!(
            report.ime_before_commit.mode,
            koto_core::MemoImeMode::Candidate
        );
        assert_eq!(report.ime_before_commit.reading, "かさ");
        assert_eq!(report.ime_before_commit.candidate.as_deref(), Some("傘"));
        // Romaji か, then SKK-committed 傘.
        assert_eq!(report.document_after_commit, "か傘");
        // Move left over 傘 and backspace removes か, leaving 傘; that is saved.
        assert_eq!(report.saved_document, "傘");
        assert_eq!(report.reloaded_document, "傘");
        assert_eq!(report.saved_path, "data/dev.koto.memo/memo.txt");
        assert!(!report.sandbox_escape_found);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn memo_dialog_opens_second_sandbox_file_and_saves_within_sandbox() {
        let root = test_root("memo_dialog_open_second_file");
        write_memo_runtime_package(&root, REAL_MEMO_KBC);
        write_skk_dict(&root);

        // Seed two files in the app sandbox. `memo.txt` sorts before `note.txt`,
        // so `dir_list` returns [memo.txt, note.txt] and the dialog highlights
        // memo.txt first.
        let data_dir = root.join("data").join(MEMO_APP_ID);
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("memo.txt"), "first").unwrap();
        fs::write(data_dir.join("note.txt"), "second").unwrap();

        use koto_core::runtime::text_intent as ti;
        let intent = |bits: u32| VmInputSnapshot {
            intent_bits: bits,
            ..VmInputSnapshot::empty()
        };

        let mut session = BytecodeAppSession::launch(&root, MEMO_APP_ID).unwrap();
        // Frame 1 loads the default file (memo.txt).
        session.step_frame(VmInputSnapshot::empty()).unwrap();
        assert_eq!(session.document(), "first");

        // Open the picker, move to the second entry, and confirm it.
        session.step_frame(intent(ti::OPEN)).unwrap();
        session.step_frame(intent(ti::DOWN)).unwrap();
        session.step_frame(intent(ti::COMMIT)).unwrap();
        assert_eq!(session.document(), "second");

        // Type an ASCII edit (IME starts off), then save + exit.
        session
            .step_frame(VmInputSnapshot {
                text_codepoint: 'x' as u32,
                ..VmInputSnapshot::empty()
            })
            .unwrap();
        session.step_frame(intent(ti::EXIT)).unwrap();
        assert!(session.has_exited());

        // The edit lands in note.txt, inside the sandbox; memo.txt is untouched.
        let saved_note = fs::read_to_string(data_dir.join("note.txt")).unwrap();
        assert!(
            saved_note.contains('x'),
            "edit saved to note.txt: {saved_note:?}"
        );
        assert!(
            saved_note.contains("second"),
            "note.txt kept its text: {saved_note:?}"
        );
        assert_eq!(
            fs::read_to_string(data_dir.join("memo.txt")).unwrap(),
            "first"
        );
        assert!(!root.join("note.txt").exists());
        assert!(!root.join("data").join("note.txt").exists());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn memo_save_prompt_overwrites_current_file_on_yes() {
        let root = test_root("memo_save_prompt_overwrite");
        write_memo_runtime_package(&root, REAL_MEMO_KBC);
        write_skk_dict(&root);
        let data_dir = root.join("data").join(MEMO_APP_ID);
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("memo.txt"), "first").unwrap();

        use koto_core::runtime::text_intent as ti;
        let intent = |bits: u32| VmInputSnapshot {
            intent_bits: bits,
            ..VmInputSnapshot::empty()
        };
        let typed = |ch: char| VmInputSnapshot {
            text_codepoint: ch as u32,
            ..VmInputSnapshot::empty()
        };

        let mut session = BytecodeAppSession::launch(&root, MEMO_APP_ID).unwrap();
        session.step_frame(VmInputSnapshot::empty()).unwrap();
        assert_eq!(session.document(), "first");

        // Edit, then F2 -> y confirms overwrite of memo.txt.
        session.step_frame(typed('x')).unwrap();
        session.step_frame(intent(ti::SAVE)).unwrap();
        assert!(session
            .text()
            .contains(&(10, 286, String::from("上書き保存しますか? (y/n)"))));
        session.step_frame(typed('y')).unwrap();

        let saved = fs::read_to_string(data_dir.join("memo.txt")).unwrap();
        assert!(saved.contains('x'), "overwrite kept the edit: {saved:?}");
        assert!(
            saved.contains("first"),
            "overwrite kept the text: {saved:?}"
        );
        // No stray new file, nothing outside the sandbox.
        assert_eq!(
            fs::read_dir(&data_dir).unwrap().count(),
            1,
            "overwrite did not create extra files"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn memo_save_as_writes_new_sandbox_file_and_switches_active() {
        let root = test_root("memo_save_as_new_file");
        write_memo_runtime_package(&root, REAL_MEMO_KBC);
        write_skk_dict(&root);
        let data_dir = root.join("data").join(MEMO_APP_ID);
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("memo.txt"), "first").unwrap();

        use koto_core::runtime::text_intent as ti;
        let intent = |bits: u32| VmInputSnapshot {
            intent_bits: bits,
            ..VmInputSnapshot::empty()
        };
        let typed = |ch: char| VmInputSnapshot {
            text_codepoint: ch as u32,
            ..VmInputSnapshot::empty()
        };

        let mut session = BytecodeAppSession::launch(&root, MEMO_APP_ID).unwrap();
        session.step_frame(VmInputSnapshot::empty()).unwrap();

        // Edit, then F2 -> n -> type a new name -> confirm saves a copy.
        session.step_frame(typed('x')).unwrap();
        session.step_frame(intent(ti::SAVE)).unwrap();
        session.step_frame(typed('n')).unwrap();
        for ch in "note2.txt".chars() {
            session.step_frame(typed(ch)).unwrap();
        }
        session.step_frame(intent(ti::COMMIT)).unwrap();

        // The new file holds the edited document; the original is untouched.
        let saved = fs::read_to_string(data_dir.join("note2.txt")).unwrap();
        assert!(saved.contains('x'), "save-as wrote the edit: {saved:?}");
        assert!(
            saved.contains("first"),
            "save-as wrote the document: {saved:?}"
        );
        assert_eq!(
            fs::read_to_string(data_dir.join("memo.txt")).unwrap(),
            "first"
        );
        assert!(!root.join("note2.txt").exists());

        // note2.txt is now active: a further save + y writes back to it.
        session.step_frame(typed('y')).unwrap();
        session.step_frame(intent(ti::SAVE)).unwrap();
        session.step_frame(typed('y')).unwrap();
        assert!(fs::read_to_string(data_dir.join("note2.txt"))
            .unwrap()
            .contains('y'));
        assert_eq!(
            fs::read_to_string(data_dir.join("memo.txt")).unwrap(),
            "first"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bytecode_session_steps_smoke_app_and_surfaces_draw_output() {
        let root = test_root("bytecode_session_steps_smoke_app");
        write_memo_runtime_package(&root, &memo_smoke_kbc());

        let mut session = BytecodeAppSession::launch(&root, MEMO_APP_ID).unwrap();
        let result = session
            .step_frame(koto_core::VmInputSnapshot::empty())
            .unwrap();

        assert_eq!(result, koto_core::VmRunResult::Exited(0));
        assert!(session.has_exited());
        assert_eq!(session.text(), [(0, 0, String::from("koto memo\n"))]);
        assert_eq!(
            fs::read(root.join("data").join(MEMO_APP_ID).join("memo.txt")).unwrap(),
            b"koto memo\n"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bytecode_session_drives_ime_and_editor_through_the_vm() {
        let root = test_root("bytecode_session_drives_ime");
        write_memo_runtime_package(&root, &feed_keys_kbc(&[('k', false), ('a', false)], false));

        let mut session = BytecodeAppSession::launch(&root, MEMO_APP_ID).unwrap();
        let result = session
            .step_frame(koto_core::VmInputSnapshot::empty())
            .unwrap();

        assert_eq!(result, koto_core::VmRunResult::Exited(0));
        // Plain romaji commits kana straight into the editor document via the VM.
        assert_eq!(session.document(), "か");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bytecode_session_runs_sticky_shift_skk_conversion_through_the_vm() {
        let root = test_root("bytecode_session_skk_conversion");
        write_memo_runtime_package(
            &root,
            &feed_keys_kbc(
                &[
                    ('\0', true),
                    ('k', false),
                    ('a', false),
                    ('s', false),
                    ('a', false),
                ],
                true,
            ),
        );
        write_skk_dict(&root);

        let mut session = BytecodeAppSession::launch(&root, MEMO_APP_ID).unwrap();
        let result = session
            .step_frame(koto_core::VmInputSnapshot::empty())
            .unwrap();

        assert_eq!(result, koto_core::VmRunResult::Exited(0));
        let line = session.ime_line();
        assert_eq!(line.mode, koto_core::MemoImeMode::Candidate);
        assert_eq!(line.reading, "かさ");
        assert_eq!(line.candidate, Some("傘"));
        // Conversion stays in the IME line until committed; the document is untouched.
        assert_eq!(session.document(), "");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_app_script_chars_intents_and_frames() {
        let inputs = parse_app_script(
            "'k'            # a character\nshift convert\nframe          # empty frame\ncommit exit ime-toggle activate confirm",
        )
        .unwrap();
        assert_eq!(inputs.len(), 4);
        assert_eq!(inputs[0].text_codepoint, u32::from('k'));
        assert_eq!(
            inputs[1].intent_bits,
            koto_core::runtime::text_intent::SHIFT | koto_core::runtime::text_intent::CONVERT
        );
        assert_eq!(inputs[2], koto_core::VmInputSnapshot::empty());
        assert_eq!(
            inputs[3].intent_bits,
            koto_core::runtime::text_intent::COMMIT
                | koto_core::runtime::text_intent::EXIT
                | koto_core::runtime::text_intent::IME_TOGGLE
        );
        assert_eq!(inputs[3].pressed_bits, 1 << 4);
    }

    #[test]
    fn app_script_accepts_ime_toggle_alias() {
        let inputs = parse_app_script("ime\nime-toggle").unwrap();
        assert_eq!(inputs.len(), 2);
        assert_eq!(
            inputs[0].intent_bits,
            koto_core::runtime::text_intent::IME_TOGGLE
        );
        assert_eq!(
            inputs[1].intent_bits,
            koto_core::runtime::text_intent::IME_TOGGLE
        );
    }

    #[test]
    fn memo_app_script_handles_mixed_ascii_japanese_and_invalid_romaji() {
        let root = test_root("memo_mixed_ascii_japanese_invalid_romaji");
        write_memo_runtime_package(&root, REAL_MEMO_KBC);
        write_skk_dict(&root);

        let inputs = parse_app_script(
            "
            'a'
            'b'
            ime-toggle
            'k'
            'a'
            'k'
            'x'
            ime-toggle
            'c'
            exit
            ",
        )
        .unwrap();
        let report = run_app_scenario(&root, MEMO_APP_ID, &inputs).unwrap();

        assert_eq!(report.result, koto_core::VmRunResult::Exited(0));
        assert_eq!(report.document, "abかkxc");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn memo_app_renders_visible_lines_caret_status_and_ime_display() {
        let root = test_root("memo_visible_editor_ui");
        write_memo_runtime_package(&root, REAL_MEMO_KBC);
        write_skk_dict(&root);
        let inputs = parse_app_script(
            "
            'a'
            newline
            'b'
            up
            ime-toggle
            'k'
            ",
        )
        .unwrap();

        let mut session = BytecodeAppSession::launch(&root, MEMO_APP_ID).unwrap();
        for input in inputs {
            assert_eq!(session.step_frame(input).unwrap(), VmRunResult::Yielded);
        }

        assert_eq!(session.document(), "a\nb");
        // Title bar: app name, filename, and the unsaved badge (typing dirtied it).
        assert!(session.text().contains(&(28, 6, String::from("メモ帳"))));
        assert!(session.text().contains(&(74, 9, String::from("memo.txt"))));
        assert!(session.text().contains(&(264, 6, String::from("未保存"))));
        // Document rows in the white page.
        assert!(session.text().contains(&(8, 26, String::from("a"))));
        assert!(session.text().contains(&(8, 39, String::from("b"))));
        // Plain romaji composing shows an inline preedit at the caret (col 1,
        // row 0 after the `up`), not a conversion panel.
        assert!(session.text().contains(&(14, 26, String::from("k"))));
        // Command bar shows the normal editing actions (including F4 open) and
        // the cursor status.
        assert!(session.text().contains(&(
            10,
            286,
            String::from("F1入力 F2保存 F3折返 F4開く F5新規")
        )));
        assert!(session
            .text()
            .iter()
            .any(|(_, _, text)| text.starts_with("Ln ")));
        // Caret, title bar, white document area, and command bar.
        assert!(session
            .draw_rects()
            .iter()
            .any(|&(_, y, w, h, _)| (26..274).contains(&y) && w == 2 && h == 13));
        assert!(session.draw_rects().contains(&(0, 0, 320, 24, 6506)));
        assert!(session
            .draw_rects()
            .iter()
            .any(|&(x, y, w, h, _)| (x, y, w, h) == (2, 24, 316, 250)));
        assert!(session.draw_rects().contains(&(0, 274, 320, 46, 6506)));
        // The old conversion panel must not appear.
        assert!(!session
            .draw_rects()
            .iter()
            .any(|&(x, y, w, h, _)| (x, y, w, h) == (2, 210, 316, 64)));

        let cancel = VmInputSnapshot {
            intent_bits: koto_core::runtime::text_intent::CANCEL,
            ..VmInputSnapshot::empty()
        };
        assert_eq!(session.step_frame(cancel).unwrap(), VmRunResult::Yielded);
        // Cancel clears the inline preedit.
        assert!(!session.text().iter().any(|(_, _, text)| text == "k"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn memo_app_draws_scrollbar_for_long_documents() {
        let root = test_root("memo_scrollbar");
        write_memo_runtime_package(&root, REAL_MEMO_KBC);
        write_skk_dict(&root);
        let mut session = BytecodeAppSession::launch(&root, MEMO_APP_ID).unwrap();

        for _ in 0..24 {
            let input = VmInputSnapshot {
                intent_bits: koto_core::runtime::text_intent::NEWLINE,
                ..VmInputSnapshot::empty()
            };
            assert_eq!(session.step_frame(input).unwrap(), VmRunResult::Yielded);
        }
        // Scrollbar track (C_TRACK sign-extends from RGB565 52891).
        assert!(session
            .draw_rects()
            .iter()
            .any(|&(x, y, w, h, _)| (x, y, w, h) == (306, 26, 10, 244)));
        let first_thumb_y = session
            .draw_rects()
            .iter()
            .find_map(|&(x, y, w, h, _)| ((x, w, h) == (306, 10, 26)).then_some(y))
            .expect("scrollbar thumb");

        for _ in 0..24 {
            let input = VmInputSnapshot {
                intent_bits: koto_core::runtime::text_intent::UP,
                ..VmInputSnapshot::empty()
            };
            assert_eq!(session.step_frame(input).unwrap(), VmRunResult::Yielded);
        }
        let second_thumb_y = session
            .draw_rects()
            .iter()
            .find_map(|&(x, y, w, h, _)| ((x, w, h) == (306, 10, 26)).then_some(y))
            .expect("scrollbar thumb after scrolling");

        assert!(second_thumb_y < first_thumb_y);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn memo_app_keeps_caret_in_painted_region_when_scrolled() {
        let root = test_root("memo_caret_scroll");
        write_memo_runtime_package(&root, REAL_MEMO_KBC);
        write_skk_dict(&root);
        let mut session = BytecodeAppSession::launch(&root, MEMO_APP_ID).unwrap();

        // Push the cursor well past one screen of rows.
        for _ in 0..30 {
            let input = VmInputSnapshot {
                intent_bits: koto_core::runtime::text_intent::NEWLINE,
                ..VmInputSnapshot::empty()
            };
            assert_eq!(session.step_frame(input).unwrap(), VmRunResult::Yielded);
        }

        // The caret (2px wide, one row tall) must land inside the painted document
        // region (y 26..274), not behind the command bar — i.e. the editor viewport
        // matches the 19 rows the app draws.
        let caret = session
            .draw_rects()
            .iter()
            .find(|&&(_, _, w, h, _)| w == 2 && h == 13)
            .copied()
            .expect("caret rect");
        assert!(
            (26..274).contains(&caret.1),
            "caret y {} outside painted region",
            caret.1
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn memo_app_commit_intent_confirms_converted_candidate() {
        let root = test_root("memo_commit_candidate");
        write_memo_runtime_package(&root, REAL_MEMO_KBC);
        write_skk_dict(&root);
        let inputs = parse_app_script(
            "
            ime-toggle
            shift
            'k'
            'a'
            's'
            'a'
            convert
            commit
            exit
            ",
        )
        .unwrap();

        let report = run_app_scenario(&root, MEMO_APP_ID, &inputs).unwrap();

        assert_eq!(report.result, VmRunResult::Exited(0));
        assert_eq!(report.document, "傘");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn memo_app_cycles_candidates_and_shows_position() {
        use koto_core::runtime::text_intent as ti;
        let root = test_root("memo_cycle_candidates");
        write_memo_runtime_package(&root, REAL_MEMO_KBC);
        write_skk_dict(&root);
        let mut session = BytecodeAppSession::launch(&root, MEMO_APP_ID).unwrap();
        let intent = |bits| VmInputSnapshot {
            intent_bits: bits,
            ..VmInputSnapshot::empty()
        };
        let chr = |c: char| VmInputSnapshot {
            text_codepoint: c as u32,
            ..VmInputSnapshot::empty()
        };

        session.step_frame(intent(ti::IME_TOGGLE)).unwrap();
        session.step_frame(intent(ti::SHIFT)).unwrap();
        for c in ['k', 'a', 's', 'a'] {
            session.step_frame(chr(c)).unwrap();
        }

        // かさ -> /傘/笠/ : first Tab shows the first candidate inline and `1/2`
        // compactly in the status bar.
        session.step_frame(intent(ti::CONVERT)).unwrap();
        assert_eq!(session.ime_line().candidate, Some("傘"));
        assert!(session.text().contains(&(8, 26, String::from("傘"))));
        assert!(session.text().contains(&(72, 303, String::from("候補"))));
        assert!(session.text().contains(&(102, 303, String::from("1/2"))));
        assert!(!session
            .draw_rects()
            .iter()
            .any(|&(x, y, w, h, _)| (x, y, w, h) == (2, 210, 316, 64)));

        // A second Tab cycles to the next candidate and updates the position.
        session.step_frame(intent(ti::CONVERT)).unwrap();
        assert_eq!(session.ime_line().candidate, Some("笠"));
        assert!(session.text().contains(&(8, 26, String::from("笠"))));
        assert!(session.text().contains(&(102, 303, String::from("2/2"))));

        // Backspace edits the reading: drop the candidate + last kana (かさ -> か),
        // staying in conversion without touching the document.
        session.step_frame(intent(ti::BACKSPACE)).unwrap();
        assert_eq!(session.ime_line().mode, MemoImeMode::Converting);
        assert_eq!(session.ime_line().reading, "か");
        assert_eq!(session.document(), "");

        // A second Backspace empties the reading and ends the conversion.
        session.step_frame(intent(ti::BACKSPACE)).unwrap();
        assert_eq!(session.ime_line().mode, MemoImeMode::Empty);
        assert_eq!(session.document(), "");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn memo_app_keeps_inline_candidate_visible_at_viewport_bottom() {
        // KOTO-0106: conversion remains on the last visible row, with no reserved
        // panel band reducing the document viewport.
        use koto_core::runtime::text_intent as ti;
        let root = test_root("memo_inline_ime_bottom");
        write_memo_runtime_package(&root, REAL_MEMO_KBC);
        write_skk_dict(&root);
        let mut session = BytecodeAppSession::launch(&root, MEMO_APP_ID).unwrap();
        let intent = |bits| VmInputSnapshot {
            intent_bits: bits,
            ..VmInputSnapshot::empty()
        };
        let chr = |c: char| VmInputSnapshot {
            text_codepoint: c as u32,
            ..VmInputSnapshot::empty()
        };

        // Push the caret onto the last visible row.
        for _ in 0..24 {
            session.step_frame(intent(ti::NEWLINE)).unwrap();
        }
        let caret_before = session
            .draw_rects()
            .iter()
            .find(|&&(_, _, w, h, _)| w == 2 && h == 13)
            .copied()
            .expect("caret rect");
        assert!(
            caret_before.1 >= 210,
            "caret y {} not at the viewport bottom before conversion",
            caret_before.1
        );

        // Start a かさ conversion on that bottom row.
        session.step_frame(intent(ti::IME_TOGGLE)).unwrap();
        session.step_frame(intent(ti::SHIFT)).unwrap();
        for c in ['k', 'a', 's', 'a'] {
            session.step_frame(chr(c)).unwrap();
        }
        session.step_frame(intent(ti::CONVERT)).unwrap();

        // The old panel is gone and the caret remains on the bottom row.
        assert!(!session
            .draw_rects()
            .iter()
            .any(|&(x, y, w, h, _)| (x, y, w, h) == (2, 210, 316, 64)));
        let caret = session
            .draw_rects()
            .iter()
            .find(|&&(_, _, w, h, _)| w == 2 && h == 13)
            .copied()
            .expect("caret rect");
        assert!(
            caret.1 >= 210 && caret.1 + 13 <= 274,
            "caret y {} left the bottom viewport row",
            caret.1
        );

        // Candidate text is inline on that row; navigation and compact status
        // continue to update.
        assert_eq!(session.ime_line().candidate, Some("傘"));
        assert!(session.text().contains(&(8, caret.1, String::from("傘"))));
        assert!(session.text().contains(&(102, 303, String::from("1/2"))));
        session.step_frame(intent(ti::CONVERT)).unwrap();
        assert_eq!(session.ime_line().candidate, Some("笠"));
        assert!(session.text().contains(&(8, caret.1, String::from("笠"))));
        assert!(session.text().contains(&(102, 303, String::from("2/2"))));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn memo_app_inserts_inline_ime_layout_between_existing_text() {
        use koto_core::runtime::text_intent as ti;
        let root = test_root("memo_inline_ime_insert_layout");
        write_memo_runtime_package(&root, REAL_MEMO_KBC);
        write_skk_dict(&root);
        let mut session = BytecodeAppSession::launch(&root, MEMO_APP_ID).unwrap();
        let intent = |bits| VmInputSnapshot {
            intent_bits: bits,
            ..VmInputSnapshot::empty()
        };
        let chr = |c: char| VmInputSnapshot {
            text_codepoint: c as u32,
            ..VmInputSnapshot::empty()
        };

        for c in ['a', 'b', 'c', 'd'] {
            session.step_frame(chr(c)).unwrap();
        }
        session.step_frame(intent(ti::LEFT)).unwrap();
        session.step_frame(intent(ti::LEFT)).unwrap();
        session.step_frame(intent(ti::IME_TOGGLE)).unwrap();
        session.step_frame(chr('k')).unwrap();

        assert_eq!(session.document(), "abcd");
        assert!(session.text().contains(&(8, 26, String::from("ab"))));
        assert!(session.text().contains(&(20, 26, String::from("k"))));
        assert!(session.text().contains(&(26, 26, String::from("cd"))));
        assert!(session
            .draw_rects()
            .iter()
            .any(|&(x, y, w, h, _)| (x, y, w, h) == (26, 26, 2, 13)));

        session.step_frame(intent(ti::CANCEL)).unwrap();
        session.step_frame(intent(ti::SHIFT)).unwrap();
        for c in ['k', 'a', 's', 'a'] {
            session.step_frame(chr(c)).unwrap();
        }
        session.step_frame(intent(ti::CONVERT)).unwrap();

        assert_eq!(session.document(), "abcd");
        assert!(session.text().contains(&(20, 26, String::from("傘"))));
        assert!(session.text().contains(&(32, 26, String::from("cd"))));
        assert!(session
            .draw_rects()
            .iter()
            .any(|&(x, y, w, h, _)| (x, y, w, h) == (32, 26, 2, 13)));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn memo_app_clips_inline_composition_and_keeps_caret_at_right_edge() {
        use koto_core::runtime::text_intent as ti;
        let root = test_root("memo_inline_ime_right_edge");
        write_memo_runtime_package(&root, REAL_MEMO_KBC);
        write_skk_dict(&root);
        let mut session = BytecodeAppSession::launch(&root, MEMO_APP_ID).unwrap();
        let intent = |bits| VmInputSnapshot {
            intent_bits: bits,
            ..VmInputSnapshot::empty()
        };
        let chr = |c: char| VmInputSnapshot {
            text_codepoint: c as u32,
            ..VmInputSnapshot::empty()
        };

        for _ in 0..48 {
            session.step_frame(chr('a')).unwrap();
        }
        session.step_frame(intent(ti::IME_TOGGLE)).unwrap();
        session.step_frame(chr('k')).unwrap();
        session.step_frame(chr('w')).unwrap();

        assert_eq!(session.document(), "a".repeat(48));
        assert!(session.text().contains(&(296, 26, String::from("kw"))));
        assert!(session
            .draw_rects()
            .iter()
            .any(|&(x, y, w, h, _)| (x, y, w, h) == (304, 26, 2, 13)));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn memo_app_accepts_ascii_and_ime_input_after_opening_long_document() {
        use koto_core::runtime::text_intent as ti;
        let root = test_root("memo_open_long_then_type");
        write_memo_runtime_package(&root, REAL_MEMO_KBC);
        write_skk_dict(&root);
        let data_dir = root.join("data").join(MEMO_APP_ID);
        fs::create_dir_all(&data_dir).unwrap();
        let long = (0..80)
            .map(|index| format!("line {index:02}\n"))
            .collect::<String>();
        fs::write(data_dir.join("long.txt"), &long).unwrap();
        fs::write(data_dir.join("memo.txt"), "short").unwrap();

        let mut session = BytecodeAppSession::launch(&root, MEMO_APP_ID).unwrap();
        let intent = |bits| VmInputSnapshot {
            intent_bits: bits,
            ..VmInputSnapshot::empty()
        };
        let chr = |c: char| VmInputSnapshot {
            text_codepoint: c as u32,
            ..VmInputSnapshot::empty()
        };

        session.step_frame(intent(ti::OPEN)).unwrap();
        session.step_frame(intent(ti::COMMIT)).unwrap();
        assert_eq!(session.document(), long);
        assert!(session.editor_scroll_row() > 0);

        session.step_frame(chr('x')).unwrap();
        session.step_frame(intent(ti::IME_TOGGLE)).unwrap();
        session.step_frame(chr('a')).unwrap();

        assert_eq!(session.document(), format!("{long}xあ"));
        assert!(session.editor_scroll_row() > 0);
        assert!(session.editor_cursor_visible_row().is_some());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn memo_app_creates_unnamed_empty_document_then_names_it_on_save() {
        use koto_core::runtime::text_intent as ti;
        let root = test_root("memo_new_document");
        write_memo_runtime_package(&root, REAL_MEMO_KBC);
        write_skk_dict(&root);
        let mut session = BytecodeAppSession::launch(&root, MEMO_APP_ID).unwrap();
        let intent = |bits| VmInputSnapshot {
            intent_bits: bits,
            ..VmInputSnapshot::empty()
        };
        let chr = |c: char| VmInputSnapshot {
            text_codepoint: c as u32,
            ..VmInputSnapshot::empty()
        };

        session.step_frame(chr('o')).unwrap();
        session.step_frame(chr('l')).unwrap();
        session.step_frame(chr('d')).unwrap();
        session.step_frame(intent(ti::NEW)).unwrap();

        assert_eq!(session.document(), "");
        assert!(session.text().contains(&(74, 9, String::from("(新規)"))));
        assert!(session.text().contains(&(264, 6, String::from("未保存"))));
        assert!(!root
            .join("data")
            .join(MEMO_APP_ID)
            .join("note.txt")
            .exists());

        session.step_frame(chr('x')).unwrap();
        session.step_frame(intent(ti::SAVE)).unwrap();
        assert!(session
            .text()
            .contains(&(10, 6, String::from("名前を付けて保存"))));
        for c in "note.txt".chars() {
            session.step_frame(chr(c)).unwrap();
        }
        session.step_frame(intent(ti::NEWLINE)).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("data").join(MEMO_APP_ID).join("note.txt")).unwrap(),
            "x"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn memo_app_handles_repeated_backspace_and_delete_intents() {
        use koto_core::runtime::text_intent as ti;
        let root = test_root("memo_repeated_delete");
        write_memo_runtime_package(&root, REAL_MEMO_KBC);
        write_skk_dict(&root);
        let mut session = BytecodeAppSession::launch(&root, MEMO_APP_ID).unwrap();
        let intent = |bits| VmInputSnapshot {
            intent_bits: bits,
            ..VmInputSnapshot::empty()
        };
        let chr = |c: char| VmInputSnapshot {
            text_codepoint: c as u32,
            ..VmInputSnapshot::empty()
        };

        for c in "abcdef".chars() {
            session.step_frame(chr(c)).unwrap();
        }
        for _ in 0..3 {
            session.step_frame(intent(ti::BACKSPACE)).unwrap();
        }
        assert_eq!(session.document(), "abc");

        session.step_frame(intent(ti::HOME)).unwrap();
        for _ in 0..2 {
            session.step_frame(intent(ti::DELETE)).unwrap();
        }
        assert_eq!(session.document(), "c");
        assert!(session.editor_cursor_visible_row().is_some());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn memo_app_toggles_wrap_and_draws_horizontal_scrollbar() {
        let root = test_root("memo_wrap_toggle");
        write_memo_runtime_package(&root, REAL_MEMO_KBC);
        write_skk_dict(&root);
        let mut session = BytecodeAppSession::launch(&root, MEMO_APP_ID).unwrap();

        // A line wider than the viewport.
        for _ in 0..60 {
            session
                .step_frame(VmInputSnapshot {
                    text_codepoint: 'a' as u32,
                    ..VmInputSnapshot::empty()
                })
                .unwrap();
        }

        // Wrapping on by default: no horizontal scrollbar, and the "折返ON" badge.
        assert!(session.is_wrap());
        assert!(session.text().contains(&(132, 303, String::from("折返ON"))));
        assert!(!session
            .draw_rects()
            .iter()
            .any(|&(x, y, w, h, _)| (x, y, w, h) == (6, 271, 300, 3)));

        // Toggle to no-wrap (host editor setting) and repaint.
        session.toggle_wrap();
        session.step_frame(VmInputSnapshot::empty()).unwrap();
        assert!(!session.is_wrap());
        assert!(session
            .text()
            .contains(&(132, 303, String::from("折返OFF"))));
        assert!(session
            .draw_rects()
            .iter()
            .any(|&(x, y, w, h, _)| (x, y, w, h) == (6, 271, 300, 3)));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_unknown_app_script_token() {
        assert_eq!(parse_app_script("nope"), Err(SimError::InvalidInputScript));
    }

    #[test]
    fn scripted_run_without_audio_has_no_audio_events() {
        // A tiny silent app yields once, then exits.
        let root = test_root("capture_audio");
        let bytecode = koto_compiler::compile(
            "audio.koto",
            "fn main() {\n    yield_frame();\n    exit(0);\n}\n",
        )
        .unwrap();
        write_memo_runtime_package(&root, &bytecode);

        // Drive one frame directly so we can observe the audio events it issued
        // (the scenario inspector only reflects the final, post-exit frame).
        let mut session = BytecodeAppSession::launch(&root, MEMO_APP_ID).unwrap();
        session.step_frame(VmInputSnapshot::empty()).unwrap();
        assert!(session.audio_events().is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    /// The real KotoBlocks bytecode app, kept in sync by the app build loop. The
    /// KOTO-0163 validation below drives its audio the way the shell would.
    const REAL_KOTO_BLOCKS_KBC: &[u8] =
        include_bytes!("../../../../package_inputs/bytecode/koto_blocks.kbc");

    const KOTO_BLOCKS_APP_ID: &str = "dev.koto.games.koto-blocks";

    /// Stage a minimal KotoBlocks package: the real `.kbc` plus a manifest that
    /// declares the audio asset paths (the `asset_paths` permission the audio
    /// hostcalls check), the real SD-resident KMML payloads, and the app's
    /// SRAM/PSRAM budget.
    fn write_koto_blocks_package(root: &Path) {
        fs::create_dir_all(root.join("apps")).unwrap();
        fs::create_dir_all(root.join("bytecode")).unwrap();
        fs::create_dir_all(root.join("audio")).unwrap();
        fs::write(
            root.join("bytecode").join("koto_blocks.kbc"),
            REAL_KOTO_BLOCKS_KBC,
        )
        .unwrap();
        const AUDIO: &[(&str, &[u8])] = &[
            (
                "koto_blocks_bgm.kmml",
                include_bytes!("../../../../apps/koto_blocks/audio/bgm.kmml"),
            ),
            (
                "koto_blocks_move.kmml",
                include_bytes!("../../../../apps/koto_blocks/audio/move.kmml"),
            ),
            (
                "koto_blocks_rotate.kmml",
                include_bytes!("../../../../apps/koto_blocks/audio/rotate.kmml"),
            ),
            (
                "koto_blocks_lock.kmml",
                include_bytes!("../../../../apps/koto_blocks/audio/lock.kmml"),
            ),
            (
                "koto_blocks_clear.kmml",
                include_bytes!("../../../../apps/koto_blocks/audio/clear.kmml"),
            ),
            (
                "koto_blocks_tetris.kmml",
                include_bytes!("../../../../apps/koto_blocks/audio/tetris.kmml"),
            ),
            (
                "koto_blocks_over.kmml",
                include_bytes!("../../../../apps/koto_blocks/audio/over.kmml"),
            ),
        ];
        for (name, bytes) in AUDIO {
            fs::write(root.join("audio").join(name), bytes).unwrap();
        }
        fs::write(
            root.join("apps").join("koto_blocks.kpa.json"),
            r#"{
                "format": "kpa-manifest",
                "version": 1,
                "app_id": "dev.koto.games.koto-blocks",
                "name": "KotoBlocks",
                "runtime": "kotoruntime-bytecode",
                "entry": "bytecode/koto_blocks.kbc",
                "memory": { "sram_work_bytes": 24576, "psram_cache_bytes": 32768 },
                "assets": [
                    { "path": "audio/koto_blocks_bgm.kmml", "type": "audio" },
                    { "path": "audio/koto_blocks_move.kmml", "type": "audio" },
                    { "path": "audio/koto_blocks_rotate.kmml", "type": "audio" },
                    { "path": "audio/koto_blocks_lock.kmml", "type": "audio" },
                    { "path": "audio/koto_blocks_clear.kmml", "type": "audio" },
                    { "path": "audio/koto_blocks_tetris.kmml", "type": "audio" },
                    { "path": "audio/koto_blocks_over.kmml", "type": "audio" }
                ]
            }"#,
        )
        .unwrap();
    }

    /// The real KotoBlocks app reads its mounted KMML payloads and drives the
    /// owned Native KotoAudio BGM/SFX players. The runtime path renders
    /// non-silent audio under a normal gameplay cadence.
    #[test]
    fn koto_blocks_app_drives_sd_loaded_native_audio() {
        use koto_core::runtime::text_intent as ti;
        let root = test_root("koto_blocks_seq_audio");
        write_koto_blocks_package(&root);

        let empty = VmInputSnapshot::empty();
        let intent = |bits: u32| VmInputSnapshot {
            intent_bits: bits,
            ..VmInputSnapshot::empty()
        };

        let mut session = BytecodeAppSession::launch(&root, KOTO_BLOCKS_APP_ID).unwrap();

        // The title screen bakes its tile cache over ~28 frames before it accepts a
        // start intent; step past the bake so the start press is honoured.
        for _ in 0..40 {
            session.step_frame(empty).unwrap();
        }

        // Start: NEWLINE (F1) transitions title -> play and requests the BGM asset.
        session.step_frame(intent(ti::NEWLINE)).unwrap();
        assert!(
            session.audio_events().contains(&AudioEvent::BgmAsset),
            "start frame must issue the BGM asset hostcall"
        );

        // The mounted KMML payload was compiled into the owned KotoAudio player.
        {
            let audio = session.audio_handle();
            let audio = audio.lock().unwrap();
            assert!(
                audio.runtime_bgm_active(),
                "SD-loaded Native KMML BGM was not admitted"
            );
        }

        // Drive gameplay inputs that raise SFX cues (move/rotate/soft-drop), rendering
        // a generous slice of audio between presses so each short cue plays out — the
        // realistic cadence under which no cue should be dropped.
        let mut samples = Vec::new();
        let mut sfx_events = 0usize;
        let inputs = [ti::LEFT, ti::UP, ti::RIGHT, ti::DOWN, ti::DOWN];
        for _ in 0..40 {
            for &bit in &inputs {
                session.step_frame(intent(bit)).unwrap();
                sfx_events += session
                    .audio_events()
                    .iter()
                    .filter(|event| matches!(event, AudioEvent::SfxAsset))
                    .count();
                let mut chunk = vec![0i16; 4096];
                session.render_audio(&mut chunk);
                samples.extend_from_slice(&chunk);
            }
        }

        assert!(
            sfx_events > 0,
            "gameplay inputs raised no SFX asset hostcalls"
        );
        assert!(
            samples.iter().any(|&sample| sample != 0),
            "primary audio path rendered only silence"
        );

        // The primary path stayed healthy across play: BGM still looping, and no SFX
        // cue was dropped at a normal input cadence (the 3-slot SFX budget suffices).
        {
            let audio = session.audio_handle();
            let audio = audio.lock().unwrap();
            assert!(
                audio.runtime_bgm_active(),
                "BGM should still be looping during play"
            );
        }

        fs::remove_dir_all(root).unwrap();
    }

    /// The committed SDK sample bytecode, compiled from `apps/samples/*` and kept
    /// in sync by the app build loop (`harness/build_apps.py --check`). The
    /// KOTO-0178 sweep below drives every sample the way the shell would.
    ///
    /// KotoUI-authored samples (`koto-ui-gallery`, and `file-note` since
    /// KOTO-0221) are deliberately absent: they draw through the retained UI
    /// session instead of per-frame `draw_*` calls and need their package
    /// assets, so their launch/run/exit coverage lives in the dedicated
    /// `koto_ui_app_gallery` / `koto_ui_file_note` integration suites.
    const SDK_SAMPLES: &[(&str, &str, &[u8])] = &[
        (
            "dev.koto.samples.hello-text",
            "sample_hello_text.kbc",
            include_bytes!("../../../../package_inputs/bytecode/sample_hello_text.kbc"),
        ),
        (
            "dev.koto.samples.input-echo",
            "sample_input_echo.kbc",
            include_bytes!("../../../../package_inputs/bytecode/sample_input_echo.kbc"),
        ),
        (
            "dev.koto.samples.counter-loop",
            "sample_counter_loop.kbc",
            include_bytes!("../../../../package_inputs/bytecode/sample_counter_loop.kbc"),
        ),
        (
            "dev.koto.samples.ime-playground",
            "sample_ime_playground.kbc",
            include_bytes!("../../../../package_inputs/bytecode/sample_ime_playground.kbc"),
        ),
        (
            "dev.koto.samples.dirty-rects",
            "sample_dirty_rects.kbc",
            include_bytes!("../../../../package_inputs/bytecode/sample_dirty_rects.kbc"),
        ),
        (
            "dev.koto.samples.actor-array",
            "sample_actor_array.kbc",
            include_bytes!("../../../../package_inputs/bytecode/sample_actor_array.kbc"),
        ),
        (
            "dev.koto.samples.retained-tilemap",
            "sample_retained_tilemap.kbc",
            include_bytes!("../../../../package_inputs/bytecode/sample_retained_tilemap.kbc"),
        ),
        (
            "dev.koto.samples.retained-tilemap-scroll",
            "sample_retained_tilemap_scroll.kbc",
            include_bytes!(
                "../../../../package_inputs/bytecode/sample_retained_tilemap_scroll.kbc"
            ),
        ),
    ];

    fn write_sample_package(root: &Path, app_id: &str, file_name: &str, bytecode: &[u8]) {
        fs::create_dir_all(root.join("apps")).unwrap();
        fs::create_dir_all(root.join("bytecode")).unwrap();
        fs::write(root.join("bytecode").join(file_name), bytecode).unwrap();
        let assets = match app_id {
            "dev.koto.samples.retained-tilemap" => {
                fs::create_dir_all(root.join("maps")).unwrap();
                fs::write(
                    root.join("maps").join("world.map"),
                    include_bytes!("../../../../apps/samples/retained_tilemap/maps/world.map"),
                )
                .unwrap();
                r#", "assets": [{ "path": "maps/world.map", "type": "data" }]"#
            }
            "dev.koto.samples.retained-tilemap-scroll" => {
                fs::create_dir_all(root.join("maps")).unwrap();
                fs::write(
                    root.join("maps").join("world.map"),
                    include_bytes!(
                        "../../../../apps/samples/retained_tilemap_scroll/maps/world.map"
                    ),
                )
                .unwrap();
                r#", "assets": [{ "path": "maps/world.map", "type": "data" }]"#
            }
            _ => "",
        };
        fs::write(
            root.join("apps").join(format!("{file_name}.kpa.json")),
            format!(
                r#"{{
                    "format": "kpa-manifest",
                    "version": 1,
                    "app_id": "{app_id}",
                    "name": "SDK Sample",
                    "runtime": "kotoruntime-bytecode",
                    "entry": "bytecode/{file_name}",
                    "permissions": {{ "fs": "sandbox", "network": false }}{assets}
                }}"#
            ),
        )
        .unwrap();
    }

    /// KOTO-0178: the SDK samples are the SDK's public face and regression
    /// fixtures at once — every committed sample must launch from its packaged
    /// bytecode, run its demo loop (drawing every yielded frame), and exit
    /// cleanly on `INTENT_EXIT` (the intent KotoSim's F10 and the device's F10
    /// legend both deliver, KOTO-0177). This is the sweep that keeps a runtime
    /// or prelude spec change from silently stranding a sample again.
    #[test]
    fn sdk_samples_launch_run_and_exit_on_exit_intent() {
        use koto_core::runtime::text_intent as ti;
        let exit = VmInputSnapshot {
            intent_bits: ti::EXIT,
            ..VmInputSnapshot::empty()
        };

        for &(app_id, file_name, bytecode) in SDK_SAMPLES {
            let root = test_root(&format!("sdk_sample_{file_name}"));
            write_sample_package(&root, app_id, file_name, bytecode);

            let mut session = BytecodeAppSession::launch(&root, app_id)
                .unwrap_or_else(|error| panic!("{app_id}: launch failed: {error:?}"));

            // A representative slice of the demo loop: each yielded frame keeps
            // running and produces immediate or retained pixel output.
            for frame in 0..8 {
                session
                    .step_frame(VmInputSnapshot::empty())
                    .unwrap_or_else(|error| panic!("{app_id}: frame {frame} trapped: {error:?}"));
                assert!(
                    !session.has_exited(),
                    "{app_id}: exited at idle frame {frame} without an EXIT intent"
                );
                assert!(
                    !session.draw_rects().is_empty() || !session.draw_pixels().is_empty(),
                    "{app_id}: yielded frame {frame} drew nothing"
                );
            }

            // The EXIT intent must end the app cleanly on that same frame.
            session
                .step_frame(exit)
                .unwrap_or_else(|error| panic!("{app_id}: exit frame trapped: {error:?}"));
            assert!(session.has_exited(), "{app_id}: EXIT intent was ignored");
            assert_eq!(
                session.result(),
                koto_core::VmRunResult::Exited(0),
                "{app_id}: exit was not clean"
            );

            fs::remove_dir_all(root).unwrap();
        }
    }

    /// KOTO-0178 companion: `dev.koto.sample` (the hand-assembled KOTO-0035
    /// launch fixture, `package_inputs/bytecode/main.kbc`) is a baseline package
    /// whose whole demo is "launch and exit 0 immediately" — pin that so it is
    /// never mistaken for a hung or broken sample.
    #[test]
    fn baseline_sample_package_exits_immediately() {
        let root = test_root("sdk_sample_baseline");
        write_sample_package(
            &root,
            "dev.koto.sample",
            "main.kbc",
            include_bytes!("../../../../package_inputs/bytecode/main.kbc"),
        );

        let report = run_app_scenario(&root, "dev.koto.sample", &[]).unwrap();
        assert_eq!(report.frames, 1);
        assert_eq!(report.result, koto_core::VmRunResult::Exited(0));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn run_app_scenario_launches_app_to_exit() {
        let root = test_root("run_app_scenario_to_exit");
        write_memo_runtime_package(&root, &memo_smoke_kbc());

        let report = run_app_scenario(&root, MEMO_APP_ID, &[]).unwrap();

        assert_eq!(report.app_id, MEMO_APP_ID);
        assert_eq!(report.frames, 1);
        assert_eq!(report.result, koto_core::VmRunResult::Exited(0));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn run_app_scenario_reports_budget_high_water() {
        let root = test_root("run_app_scenario_budget");
        // A frame that uses a local and draws a rect (two host calls), then exits.
        let bytecode = koto_compiler::compile(
            "app.koto",
            "fn main() {\n    let a = 7;\n    draw_rect(1, 2, 3, 4, a);\n    yield_frame();\n    exit(0);\n}\n",
        )
        .unwrap();
        write_memo_runtime_package(&root, &bytecode);

        let report = run_app_scenario(&root, MEMO_APP_ID, &[]).unwrap();
        let budget = &report.budget;

        assert_eq!(budget.app_id, MEMO_APP_ID);
        assert_eq!(budget.frames, report.frames);
        // Capacities are the canonical simulator profile.
        assert_eq!(budget.stack_slots_cap, SIM_VM_STACK_SLOTS as u16);
        assert_eq!(budget.call_depth_cap, SIM_VM_CALL_DEPTH as u16);
        assert_eq!(
            budget.local_slots_cap,
            koto_core::runtime::VM_LOCAL_SLOTS as u16
        );
        assert_eq!(budget.frame_fuel_cap, SIM_FRAME_FUEL);
        // The drawing/local-using frame leaves nonzero, bounded peaks.
        assert!(budget.stack_slots_peak > 0 && budget.stack_slots_peak <= budget.stack_slots_cap);
        assert!(budget.local_slots_peak >= 1);
        assert!(budget.frame_fuel_peak > 0);
        assert!(budget.host_calls_per_frame_peak >= 1);
        assert!(budget.draw_rects_peak >= 1);
        assert!(budget.heap_bytes_peak <= budget.heap_request);
        assert_eq!(
            budget.ui_session_sram_bytes,
            koto_core::UI_SESSION_SRAM_BYTES
        );
        assert_eq!(budget.ui_render_commands_peak, 0);

        // describe round-trips into the parseable one-line key=value form.
        let line = describe_app_budget_report(budget);
        assert!(line.starts_with(&format!("budget app={MEMO_APP_ID} ")));
        assert!(line.contains(&format!("stack_cap={SIM_VM_STACK_SLOTS}")));
        assert!(line.contains("heap_request="));
        assert!(line.contains("heap_budget="));
        assert!(line.contains("frame_time_us_peak="));
        assert!(line.contains("ui_session_sram="));
        assert!(line.contains("ui_render_commands_peak="));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn run_app_scenario_reports_trap_diagnostic() {
        let root = test_root("run_app_scenario_trap");
        write_memo_runtime_package(&root, &trap_kbc());

        let error = run_app_scenario(&root, MEMO_APP_ID, &[]).unwrap_err();

        match error {
            AppRunError::Trap(summary) => {
                assert_eq!(summary.app_id, MEMO_APP_ID);
                assert_eq!(summary.kind, AppFailureKind::RuntimeTrap);
                assert!(summary.describe().contains("runtime-trap"));
                let diagnostic = summary.diagnostic.expect("trap diagnostic");
                assert_eq!(diagnostic.app_id, MEMO_APP_ID);
                assert_eq!(diagnostic.frame, 1);
                assert_eq!(
                    diagnostic.vm_error,
                    Some(koto_core::VmError::DivisionByZero)
                );
                assert_eq!(diagnostic.source, None);
                assert!(summary.detail.contains("DivisionByZero"));
            }
            other => panic!("expected a trap diagnostic, got {other:?}"),
        }

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn run_app_scenario_reports_trap_source_location_from_debug_map() {
        let root = test_root("run_app_scenario_trap_debug");
        let bytecode = koto_compiler::compile(
            "app.koto",
            "fn main() {\n    let x = 1 / 0;\n    exit(x);\n}\n",
        )
        .unwrap();
        write_memo_runtime_package(&root, &bytecode);

        let error = run_app_scenario(&root, MEMO_APP_ID, &[]).unwrap_err();

        match error {
            AppRunError::Trap(summary) => {
                assert_eq!(summary.kind, AppFailureKind::RuntimeTrap);
                let diagnostic = summary.diagnostic.expect("trap diagnostic");
                let source = diagnostic.source.as_ref().expect("debug source location");
                assert_eq!(source.file, "app.koto");
                assert_eq!(source.line, 2);
                assert_eq!(source.col, 5);
                assert!(summary.detail.contains("app.koto:2:5"));
                assert!(summary.detail.contains("DivisionByZero"));
            }
            other => panic!("expected a trap diagnostic, got {other:?}"),
        }

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn run_app_scenario_reports_bad_bytecode_failure_summary() {
        let root = test_root("run_app_scenario_bad_bytecode");
        write_memo_runtime_package(&root, b"not kbc");

        let error = run_app_scenario(&root, MEMO_APP_ID, &[]).unwrap_err();

        match error {
            AppRunError::Launch(summary) => {
                assert_eq!(summary.app_id, MEMO_APP_ID);
                assert_eq!(summary.kind, AppFailureKind::VerificationFailed);
                assert_eq!(summary.detail, "RuntimeVerifyFailed");
                assert_eq!(summary.diagnostic, None);
                assert_eq!(
                    summary.describe(),
                    "app dev.koto.memo failure kind=verification-failed detail=RuntimeVerifyFailed"
                );
            }
            other => panic!("expected a launch failure summary, got {other:?}"),
        }

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn launch_reports_unsupported_runtime_missing_entry_and_bad_bytecode() {
        let unsupported = test_root("launch_unsupported_runtime");
        write_runtime_package(&unsupported, "other-runtime", &minimal_exit_kbc(0));
        let package = load_packages(&unsupported)
            .unwrap()
            .iter()
            .next()
            .copied()
            .unwrap();
        assert_eq!(
            launch_package(&unsupported, &package),
            Err(SimError::InvalidRuntime)
        );
        fs::remove_dir_all(unsupported).unwrap();

        let missing = test_root("launch_missing_entry");
        write_manifest_only(&missing, "kotoruntime-bytecode");
        let package = load_packages(&missing)
            .unwrap()
            .iter()
            .next()
            .copied()
            .unwrap();
        assert_eq!(launch_package(&missing, &package), Err(SimError::Io));
        fs::remove_dir_all(missing).unwrap();

        let bad = test_root("launch_bad_bytecode");
        write_runtime_package(&bad, "kotoruntime-bytecode", b"not kbc");
        let package = load_packages(&bad).unwrap().iter().next().copied().unwrap();
        assert_eq!(
            launch_package(&bad, &package),
            Err(SimError::RuntimeVerifyFailed)
        );
        fs::remove_dir_all(bad).unwrap();
    }

    #[test]
    fn vm_host_calls_write_and_read_sandboxed_files() {
        let root = test_root("vm_host_calls_write_and_read_sandboxed_files");
        fs::create_dir_all(&root).unwrap();

        let write_code = [
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 0),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 8),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 1),
            insn(
                koto_core::runtime::opcode::HOST_CALL,
                koto_core::runtime::host_call::FILE_OPEN,
                0,
            ),
            insn(koto_core::runtime::opcode::DROP, 0, 0),
            insn(koto_core::runtime::opcode::STORE_LOCAL, 0, 0),
            insn(koto_core::runtime::opcode::LOAD_LOCAL, 0, 0),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 16),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 5),
            insn(
                koto_core::runtime::opcode::HOST_CALL,
                koto_core::runtime::host_call::FILE_WRITE,
                0,
            ),
            insn(koto_core::runtime::opcode::DROP, 0, 0),
            insn(koto_core::runtime::opcode::DROP, 0, 0),
            insn(koto_core::runtime::opcode::LOAD_LOCAL, 0, 0),
            insn(
                koto_core::runtime::opcode::HOST_CALL,
                koto_core::runtime::host_call::FILE_CLOSE,
                0,
            ),
            insn(koto_core::runtime::opcode::DROP, 0, 0),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 0),
            insn(
                koto_core::runtime::opcode::HOST_CALL,
                koto_core::runtime::host_call::EXIT,
                0,
            ),
        ];
        let write_bytes = kbc_with_heap(&write_code, 64);
        let program =
            koto_core::verify_kbc(&write_bytes, koto_core::RuntimeLimits::simulator_default())
                .unwrap();
        let mut vm = koto_core::BytecodeVm::<16, 4>::new(&program).unwrap();
        let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
        heap[0..8].copy_from_slice(b"memo.txt");
        heap[16..21].copy_from_slice(b"hello");
        let mut host =
            SimRuntimeHost::new(HostFs::mounted(&root).unwrap(), "dev.koto.test").unwrap();

        assert_eq!(
            vm.execute_frame(
                &write_bytes,
                &program,
                &mut host,
                koto_core::VmInputSnapshot::empty(),
                100,
                &mut heap
            ),
            Ok(koto_core::VmRunResult::Exited(0))
        );
        assert_eq!(
            fs::read(root.join("data").join("dev.koto.test").join("memo.txt")).unwrap(),
            b"hello"
        );

        let read_code = [
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 0),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 8),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 0),
            insn(
                koto_core::runtime::opcode::HOST_CALL,
                koto_core::runtime::host_call::FILE_OPEN,
                0,
            ),
            insn(koto_core::runtime::opcode::DROP, 0, 0),
            insn(koto_core::runtime::opcode::STORE_LOCAL, 0, 0),
            insn(koto_core::runtime::opcode::LOAD_LOCAL, 0, 0),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 32),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 5),
            insn(
                koto_core::runtime::opcode::HOST_CALL,
                koto_core::runtime::host_call::FILE_READ,
                0,
            ),
            insn(koto_core::runtime::opcode::DROP, 0, 0),
            insn(koto_core::runtime::opcode::DROP, 0, 0),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 0),
            insn(
                koto_core::runtime::opcode::HOST_CALL,
                koto_core::runtime::host_call::EXIT,
                0,
            ),
        ];
        let read_bytes = kbc_with_heap(&read_code, 64);
        let program =
            koto_core::verify_kbc(&read_bytes, koto_core::RuntimeLimits::simulator_default())
                .unwrap();
        let mut vm = koto_core::BytecodeVm::<16, 4>::new(&program).unwrap();
        let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
        heap[0..8].copy_from_slice(b"memo.txt");
        let mut host =
            SimRuntimeHost::new(HostFs::mounted(&root).unwrap(), "dev.koto.test").unwrap();

        assert_eq!(
            vm.execute_frame(
                &read_bytes,
                &program,
                &mut host,
                koto_core::VmInputSnapshot::empty(),
                100,
                &mut heap
            ),
            Ok(koto_core::VmRunResult::Exited(0))
        );
        assert_eq!(&heap[32..37], b"hello");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn game2d_tilemap_present_emits_only_set_cells() {
        let root = test_root("game2d_tilemap_present_emits_only_set_cells");
        fs::create_dir_all(&root).unwrap();
        let mut host =
            SimRuntimeHost::new(HostFs::mounted(&root).unwrap(), "dev.koto.test").unwrap();

        // Two distinct 16x16 RGB565 tiles in the app heap: tile A at offset 0,
        // tile B at offset 512 (one tile is 16*16*2 = 512 bytes).
        let mut heap = vec![0u8; 1024];
        heap[0..512].fill(0xA1);
        heap[512..1024].fill(0xB2);

        // Empty layer presents nothing.
        assert_eq!(host.game2d_present(&heap), HostCallOutcome::Ok0);
        assert!(host.draw_pixels.is_empty());

        // Set cell (1, 2) -> tile A (offset 0) and (0, 0) -> tile B (offset 512).
        assert_eq!(host.game2d_set_tile(0, 1, 2, 0), HostCallOutcome::Ok0);
        assert_eq!(host.game2d_set_tile(0, 0, 0, 512), HostCallOutcome::Ok0);
        // Out-of-range cell and non-zero layer are rejected.
        assert_eq!(
            host.game2d_set_tile(0, 10, 0, 0),
            HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT)
        );
        assert_eq!(
            host.game2d_set_tile(1, 0, 0, 0),
            HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT)
        );

        host.clear_frame_draw();
        assert_eq!(host.game2d_present(&heap), HostCallOutcome::Ok0);
        // Cell (0,0)->B at well origin (8, 0); cell (1,2)->A at (8+16, 32).
        assert!(host.draw_pixels.contains(&(8, 0, 16, 16, vec![0xB2; 512])));
        assert!(host
            .draw_pixels
            .contains(&(24, 32, 16, 16, vec![0xA1; 512])));
        assert_eq!(host.draw_pixels.len(), 2);

        // The tilemap is retained across the per-frame draw clear.
        host.clear_frame_draw();
        assert_eq!(host.game2d_clear_layer(0), HostCallOutcome::Ok0);
        assert_eq!(host.game2d_present(&heap), HostCallOutcome::Ok0);
        assert!(host.draw_pixels.is_empty());

        // KOTO-0199: active dimensions and pixel origin are configurable up to 20x20.
        assert_eq!(
            host.game2d_configure_tilemap(0, 20, 20, -16, 32),
            HostCallOutcome::Ok0
        );
        assert_eq!(host.game2d_set_tile(0, 19, 19, 0), HostCallOutcome::Ok0);
        host.clear_frame_draw();
        assert_eq!(host.game2d_present(&heap), HostCallOutcome::Ok0);
        assert!(host
            .draw_pixels
            .contains(&(288, 336, 16, 16, vec![0xA1; 512])));
        assert_eq!(
            host.game2d_configure_tilemap(0, 21, 20, 0, 0),
            HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT)
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn game2d_sprite_present_emits_stamp_cells_over_board() {
        let root = test_root("game2d_sprite_present_emits_stamp_cells_over_board");
        fs::create_dir_all(&root).unwrap();
        let mut host =
            SimRuntimeHost::new(HostFs::mounted(&root).unwrap(), "dev.koto.test").unwrap();

        // Heap: tile A at 0, tile B at 512, and a stamp word at 1024. The stamp's
        // packed u16 0x0014 has nibbles 4, 1, 0, 0 -> cells (dcol,drow) (0,1),(1,0),
        // (0,0),(0,0). Use count=2 so only the first two cells are drawn.
        let mut heap = vec![0u8; 2048];
        heap[0..512].fill(0xA1);
        heap[512..1024].fill(0xB2);
        heap[1024] = 0x14; // low byte: nibble0=4 (->(0,1)), nibble1=1 (->(1,0))
        heap[1025] = 0x00;

        // Undefined stamp / out-of-range sprite are rejected; a clean present is empty.
        assert_eq!(host.game2d_present(&heap), HostCallOutcome::Ok0);
        assert!(host.draw_pixels.is_empty());
        assert_eq!(
            host.game2d_sprite_set(GAME2D_MAX_SPRITES as i32, 0, 0, 0, 0),
            HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT)
        );

        assert_eq!(
            host.game2d_stamp_define(3, 1024, 2, 0),
            HostCallOutcome::Ok0
        );
        // Place sprite 0 = stamp 3 at pixel (40, 48) drawing tile B (offset 512).
        assert_eq!(
            host.game2d_sprite_set(0, 3, 40, 48, 512),
            HostCallOutcome::Ok0
        );

        host.clear_frame_draw();
        assert_eq!(host.game2d_present(&heap), HostCallOutcome::Ok0);
        // Cell 0 (0,1) -> (40, 64); cell 1 (1,0) -> (56, 48), both tile B.
        assert!(host
            .draw_pixels
            .contains(&(40, 64, 16, 16, vec![0xB2; 512])));
        assert!(host
            .draw_pixels
            .contains(&(56, 48, 16, 16, vec![0xB2; 512])));
        assert_eq!(host.draw_pixels.len(), 2);

        // Sprites are retained across the per-frame clear; hiding empties the layer.
        host.clear_frame_draw();
        assert_eq!(host.game2d_present(&heap), HostCallOutcome::Ok0);
        assert_eq!(host.draw_pixels.len(), 2);
        assert_eq!(host.game2d_sprite_hide(0), HostCallOutcome::Ok0);
        host.clear_frame_draw();
        assert_eq!(host.game2d_present(&heap), HostCallOutcome::Ok0);
        assert!(host.draw_pixels.is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn game2d_text_layer_retains_and_diffs_by_id() {
        let root = test_root("game2d_text_layer_retains_and_diffs_by_id");
        fs::create_dir_all(&root).unwrap();
        let mut host =
            SimRuntimeHost::new(HostFs::mounted(&root).unwrap(), "dev.koto.test").unwrap();

        // A clean layer is empty; an out-of-range id is rejected.
        assert!(host.text_items.iter().all(Option::is_none));
        assert_eq!(
            host.game2d_text_set(GAME2D_MAX_TEXT_ITEMS as i32, 0, 0, "x", 1),
            HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT)
        );

        // Set two id-keyed items; they are retained at their slots.
        assert_eq!(
            host.game2d_text_set(1, 250, 212, "000123", 2113),
            HostCallOutcome::Ok0
        );
        assert_eq!(
            host.game2d_text_set(0, 284, 5, "実行中", 11593),
            HostCallOutcome::Ok0
        );
        let item = host.text_items[1].as_ref().unwrap();
        assert_eq!(
            (item.x, item.y, item.rgb565, item.text.as_str()),
            (250, 212, 2113, "000123")
        );

        // Items survive the per-frame draw clear (like the sprite/tilemap layers).
        host.clear_frame_draw();
        assert_eq!(host.text_items[0].as_ref().unwrap().text, "実行中");
        assert_eq!(host.text_items[1].as_ref().unwrap().text, "000123");

        // Updating an id replaces only that item; hiding clears its slot.
        assert_eq!(
            host.game2d_text_set(1, 250, 212, "000456", 2113),
            HostCallOutcome::Ok0
        );
        assert_eq!(host.text_items[1].as_ref().unwrap().text, "000456");
        assert_eq!(host.game2d_text_hide(0), HostCallOutcome::Ok0);
        assert!(host.text_items[0].is_none());
        assert!(host.text_items[1].is_some());

        // clear_all empties every slot.
        assert_eq!(host.game2d_text_clear_all(), HostCallOutcome::Ok0);
        assert!(host.text_items.iter().all(Option::is_none));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn game2d_static_layer_captures_and_retains_across_frame_clear() {
        let root = test_root("game2d_static_layer_captures_and_retains_across_frame_clear");
        fs::create_dir_all(&root).unwrap();
        let mut host =
            SimRuntimeHost::new(HostFs::mounted(&root).unwrap(), "dev.koto.test").unwrap();

        // Outside a capture, draws land in the per-frame immediate lists.
        assert_eq!(host.draw_rect(0, 0, 320, 320, 1), HostCallOutcome::Ok0);
        assert_eq!(host.draw_rects.len(), 1);
        assert!(host.static_rects.is_empty());

        // Between begin/end, draws are captured into the retained static layer.
        assert_eq!(host.game2d_static_begin(), HostCallOutcome::Ok0);
        assert_eq!(host.draw_rect(8, 0, 160, 320, 2), HostCallOutcome::Ok0);
        assert_eq!(host.draw_text_color(10, 5, "NEXT", 7), HostCallOutcome::Ok0);
        assert_eq!(host.game2d_static_end(), HostCallOutcome::Ok0);
        assert_eq!(host.static_rects, vec![(8, 0, 160, 320, 2)]);
        assert_eq!(host.static_text, vec![(10, 5, "NEXT".to_string())]);
        assert_eq!(host.static_text_colors, vec![7]);
        // The immediate list still holds only the pre-capture rect.
        assert_eq!(host.draw_rects.len(), 1);

        // After capture, draws route back to the immediate lists, and the static
        // layer survives the per-frame draw clear (like the tilemap).
        assert_eq!(host.draw_rect(20, 20, 16, 16, 3), HostCallOutcome::Ok0);
        host.clear_frame_draw();
        assert!(host.draw_rects.is_empty());
        assert_eq!(host.static_rects, vec![(8, 0, 160, 320, 2)]);
        assert_eq!(host.static_text.len(), 1);

        // A fresh begin clears the previous build before recapturing.
        assert_eq!(host.game2d_static_begin(), HostCallOutcome::Ok0);
        assert!(host.static_rects.is_empty());
        assert!(host.static_text.is_empty());
        assert_eq!(host.draw_rect(1, 2, 3, 4, 9), HostCallOutcome::Ok0);
        assert_eq!(host.game2d_static_end(), HostCallOutcome::Ok0);
        assert_eq!(host.static_rects, vec![(1, 2, 3, 4, 9)]);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn vm_host_calls_report_text_and_file_failures_on_stack() {
        let root = test_root("vm_host_calls_report_text_and_file_failures_on_stack");
        fs::create_dir_all(&root).unwrap();
        let mut host =
            SimRuntimeHost::new(HostFs::mounted(&root).unwrap(), "dev.koto.test").unwrap();

        let bad_path_code = [
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 0),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 10),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 0),
            insn(
                koto_core::runtime::opcode::HOST_CALL,
                koto_core::runtime::host_call::FILE_OPEN,
                0,
            ),
        ];
        let bytes = kbc_with_heap(&bad_path_code, 32);
        let program =
            koto_core::verify_kbc(&bytes, koto_core::RuntimeLimits::simulator_default()).unwrap();
        let mut vm = koto_core::BytecodeVm::<16, 4>::new(&program).unwrap();
        let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
        heap[0..10].copy_from_slice(b"../bad.txt");

        assert_eq!(
            vm.execute_frame(
                &bytes,
                &program,
                &mut host,
                koto_core::VmInputSnapshot::empty(),
                4,
                &mut heap
            ),
            Ok(koto_core::VmRunResult::FuelExhausted)
        );
        // Fixed-arity failure: a result slot (0) then a negative status carrying
        // the error code. `file_open` has one result (the handle).
        assert_eq!(
            vm.pop_value().unwrap(),
            -koto_core::HostErrorCode::PERMISSION_DENIED.0
        );
        assert_eq!(vm.pop_value().unwrap(), 0);

        let missing_file_code = [
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 0),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 11),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 0),
            insn(
                koto_core::runtime::opcode::HOST_CALL,
                koto_core::runtime::host_call::FILE_OPEN,
                0,
            ),
        ];
        let bytes = kbc_with_heap(&missing_file_code, 32);
        let program =
            koto_core::verify_kbc(&bytes, koto_core::RuntimeLimits::simulator_default()).unwrap();
        let mut vm = koto_core::BytecodeVm::<16, 4>::new(&program).unwrap();
        let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
        heap[0..11].copy_from_slice(b"missing.txt");

        assert_eq!(
            vm.execute_frame(
                &bytes,
                &program,
                &mut host,
                koto_core::VmInputSnapshot::empty(),
                4,
                &mut heap
            ),
            Ok(koto_core::VmRunResult::FuelExhausted)
        );
        assert_eq!(
            vm.pop_value().unwrap(),
            -koto_core::HostErrorCode::IO_ERROR.0
        );
        assert_eq!(vm.pop_value().unwrap(), 0);

        let bad_text_code = [
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 1),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 2),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 0),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 1),
            insn(
                koto_core::runtime::opcode::HOST_CALL,
                koto_core::runtime::host_call::DRAW_TEXT,
                0,
            ),
        ];
        let bytes = kbc_with_heap(&bad_text_code, 8);
        let program =
            koto_core::verify_kbc(&bytes, koto_core::RuntimeLimits::simulator_default()).unwrap();
        let mut vm = koto_core::BytecodeVm::<16, 4>::new(&program).unwrap();
        let mut heap = vec![0u8; program.header().max_heap_bytes as usize];
        heap[0] = 0xFF;

        assert_eq!(
            vm.execute_frame(
                &bytes,
                &program,
                &mut host,
                koto_core::VmInputSnapshot::empty(),
                5,
                &mut heap
            ),
            Ok(koto_core::VmRunResult::FuelExhausted)
        );
        // `draw_text` has no result, so failure pushes only the negative status.
        assert_eq!(
            vm.pop_value().unwrap(),
            -koto_core::HostErrorCode::BAD_ARGUMENT.0
        );

        let bad_pointer_code = [
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 1),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 2),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 30),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 4),
            insn(
                koto_core::runtime::opcode::HOST_CALL,
                koto_core::runtime::host_call::DRAW_TEXT,
                0,
            ),
        ];
        let bytes = kbc_with_heap(&bad_pointer_code, 32);
        let program =
            koto_core::verify_kbc(&bytes, koto_core::RuntimeLimits::simulator_default()).unwrap();
        let mut vm = koto_core::BytecodeVm::<16, 4>::new(&program).unwrap();
        let mut heap = vec![0u8; program.header().max_heap_bytes as usize];

        assert_eq!(
            vm.execute_frame(
                &bytes,
                &program,
                &mut host,
                koto_core::VmInputSnapshot::empty(),
                5,
                &mut heap
            ),
            Err(koto_core::VmError::MemoryOutOfBounds)
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn inspector_reports_vm_and_host_state_after_yielded_frame() {
        let root = test_root("inspector_yielded_frame");
        write_memo_runtime_package(&root, &draw_then_yield_kbc());

        let mut session = BytecodeAppSession::launch(&root, MEMO_APP_ID).unwrap();

        // Before any frame the inspector reports a not-yet-run app.
        let before = session.inspect();
        assert_eq!(before.app_id, MEMO_APP_ID);
        assert_eq!(before.frame, 0);
        assert_eq!(before.last_host_call, None);
        assert_eq!(before.last_host_call_name(), "<none>");
        assert_eq!(before.frame_fuel_used, 0);
        assert_eq!(before.text_draws, 0);

        let input = koto_core::VmInputSnapshot {
            text_codepoint: u32::from('z'),
            ..koto_core::VmInputSnapshot::empty()
        };
        let result = session.step_frame(input).unwrap();
        assert_eq!(result, koto_core::VmRunResult::Yielded);

        // After one yielded frame that made a draw_text host call, every field
        // the inspector tracks has moved off its initial value.
        let after = session.inspect();
        assert_eq!(after.frame, 1);
        assert_eq!(after.run_state, koto_core::VmRunResult::Yielded);
        assert_eq!(
            after.last_host_call,
            Some(koto_core::runtime::host_call::YIELD_FRAME)
        );
        assert_eq!(after.last_host_call_name(), "yield_frame");
        assert!(after.frame_fuel_used > before.frame_fuel_used);
        assert_eq!(after.last_vm_error, None);
        assert_eq!(after.last_input.text_codepoint, u32::from('z'));
        assert_eq!(after.text_draws, 1);
        assert_eq!(after.draw_rects, 0);
        assert_eq!(after.open_files, 0);
        assert!(describe_inspector_report(&after).contains("state=yielded"));
        assert!(describe_inspector_report(&after).contains("last_host_call=yield_frame"));

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn inspector_reports_open_sandboxed_file_handles() {
        let root = test_root("inspector_open_files");
        write_memo_runtime_package(&root, &open_then_yield_kbc());

        let mut session = BytecodeAppSession::launch(&root, MEMO_APP_ID).unwrap();
        let result = session
            .step_frame(koto_core::VmInputSnapshot::empty())
            .unwrap();

        assert_eq!(result, koto_core::VmRunResult::Yielded);
        let report = session.inspect();
        // The handle indexes the per-app sandbox; the inspector reports occupancy
        // (one open handle) without surfacing any host path.
        assert_eq!(report.open_files, 1);
        assert_eq!(
            report.last_host_call,
            Some(koto_core::runtime::host_call::YIELD_FRAME)
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn inspector_reports_last_vm_error_after_trap() {
        let root = test_root("inspector_trap");
        write_memo_runtime_package(&root, &trap_kbc());

        let mut session = BytecodeAppSession::launch(&root, MEMO_APP_ID).unwrap();
        assert!(session
            .step_frame(koto_core::VmInputSnapshot::empty())
            .is_err());

        let report = session.inspect();
        assert_eq!(
            report.last_vm_error,
            Some(koto_core::VmError::DivisionByZero)
        );
        assert!(describe_inspector_report(&report).contains("error=DivisionByZero"));

        fs::remove_dir_all(root).unwrap();
    }

    fn test_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("koto-sim-{name}-{unique}"))
    }

    #[test]
    fn shell_prefs_roundtrip_persists_favorites_and_sort() {
        let root = test_root("shell_prefs_roundtrip");
        let mut packages = PackageList::new();
        packages.push(PackageInfo::new("dev.koto.a", "A").unwrap());
        packages.push(PackageInfo::new("dev.koto.b", "B").unwrap());

        let mut shell = ShellState::new(packages.clone());
        assert!(shell.set_favorite_by_app_id("dev.koto.b", true));
        shell.set_sort_mode(SortMode::Favorite);
        save_shell_prefs(&shell, &root).unwrap();

        let mut restored = ShellState::new(packages);
        assert_eq!(restored.sort_mode(), SortMode::Default);
        apply_shell_prefs(&mut restored, &root);

        assert_eq!(restored.sort_mode(), SortMode::Favorite);
        let b_favorite = restored
            .packages()
            .iter()
            .find(|p| p.app_id() == "dev.koto.b")
            .map(|p| p.is_favorite())
            .unwrap_or(false);
        assert!(b_favorite);

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn legacy_shell_locale_preference_is_ignored_without_losing_launcher_state() {
        let root = test_root("shell_prefs_locale_migration");
        let prefs_dir = root.join("data").join(SHELL_PREFS_APP_ID);
        fs::create_dir_all(&prefs_dir).unwrap();
        fs::write(
            prefs_dir.join("prefs.txt"),
            "locale=ja-JP\nsort=Favorite\ncategory=Tools\nfav=dev.koto.b\n",
        )
        .unwrap();

        let mut packages = PackageList::new();
        packages.push(PackageInfo::new("dev.koto.a", "A").unwrap());
        packages.push(PackageInfo::new("dev.koto.b", "B").unwrap());
        let mut shell = ShellState::new(packages);
        apply_shell_prefs(&mut shell, &root);

        assert_eq!(shell.locale(), koto_core::Locale::EnUs);
        assert_eq!(shell.sort_mode(), SortMode::Favorite);
        assert_eq!(shell.category_filter(), Some("Tools"));
        assert!(shell
            .packages()
            .iter()
            .find(|package| package.app_id() == "dev.koto.b")
            .unwrap()
            .is_favorite());

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn shell_frames_differ_for_english_japanese_and_pseudolocale() {
        const FONT: &[u8] = include_bytes!("../../../../assets/fonts/mplus12.kfont");

        let font = BitmapFont::from_bytes(FONT).unwrap();
        let mut packages = PackageList::new();
        packages.push(PackageInfo::new("dev.koto.one", "One").unwrap());
        let mut shell = ShellState::new(packages);
        shell.set_save_status(koto_core::shell::SaveStatus::Saved);

        let mut hashes = [0u64; 3];
        for (index, locale) in [
            koto_core::Locale::EnUs,
            koto_core::Locale::JaJp,
            koto_core::Locale::QpsPloc,
        ]
        .into_iter()
        .enumerate()
        {
            let mut config = koto_core::ConfigService::new();
            config.set_locale(locale);
            shell.apply_config_snapshot(config.snapshot());
            let mut framebuffer = Framebuffer::new(320, 320);
            shell.paint(&mut framebuffer.as_canvas(), &font);
            hashes[index] = framebuffer
                .as_canvas()
                .pixels()
                .iter()
                .fold(0xcbf29ce484222325u64, |hash, byte| {
                    (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
                });
        }

        assert_ne!(hashes[0], hashes[1]);
        assert_ne!(hashes[0], hashes[2]);
        assert_ne!(hashes[1], hashes[2]);
    }

    #[test]
    fn system_config_uses_newest_valid_slot_and_survives_corruption() {
        let root = test_root("system_config_slots");
        let mut config = koto_core::ConfigService::default();
        save_system_config(&config, &root).unwrap();
        assert_eq!(load_system_config(&root).locale(), koto_core::Locale::EnUs);

        assert!(config.set_locale(koto_core::Locale::JaJp));
        assert!(config.set_utc_offset(koto_core::UtcOffset::from_minutes(9 * 60).unwrap()));
        assert!(config.set_sntp_server(koto_core::SntpServer::NictJapan));
        save_system_config(&config, &root).unwrap();
        let loaded = load_system_config(&root);
        assert_eq!(loaded.locale(), koto_core::Locale::JaJp);
        assert_eq!(loaded.utc_offset().minutes(), 9 * 60);
        assert_eq!(loaded.sntp_server(), koto_core::SntpServer::NictJapan);

        fs::write(root.join("data/dev.koto.config/config-b.bin"), b"torn").unwrap();
        let recovered = load_system_config(&root);
        assert_eq!(recovered.locale(), koto_core::Locale::EnUs);
        assert_eq!(recovered.utc_offset(), koto_core::UtcOffset::default());
        assert_eq!(recovered.sntp_server(), koto_core::SntpServer::NtpPool);
        assert_eq!(recovered.generation(), 1);

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ui_capabilities_host_call_uses_saved_locale_snapshot() {
        use koto_core::runtime::{host_call, opcode};

        let root = test_root("ui_capabilities_locale");
        let mut config = koto_core::ConfigService::new();
        assert!(config.set_locale(koto_core::Locale::JaJp));
        save_system_config(&config, &root).unwrap();

        let instructions = [
            insn(opcode::PUSH_I16, 0, 0),  // dst_ptr
            insn(opcode::PUSH_I16, 0, 64), // dst_max
            insn(opcode::HOST_CALL, host_call::UI_CAPABILITIES, 0),
            insn(opcode::DROP, 0, 0),      // status
            insn(opcode::DROP, 0, 0),      // bytes_written
            insn(opcode::PUSH_I16, 0, 0),  // x
            insn(opcode::PUSH_I16, 0, 0),  // y
            insn(opcode::PUSH_I16, 0, 32), // width: expose all 64 bytes
            insn(opcode::PUSH_I16, 0, 1),  // height
            insn(opcode::PUSH_I16, 0, 0),  // ptr
            insn(opcode::PUSH_I16, 0, 64), // len
            insn(opcode::HOST_CALL, host_call::DRAW_PIXELS_RGB565, 0),
            insn(opcode::DROP, 0, 0),
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::HOST_CALL, host_call::EXIT, 0),
        ];
        write_runtime_package(
            &root,
            KOTORUNTIME_BYTECODE,
            &kbc_with_heap(&instructions, 256),
        );

        let mut session = BytecodeAppSession::launch(&root, "dev.koto.test").unwrap();
        assert_eq!(
            session.step_frame(VmInputSnapshot::empty()).unwrap(),
            VmRunResult::Exited(0)
        );
        let bytes = &session.draw_pixels()[0].4;
        assert_eq!(&bytes[0..4], b"KUC1");
        assert_eq!(bytes[32], 5);
        assert_eq!(u32::from_le_bytes(bytes[36..40].try_into().unwrap()), 2);
        assert_eq!(&bytes[40..45], b"ja-JP");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ui_present_builds_retained_commands_and_idle_keeps_them() {
        let root = test_root("ui_present_retained");
        fs::create_dir_all(&root).unwrap();
        let mut host =
            SimRuntimeHost::new(HostFs::mounted(&root).unwrap(), "dev.koto.test").unwrap();
        let hex =
            include_str!("../../../../harness/fixtures/koto_ui_abi/valid_panel_button_mount.hex")
                .trim()
                .as_bytes();
        let nibble = |byte| match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            _ => panic!("bad fixture hex"),
        };
        let packet: Vec<u8> = (0..hex.len() / 2)
            .map(|index| nibble(hex[index * 2]) << 4 | nibble(hex[index * 2 + 1]))
            .collect();

        assert_eq!(host.ui_mount(&packet), HostCallOutcome::Ok0);
        assert_eq!(host.ui_present(), HostCallOutcome::Ok0);
        // The representative ABI scene expands two frame strokes and one focus
        // stroke into retained rectangles: 15 rects + 2 text commands. This is
        // the simulator/device parity boundary used by the KOTO-0218 record.
        assert_eq!(host.ui_rects.len(), 15);
        assert_eq!(host.ui_text.len(), 2);
        assert!(host.ui_text.iter().any(|(_, _, text)| text == "OK"));
        let retained = (host.ui_rects.clone(), host.ui_text.clone());

        host.clear_frame_draw();
        assert_eq!(host.ui_present(), HostCallOutcome::Ok0);
        assert_eq!((host.ui_rects.clone(), host.ui_text.clone()), retained);
        assert!(host.draw_rects.is_empty());
        assert!(host.text.is_empty());

        let mut bad_utf8 = packet.clone();
        bad_utf8[141] = 0xff;
        assert_eq!(
            host.ui_mount(&bad_utf8),
            HostCallOutcome::Err(koto_core::HostErrorCode::BAD_ARGUMENT)
        );
        let mut unsupported = packet.clone();
        unsupported[4..6].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(
            host.ui_mount(&unsupported),
            HostCallOutcome::Err(koto_core::HostErrorCode::UNSUPPORTED)
        );
        let mut too_many = packet.clone();
        too_many[12..14].copy_from_slice(&33u16.to_le_bytes());
        assert_eq!(
            host.ui_mount(&too_many),
            HostCallOutcome::Err(koto_core::HostErrorCode::NO_MEMORY)
        );
        assert_eq!((host.ui_rects.clone(), host.ui_text.clone()), retained);
        assert_eq!(host.ui_present(), HostCallOutcome::Ok0);
        assert_eq!((host.ui_rects.clone(), host.ui_text.clone()), retained);

        host.ui_frame_begin(VmInputSnapshot {
            pressed_bits: 1 << 4,
            ..VmInputSnapshot::empty()
        });
        let mut event = [0u8; 64];
        assert_eq!(host.ui_poll_event(&mut event), HostCallOutcome::Ok1(32));
        assert_eq!(&event[..4], b"KUE1");
        assert_eq!(event[12], 1);
        assert_eq!(u16::from_le_bytes([event[14], event[15]]), 2);

        assert_eq!(host.ui_reset(), HostCallOutcome::Ok0);
        assert!(host.ui_rects.is_empty());
        assert!(host.ui_text.is_empty());
        assert_eq!(
            host.ui_poll_event(&mut event),
            HostCallOutcome::Err(koto_core::HostErrorCode::NOT_FOUND)
        );

        let mut ime_packet = packet.clone();
        ime_packet.resize(174, 0);
        ime_packet[8..12].copy_from_slice(&174u32.to_le_bytes());
        ime_packet[24..28].copy_from_slice(&38u32.to_le_bytes());
        ime_packet[92] = 5;
        ime_packet[94..96].copy_from_slice(&1u16.to_le_bytes());
        ime_packet[112..116].copy_from_slice(&6u32.to_le_bytes());
        ime_packet[118..120].copy_from_slice(&32u16.to_le_bytes());
        ime_packet[120..124].copy_from_slice(&(-1i32).to_le_bytes());
        assert_eq!(host.ui_mount(&ime_packet), HostCallOutcome::Ok0);
        host.ui_frame_begin(VmInputSnapshot {
            pressed_bits: 1 << 4,
            ..VmInputSnapshot::empty()
        });
        host.ui_frame_begin(VmInputSnapshot {
            intent_bits: koto_core::runtime::text_intent::IME_TOGGLE,
            ..VmInputSnapshot::empty()
        });
        host.ui_frame_begin(VmInputSnapshot {
            text_codepoint: 'k' as u32,
            ..VmInputSnapshot::empty()
        });
        assert_eq!(host.ui_present(), HostCallOutcome::Ok0);
        assert!(host.ui_text.iter().any(|(_, _, text)| text == "k"));
        host.ui_frame_begin(VmInputSnapshot {
            text_codepoint: 'a' as u32,
            ..VmInputSnapshot::empty()
        });
        assert_eq!(host.ui_poll_event(&mut event), HostCallOutcome::Ok1(35));
        assert_eq!(event[12], 3);
        assert_eq!(&event[32..35], "か".as_bytes());
        assert_eq!(host.ui_present(), HostCallOutcome::Ok0);
        let text_x = host
            .ui_text
            .iter()
            .find_map(|(x, _, text)| (text == "か").then_some(*x))
            .expect("committed kana text");
        assert!(
            host.ui_rects.iter().any(|&(x, _, w, h, color)| {
                x == text_x + 12
                    && w == 1
                    && h == 12
                    && color == i32::from(koto_ui::Theme::DARK.focus.0)
            }),
            "text={:?} rects={:?}",
            host.ui_text,
            host.ui_rects
        );
        assert_eq!(host.ui_reset(), HostCallOutcome::Ok0);
        assert!(!host.ime.is_enabled());
        assert_eq!(host.ui_ime_owner, u16::MAX);

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn invalid_or_oversized_system_config_falls_back_to_defaults() {
        let root = test_root("system_config_invalid");
        let dir = root.join("data/dev.koto.config");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("config-a.bin"),
            vec![0xff; koto_core::CONFIG_FORMAT_MAX_BYTES + 1],
        )
        .unwrap();
        fs::write(dir.join("config-b.bin"), b"bad").unwrap();

        assert_eq!(
            load_system_config(&root),
            koto_core::ConfigService::default()
        );
        fs::remove_dir_all(&root).ok();
    }

    fn write_runtime_package(root: &Path, runtime: &str, bytecode: &[u8]) {
        write_manifest_only(root, runtime);
        fs::create_dir_all(root.join("bytecode")).unwrap();
        fs::write(root.join("bytecode").join("main.kbc"), bytecode).unwrap();
    }

    fn write_memo_runtime_package(root: &Path, bytecode: &[u8]) {
        fs::create_dir_all(root.join("apps")).unwrap();
        fs::create_dir_all(root.join("bytecode")).unwrap();
        fs::write(root.join("bytecode").join("memo.kbc"), bytecode).unwrap();
        fs::write(
            root.join("apps").join("memo.kpa.json"),
            r#"{
                "format": "kpa-manifest",
                "version": 1,
                "app_id": "dev.koto.memo",
                "name": "Koto Memo",
                "runtime": "kotoruntime-bytecode",
                "entry": "bytecode/memo.kbc"
            }"#,
        )
        .unwrap();
    }

    fn write_manifest_only(root: &Path, runtime: &str) {
        fs::create_dir_all(root.join("apps")).unwrap();
        fs::write(
            root.join("apps").join("test.kpa.json"),
            format!(
                r#"{{
                    "format": "kpa-manifest",
                    "version": 1,
                    "app_id": "dev.koto.test",
                    "name": "Test App",
                    "runtime": "{runtime}",
                    "entry": "bytecode/main.kbc"
                }}"#
            ),
        )
        .unwrap();
    }

    fn minimal_exit_kbc(code: i16) -> Vec<u8> {
        let instructions = [
            insn(koto_core::runtime::opcode::PUSH_I16, 0, code as u16),
            insn(
                koto_core::runtime::opcode::HOST_CALL,
                koto_core::runtime::host_call::EXIT,
                0,
            ),
        ];
        let bytecode_size = koto_core::KBC_HEADER_SIZE + instructions.len() * 4;
        let mut bytes = vec![0u8; bytecode_size];
        bytes[0..4].copy_from_slice(&koto_core::KBC_MAGIC);
        bytes[4..6].copy_from_slice(&koto_core::KBC_VERSION_MAJOR.to_le_bytes());
        bytes[6..8].copy_from_slice(&koto_core::KBC_VERSION_MINOR.to_le_bytes());
        bytes[8..12].copy_from_slice(&(koto_core::KBC_HEADER_SIZE as u32).to_le_bytes());
        bytes[16..20].copy_from_slice(&(bytecode_size as u32).to_le_bytes());
        bytes[20..24].copy_from_slice(&(koto_core::KBC_HEADER_SIZE as u32).to_le_bytes());
        bytes[24..28].copy_from_slice(&((instructions.len() * 4) as u32).to_le_bytes());
        bytes[40..42].copy_from_slice(&(SIM_VM_STACK_SLOTS as u16).to_le_bytes());
        bytes[42..44].copy_from_slice(&4u16.to_le_bytes());
        bytes[44..48].copy_from_slice(&(256u32).to_le_bytes());
        bytes[48..50].copy_from_slice(&koto_core::HOST_ABI_MAJOR.to_le_bytes());
        bytes[50..52].copy_from_slice(&koto_core::HOST_ABI_MINOR.to_le_bytes());
        for (index, word) in instructions.iter().enumerate() {
            let offset = koto_core::KBC_HEADER_SIZE + index * 4;
            bytes[offset..offset + 4].copy_from_slice(word);
        }
        bytes
    }

    fn memo_smoke_kbc() -> Vec<u8> {
        let mut instructions = Vec::new();
        emit_bytes(&mut instructions, 0, b"memo.txt");
        emit_bytes(&mut instructions, 16, b"koto memo\n");
        instructions.extend([
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 0),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 0),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 16),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 10),
            insn(
                koto_core::runtime::opcode::HOST_CALL,
                koto_core::runtime::host_call::DRAW_TEXT,
                0,
            ),
            insn(koto_core::runtime::opcode::DROP, 0, 0),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 0),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 8),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 1),
            insn(
                koto_core::runtime::opcode::HOST_CALL,
                koto_core::runtime::host_call::FILE_OPEN,
                0,
            ),
            insn(koto_core::runtime::opcode::DROP, 0, 0),
            insn(koto_core::runtime::opcode::STORE_LOCAL, 0, 0),
            insn(koto_core::runtime::opcode::LOAD_LOCAL, 0, 0),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 16),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 10),
            insn(
                koto_core::runtime::opcode::HOST_CALL,
                koto_core::runtime::host_call::FILE_WRITE,
                0,
            ),
            insn(koto_core::runtime::opcode::DROP, 0, 0),
            insn(koto_core::runtime::opcode::DROP, 0, 0),
            insn(koto_core::runtime::opcode::LOAD_LOCAL, 0, 0),
            insn(
                koto_core::runtime::opcode::HOST_CALL,
                koto_core::runtime::host_call::FILE_CLOSE,
                0,
            ),
            insn(koto_core::runtime::opcode::DROP, 0, 0),
            insn(koto_core::runtime::opcode::PUSH_I16, 0, 0),
            insn(
                koto_core::runtime::opcode::HOST_CALL,
                koto_core::runtime::host_call::EXIT,
                0,
            ),
        ]);
        kbc_with_heap(&instructions, 256u32)
    }

    /// Build a `.kbc` that feeds a key sequence into the host IME+editor through
    /// the VM. Each `(char, shift)` entry emits either a Sticky Shift key (when
    /// `shift`) or a character key; `convert` appends an `ime_convert` call.
    fn feed_keys_kbc(keys: &[(char, bool)], convert: bool) -> Vec<u8> {
        use koto_core::runtime::{host_call, ime_key, opcode};
        let mut instructions = vec![
            insn(opcode::PUSH_I16, 0, ime_key::TOGGLE as u16),
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::HOST_CALL, host_call::IME_FEED_KEY, 0),
            insn(opcode::DROP, 0, 0),
        ];
        for &(ch, shift) in keys {
            let (kind, codepoint) = if shift {
                (ime_key::SHIFT, 0)
            } else {
                (ime_key::CHARACTER, ch as i32)
            };
            instructions.push(insn(opcode::PUSH_I16, 0, kind as u16));
            instructions.push(insn(opcode::PUSH_I16, 0, codepoint as u16));
            instructions.push(insn(opcode::HOST_CALL, host_call::IME_FEED_KEY, 0));
            instructions.push(insn(opcode::DROP, 0, 0));
        }
        if convert {
            instructions.push(insn(opcode::HOST_CALL, host_call::IME_CONVERT, 0));
            instructions.push(insn(opcode::DROP, 0, 0));
        }
        instructions.push(insn(opcode::PUSH_I16, 0, 0));
        instructions.push(insn(opcode::HOST_CALL, host_call::EXIT, 0));
        kbc_with_heap(&instructions, 256u32)
    }

    fn write_skk_dict(root: &Path) {
        fs::create_dir_all(root.join("dict")).unwrap();
        fs::write(root.join(SKK_DICT_PATH), MEMO_VALIDATION_DICT).unwrap();
    }

    /// A `.kbc` that traps on a division by zero, for diagnostics tests.
    fn trap_kbc() -> Vec<u8> {
        use koto_core::runtime::{host_call, opcode};
        let instructions = [
            insn(opcode::PUSH_I16, 0, 1),
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::DIV_I32, 0, 0),
            insn(opcode::PUSH_I16, 0, 0),
            insn(opcode::HOST_CALL, host_call::EXIT, 0),
        ];
        kbc_with_heap(&instructions, 256u32)
    }

    /// A `.kbc` that draws one line of text then yields, for inspector tests:
    /// it runs a host call (`draw_text`) and stays active across the frame.
    fn draw_then_yield_kbc() -> Vec<u8> {
        use koto_core::runtime::{host_call, opcode};
        let mut instructions = Vec::new();
        emit_bytes(&mut instructions, 0, b"hi");
        instructions.extend([
            insn(opcode::PUSH_I16, 0, 0), // x
            insn(opcode::PUSH_I16, 0, 0), // y
            insn(opcode::PUSH_I16, 0, 0), // ptr
            insn(opcode::PUSH_I16, 0, 2), // len
            insn(opcode::HOST_CALL, host_call::DRAW_TEXT, 0),
            insn(opcode::DROP, 0, 0), // drop draw status
            insn(opcode::HOST_CALL, host_call::YIELD_FRAME, 0),
        ]);
        kbc_with_heap(&instructions, 256u32)
    }

    /// A `.kbc` that opens a sandboxed file and yields without closing it, so the
    /// inspector observes a live open handle across the frame boundary.
    fn open_then_yield_kbc() -> Vec<u8> {
        use koto_core::runtime::{host_call, opcode};
        let mut instructions = Vec::new();
        emit_bytes(&mut instructions, 0, b"memo.txt");
        instructions.extend([
            insn(opcode::PUSH_I16, 0, 0), // ptr
            insn(opcode::PUSH_I16, 0, 8), // len
            insn(opcode::PUSH_I16, 0, 1), // mode = write
            insn(opcode::HOST_CALL, host_call::FILE_OPEN, 0),
            insn(opcode::DROP, 0, 0), // drop open status
            insn(opcode::DROP, 0, 0), // drop handle (left open in the host)
            insn(opcode::HOST_CALL, host_call::YIELD_FRAME, 0),
        ]);
        kbc_with_heap(&instructions, 256u32)
    }

    fn emit_bytes(instructions: &mut Vec<[u8; 4]>, offset: u16, bytes: &[u8]) {
        for (index, byte) in bytes.iter().enumerate() {
            instructions.push(insn(
                koto_core::runtime::opcode::PUSH_I16,
                0,
                offset + index as u16,
            ));
            instructions.push(insn(
                koto_core::runtime::opcode::PUSH_I16,
                0,
                u16::from(*byte),
            ));
            instructions.push(insn(koto_core::runtime::opcode::STORE8, 0, 0));
        }
    }

    fn kbc_with_heap(instructions: &[[u8; 4]], heap_bytes: u32) -> Vec<u8> {
        let bytecode_size = koto_core::KBC_HEADER_SIZE + instructions.len() * 4;
        let mut bytes = vec![0u8; bytecode_size];
        bytes[0..4].copy_from_slice(&koto_core::KBC_MAGIC);
        bytes[4..6].copy_from_slice(&koto_core::KBC_VERSION_MAJOR.to_le_bytes());
        bytes[6..8].copy_from_slice(&koto_core::KBC_VERSION_MINOR.to_le_bytes());
        bytes[8..12].copy_from_slice(&(koto_core::KBC_HEADER_SIZE as u32).to_le_bytes());
        bytes[16..20].copy_from_slice(&(bytecode_size as u32).to_le_bytes());
        bytes[20..24].copy_from_slice(&(koto_core::KBC_HEADER_SIZE as u32).to_le_bytes());
        bytes[24..28].copy_from_slice(&((instructions.len() * 4) as u32).to_le_bytes());
        bytes[40..42].copy_from_slice(&(SIM_VM_STACK_SLOTS as u16).to_le_bytes());
        bytes[42..44].copy_from_slice(&4u16.to_le_bytes());
        bytes[44..48].copy_from_slice(&heap_bytes.to_le_bytes());
        bytes[48..50].copy_from_slice(&koto_core::HOST_ABI_MAJOR.to_le_bytes());
        bytes[50..52].copy_from_slice(&koto_core::HOST_ABI_MINOR.to_le_bytes());
        for (index, word) in instructions.iter().enumerate() {
            let offset = koto_core::KBC_HEADER_SIZE + index * 4;
            bytes[offset..offset + 4].copy_from_slice(word);
        }
        bytes
    }

    fn insn(op: u8, operand: u8, immediate: u16) -> [u8; 4] {
        let imm = immediate.to_le_bytes();
        [imm[0], imm[1], operand, op]
    }
}
