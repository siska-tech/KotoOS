use std::fmt;
use std::path::{Path, PathBuf};

use koto_core::{ManifestFields, PackageManifest, KPA_MANIFEST_FORMAT, KPA_MANIFEST_VERSION};
use serde::{Deserialize, Serialize};

const RUNTIME_BYTECODE: &str = "kotoruntime-bytecode";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScaffoldOptions {
    pub root: PathBuf,
    pub app_id: String,
    pub name: String,
    pub app_dir: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScaffoldResult {
    pub app_id: String,
    pub source: PathBuf,
    /// Starter include (KOTO-0183): the root source pulls this in via
    /// `include "helpers.koto";`, so every scaffolded app exercises the
    /// multi-file compile path end-to-end.
    pub helpers: PathBuf,
    /// The single per-app descriptor (`apps/<dir>/app.json`, KOTO-0195).
    pub descriptor: PathBuf,
    pub icon: PathBuf,
    pub scenario: PathBuf,
}

#[derive(Debug)]
pub enum ScaffoldError {
    InvalidManifestFields,
    InvalidAppDirectory(PathBuf),
    AppAlreadyRegistered(String),
    PathExists(PathBuf),
    DescriptorScan {
        path: PathBuf,
        source: std::io::Error,
    },
    DescriptorJson {
        path: PathBuf,
        source: serde_json::Error,
    },
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl fmt::Display for ScaffoldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidManifestFields => {
                write!(
                    f,
                    "app ID, name, runtime, entry, or icon failed manifest validation"
                )
            }
            Self::InvalidAppDirectory(path) => {
                write!(
                    f,
                    "app directory must be a relative path: {}",
                    path.display()
                )
            }
            Self::AppAlreadyRegistered(app_id) => {
                write!(f, "app ID is already registered: {app_id}")
            }
            Self::PathExists(path) => write!(f, "refusing to overwrite {}", path.display()),
            Self::DescriptorScan { path, source } => {
                write!(
                    f,
                    "failed to scan descriptors under {}: {source}",
                    path.display()
                )
            }
            Self::DescriptorJson { path, source } => {
                write!(f, "invalid descriptor JSON {}: {source}", path.display())
            }
            Self::Io { path, source } => write!(f, "failed to write {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for ScaffoldError {}

pub fn scaffold_app(options: ScaffoldOptions) -> Result<ScaffoldResult, ScaffoldError> {
    let root = options.root;
    let app_id = options.app_id;
    let name = options.name;
    let slug = app_slug(&app_id);
    let app_dir = options
        .app_dir
        .unwrap_or_else(|| PathBuf::from("apps").join(&slug));
    if app_dir.is_absolute() || app_dir.components().any(is_parent_dir) {
        return Err(ScaffoldError::InvalidAppDirectory(app_dir));
    }

    let source = app_dir.join("src/main.koto");
    let helpers = app_dir.join("src/helpers.koto");
    let scenario = app_dir.join("scenarios/smoke.txt");
    let icon = app_dir.join("icon.kicon");
    let descriptor = app_dir.join("app.json");

    // Validate app_id / name / runtime / entry / icon through the same manifest
    // rules the runtime and packer use; the build derives these staged paths.
    PackageManifest::new(ManifestFields {
        format: KPA_MANIFEST_FORMAT,
        version: KPA_MANIFEST_VERSION,
        app_id: &app_id,
        name: &name,
        runtime: RUNTIME_BYTECODE,
        entry: &format!("bytecode/{slug}.kbc"),
        icon: Some(&format!("icons/{slug}.kicon")),
        shell_icon: None,
        fs_permission: Some("sandbox"),
        network_permission: Some(false),
        sram_work_bytes: None,
        psram_cache_bytes: None,
        description: None,
        category: None,
    })
    .map_err(|_| ScaffoldError::InvalidManifestFields)?;

    // Duplicate-app_id detection now lives with the scanner (KOTO-0195): no
    // shared registry file to rewrite, so scaffolding one app never touches
    // another's descriptor.
    if existing_app_ids(&root)?.iter().any(|id| id == &app_id) {
        return Err(ScaffoldError::AppAlreadyRegistered(app_id));
    }

    let files = [
        source.as_path(),
        helpers.as_path(),
        scenario.as_path(),
        icon.as_path(),
        descriptor.as_path(),
    ];
    for path in files {
        if root.join(path).exists() {
            return Err(ScaffoldError::PathExists(path.to_path_buf()));
        }
    }

    write_file(&root, &source, starter_source(&name).as_bytes())?;
    write_file(&root, &helpers, starter_helpers().as_bytes())?;
    write_file(&root, &scenario, starter_scenario().as_bytes())?;
    write_file(&root, &icon, starter_icon(&slug).as_bytes())?;
    write_file(
        &root,
        &descriptor,
        starter_descriptor(&app_id, &name, &slug)?.as_bytes(),
    )?;

    Ok(ScaffoldResult {
        app_id,
        source,
        helpers,
        descriptor,
        icon,
        scenario,
    })
}

fn app_slug(app_id: &str) -> String {
    app_id.rsplit('.').next().unwrap_or("app").replace('-', "_")
}

fn is_parent_dir(component: std::path::Component<'_>) -> bool {
    matches!(component, std::path::Component::ParentDir)
}

/// Collect the `app_id` of every `apps/**/app.json` descriptor (empty when the
/// apps tree does not exist yet).
fn existing_app_ids(root: &Path) -> Result<Vec<String>, ScaffoldError> {
    fn walk(dir: &Path, ids: &mut Vec<String>) -> Result<(), ScaffoldError> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(source) => {
                return Err(ScaffoldError::DescriptorScan {
                    path: dir.to_path_buf(),
                    source,
                })
            }
        };
        for entry in entries {
            let entry = entry.map_err(|source| ScaffoldError::DescriptorScan {
                path: dir.to_path_buf(),
                source,
            })?;
            let path = entry.path();
            if path.is_dir() {
                walk(&path, ids)?;
            } else if path.file_name().is_some_and(|name| name == "app.json") {
                let text = std::fs::read_to_string(&path).map_err(|source| {
                    ScaffoldError::DescriptorScan {
                        path: path.clone(),
                        source,
                    }
                })?;
                let descriptor: DescriptorId = serde_json::from_str(&text).map_err(|source| {
                    ScaffoldError::DescriptorJson {
                        path: path.clone(),
                        source,
                    }
                })?;
                ids.push(descriptor.app_id);
            }
        }
        Ok(())
    }
    let mut ids = Vec::new();
    walk(&root.join("apps"), &mut ids)?;
    Ok(ids)
}

fn write_file(root: &Path, relative: &Path, bytes: &[u8]) -> Result<(), ScaffoldError> {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ScaffoldError::Io {
            path: relative.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(&path, bytes).map_err(|source| ScaffoldError::Io {
        path: relative.to_path_buf(),
        source,
    })
}

fn starter_source(name: &str) -> String {
    format!(
        r#"// Generated KotoSDK starter app.
// `include` splices helpers.koto here at compile time (KOTO-0183). Remember:
// every call is still inlined into main, so a helper's body costs bytecode at
// each call site no matter which file defines it.
include "helpers.koto";

fn main() {{
    let ticks = 0;
    loop {{
        let intent = text_intent();
        if (intent & INTENT_EXIT) != 0 {{
            exit(0);
        }}

        draw_rect(0, 0, 320, 320, 0);
        draw_text(16, 24, "{name}", 18);
        draw_text(16, 44, "new Koto app is running", 20);
        draw_text(16, 64, "press F10 to exit", 22);
        ticks = tick_wrap(ticks);
        yield_frame();
    }}
}}
"#
    )
}

fn starter_helpers() -> &'static str {
    r#"// Helpers included by main.koto (KOTO-0183 source-file splitting).

fn tick_wrap(ticks: int) -> int {
    if ticks >= 32767 { return 0; }
    return ticks + 1;
}
"#
}

fn starter_scenario() -> &'static str {
    "# One input frame per line. This smoke scenario lets the app draw, then exits.\nframe\nexit\n"
}

fn starter_icon(slug: &str) -> String {
    let mut text = String::from("KICON1\n");
    let hash = slug.bytes().fold(0u8, |acc, byte| acc.wrapping_add(byte));
    for y in 0..40 {
        for x in 0..40 {
            let border = x == 0 || y == 0 || x == 39 || y == 39;
            let diagonal = ((x + y + usize::from(hash % 7)) % 11) == 0;
            text.push(if border || diagonal { '#' } else { '.' });
        }
        text.push('\n');
    }
    text
}

fn starter_descriptor(app_id: &str, name: &str, slug: &str) -> Result<String, ScaffoldError> {
    let descriptor = AppDescriptor {
        app_id: app_id.to_string(),
        kind: "koto".to_string(),
        package: slug.to_string(),
        name: name.to_string(),
        description: String::new(),
        category: "アプリ".to_string(),
        runtime: RUNTIME_BYTECODE.to_string(),
        source: "src/main.koto".to_string(),
        icon: "icon.kicon".to_string(),
        memory: Memory {
            sram_work_bytes: 16384,
            psram_cache_bytes: 32768,
        },
        permissions: Permissions {
            fs: "sandbox".to_string(),
            network: false,
        },
    };
    serde_json::to_string_pretty(&descriptor)
        .map(|json| json + "\n")
        .map_err(|source| ScaffoldError::DescriptorJson {
            path: PathBuf::from("app.json"),
            source,
        })
}

/// Minimal shape used only to read an existing descriptor's `app_id`.
#[derive(Clone, Debug, Deserialize)]
struct DescriptorId {
    app_id: String,
}

/// The per-app `app.json` descriptor written by the scaffold: build recipe plus
/// the absorbed package fields (KOTO-0195). Serialize order is the field order.
#[derive(Clone, Debug, Serialize)]
struct AppDescriptor {
    app_id: String,
    kind: String,
    package: String,
    name: String,
    description: String,
    category: String,
    runtime: String,
    source: String,
    icon: String,
    memory: Memory,
    permissions: Permissions,
}

#[derive(Clone, Debug, Serialize)]
struct Memory {
    sram_work_bytes: u32,
    psram_cache_bytes: u32,
}

#[derive(Clone, Debug, Serialize)]
struct Permissions {
    fs: String,
    network: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("koto-app-scaffold-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("apps")).unwrap();
        root
    }

    fn write_descriptor(root: &Path, dir: &str, app_id: &str) {
        let path = root.join("apps").join(dir).join("app.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            format!("{{\"app_id\":\"{app_id}\",\"kind\":\"koto\",\"package\":\"{dir}\",\"name\":\"X\",\"source\":\"src/main.koto\"}}\n"),
        )
        .unwrap();
    }

    #[test]
    fn creates_buildable_app_layout_and_descriptor() {
        let root = temp_root("creates");

        let result = scaffold_app(ScaffoldOptions {
            root: root.clone(),
            app_id: "dev.koto.test.todo-list".to_string(),
            name: "Todo List".to_string(),
            app_dir: None,
        })
        .unwrap();

        assert_eq!(result.source, PathBuf::from("apps/todo_list/src/main.koto"));
        assert_eq!(
            result.helpers,
            PathBuf::from("apps/todo_list/src/helpers.koto")
        );
        assert_eq!(result.descriptor, PathBuf::from("apps/todo_list/app.json"));
        assert_eq!(result.icon, PathBuf::from("apps/todo_list/icon.kicon"));
        assert!(root.join(&result.source).exists());
        assert!(root.join(&result.helpers).exists());
        assert!(root.join(&result.scenario).exists());
        assert!(root.join(&result.icon).exists());

        let source_text = std::fs::read_to_string(root.join(&result.source)).unwrap();
        assert!(source_text.contains("include \"helpers.koto\";"));

        let descriptor_text = std::fs::read_to_string(root.join(&result.descriptor)).unwrap();
        assert!(descriptor_text.contains("\"app_id\": \"dev.koto.test.todo-list\""));
        assert!(descriptor_text.contains("\"package\": \"todo_list\""));
        assert!(descriptor_text.contains("\"source\": \"src/main.koto\""));
        assert!(descriptor_text.contains("\"icon\": \"icon.kicon\""));

        // No shared registry is created or touched.
        assert!(!root.join("apps/apps.json").exists());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn scaffolding_leaves_other_app_descriptors_untouched() {
        let root = temp_root("isolated");
        write_descriptor(&root, "existing", "dev.koto.test.existing");
        let before = std::fs::read(root.join("apps/existing/app.json")).unwrap();

        scaffold_app(ScaffoldOptions {
            root: root.clone(),
            app_id: "dev.koto.test.fresh".to_string(),
            name: "Fresh".to_string(),
            app_dir: None,
        })
        .unwrap();

        let after = std::fs::read(root.join("apps/existing/app.json")).unwrap();
        assert_eq!(
            before, after,
            "an existing descriptor must not be rewritten"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_invalid_app_id_with_core_manifest_validation() {
        let root = temp_root("invalid");
        let error = scaffold_app(ScaffoldOptions {
            root: root.clone(),
            app_id: "Dev.Koto.Bad".to_string(),
            name: "Bad".to_string(),
            app_dir: None,
        })
        .unwrap_err();

        assert!(matches!(error, ScaffoldError::InvalidManifestFields));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn refuses_duplicate_app_ids() {
        let root = temp_root("duplicate");
        write_descriptor(&root, "dup", "dev.koto.test.dup");

        let error = scaffold_app(ScaffoldOptions {
            root: root.clone(),
            app_id: "dev.koto.test.dup".to_string(),
            name: "Duplicate".to_string(),
            app_dir: None,
        })
        .unwrap_err();

        assert!(matches!(error, ScaffoldError::AppAlreadyRegistered(_)));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn generated_app_compiles_and_launches_in_sim() {
        let root = temp_root("launches");
        let result = scaffold_app(ScaffoldOptions {
            root: root.clone(),
            app_id: "dev.koto.test.launch".to_string(),
            name: "Launch Test".to_string(),
            app_dir: None,
        })
        .unwrap();

        // Compile via the absolute source path so the `include "helpers.koto";`
        // in the starter resolves against the temp root, not the test's cwd.
        let source_abs = root.join(&result.source);
        let source = std::fs::read_to_string(&source_abs).unwrap();
        let bytecode = koto_compiler::compile(source_abs.to_str().unwrap(), &source).unwrap();
        let output = root.join("package_inputs/bytecode/launch.kbc");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        std::fs::write(output, bytecode).unwrap();

        // The manifest is generated from app.json by the build (harness), not
        // by the scaffold. Synthesize a minimal one here to prove the
        // scaffolded *source* compiles, packs, and launches.
        let manifest = br#"{
  "format": "kpa-manifest",
  "version": 1,
  "app_id": "dev.koto.test.launch",
  "name": "Launch Test",
  "entry": "bytecode/launch.kbc",
  "runtime": "kotoruntime-bytecode",
  "assets": [{ "path": "bytecode/launch.kbc", "type": "bytecode", "sequential": true }],
  "permissions": { "fs": "sandbox", "network": false }
}
"#;
        let package = kpa_packer::pack_manifest(
            manifest,
            kpa_packer::PackOptions {
                assets_root: root.join("package_inputs"),
            },
        )
        .unwrap();
        std::fs::create_dir_all(root.join("sdcard_mock/apps")).unwrap();
        std::fs::write(root.join("sdcard_mock/apps/launch.kpa"), package.bytes()).unwrap();

        let scenario = std::fs::read_to_string(root.join(&result.scenario)).unwrap();
        let inputs = koto_sim::parse_app_script(&scenario).unwrap();
        let report =
            koto_sim::run_app_scenario(root.join("sdcard_mock"), &result.app_id, &inputs).unwrap();

        assert_eq!(report.result, koto_core::VmRunResult::Exited(0));
        assert_eq!(report.app_id, "dev.koto.test.launch");
        let _ = std::fs::remove_dir_all(root);
    }
}
