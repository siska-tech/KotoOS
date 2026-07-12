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
    pub manifest: PathBuf,
    pub icon: PathBuf,
    pub scenario: PathBuf,
    pub registry: PathBuf,
}

#[derive(Debug)]
pub enum ScaffoldError {
    InvalidManifestFields,
    InvalidAppDirectory(PathBuf),
    AppAlreadyRegistered(String),
    PathExists(PathBuf),
    RegistryRead {
        path: PathBuf,
        source: std::io::Error,
    },
    RegistryJson {
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
            Self::RegistryRead { path, source } => {
                write!(f, "failed to read registry {}: {source}", path.display())
            }
            Self::RegistryJson { path, source } => {
                write!(f, "invalid registry JSON {}: {source}", path.display())
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
    let output = PathBuf::from("sdcard_mock")
        .join("bytecode")
        .join(format!("{slug}.kbc"));
    let manifest = PathBuf::from("sdcard_mock")
        .join("apps")
        .join(format!("{slug}.kpa.json"));
    let icon = PathBuf::from("sdcard_mock")
        .join("icons")
        .join(format!("{slug}.kicon"));
    let entry = relative_to_sdcard(&output);
    let icon_entry = relative_to_sdcard(&icon);

    PackageManifest::new(ManifestFields {
        format: KPA_MANIFEST_FORMAT,
        version: KPA_MANIFEST_VERSION,
        app_id: &app_id,
        name: &name,
        runtime: RUNTIME_BYTECODE,
        entry: &entry,
        icon: Some(&icon_entry),
        shell_icon: None,
        fs_permission: Some("sandbox"),
        network_permission: Some(false),
        sram_work_bytes: None,
        psram_cache_bytes: None,
        description: None,
        category: None,
    })
    .map_err(|_| ScaffoldError::InvalidManifestFields)?;

    let registry_path = PathBuf::from("apps/apps.json");
    let registry_abs = root.join(&registry_path);
    let registry_text =
        std::fs::read_to_string(&registry_abs).map_err(|source| ScaffoldError::RegistryRead {
            path: registry_path.clone(),
            source,
        })?;
    let mut registry: AppRegistry =
        serde_json::from_str(&registry_text).map_err(|source| ScaffoldError::RegistryJson {
            path: registry_path.clone(),
            source,
        })?;
    if registry.apps.iter().any(|app| app.app_id == app_id) {
        return Err(ScaffoldError::AppAlreadyRegistered(app_id));
    }

    let files = [
        source.as_path(),
        helpers.as_path(),
        scenario.as_path(),
        manifest.as_path(),
        icon.as_path(),
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
        &manifest,
        starter_manifest(&app_id, &name, &entry, &icon_entry)?.as_bytes(),
    )?;

    registry.apps.push(RegistryApp {
        app_id: app_id.clone(),
        kind: "koto".to_string(),
        source: source.to_string_lossy().replace('\\', "/"),
        output: output.to_string_lossy().replace('\\', "/"),
        manifest: manifest.to_string_lossy().replace('\\', "/"),
    });
    let registry_json =
        serde_json::to_string_pretty(&registry).map_err(|source| ScaffoldError::RegistryJson {
            path: registry_path.clone(),
            source,
        })? + "\n";
    write_file(&root, &registry_path, registry_json.as_bytes())?;

    Ok(ScaffoldResult {
        app_id,
        source,
        helpers,
        manifest,
        icon,
        scenario,
        registry: registry_path,
    })
}

fn app_slug(app_id: &str) -> String {
    app_id.rsplit('.').next().unwrap_or("app").replace('-', "_")
}

fn is_parent_dir(component: std::path::Component<'_>) -> bool {
    matches!(component, std::path::Component::ParentDir)
}

fn relative_to_sdcard(path: &Path) -> String {
    path.strip_prefix("sdcard_mock")
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
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

fn starter_manifest(
    app_id: &str,
    name: &str,
    entry: &str,
    icon: &str,
) -> Result<String, ScaffoldError> {
    let manifest = SourceManifest {
        format: KPA_MANIFEST_FORMAT.to_string(),
        version: KPA_MANIFEST_VERSION,
        app_id: app_id.to_string(),
        name: name.to_string(),
        entry: entry.to_string(),
        runtime: RUNTIME_BYTECODE.to_string(),
        icon: icon.to_string(),
        memory: Memory {
            sram_work_bytes: 16384,
            psram_cache_bytes: 32768,
        },
        assets: vec![
            ManifestAsset {
                path: entry.to_string(),
                asset_type: "bytecode".to_string(),
                sequential: true,
            },
            ManifestAsset {
                path: icon.to_string(),
                asset_type: "image".to_string(),
                sequential: false,
            },
        ],
        permissions: Permissions {
            fs: "sandbox".to_string(),
            network: false,
        },
    };
    serde_json::to_string_pretty(&manifest)
        .map(|json| json + "\n")
        .map_err(|source| ScaffoldError::RegistryJson {
            path: PathBuf::from("manifest"),
            source,
        })
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct AppRegistry {
    comment: String,
    apps: Vec<RegistryApp>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RegistryApp {
    app_id: String,
    kind: String,
    source: String,
    output: String,
    manifest: String,
}

#[derive(Clone, Debug, Serialize)]
struct SourceManifest {
    format: String,
    version: u32,
    app_id: String,
    name: String,
    entry: String,
    runtime: String,
    icon: String,
    memory: Memory,
    assets: Vec<ManifestAsset>,
    permissions: Permissions,
}

#[derive(Clone, Debug, Serialize)]
struct Memory {
    sram_work_bytes: u32,
    psram_cache_bytes: u32,
}

#[derive(Clone, Debug, Serialize)]
struct ManifestAsset {
    path: String,
    #[serde(rename = "type")]
    asset_type: String,
    sequential: bool,
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
        std::fs::write(
            root.join("apps/apps.json"),
            "{\n  \"comment\": \"test registry\",\n  \"apps\": []\n}\n",
        )
        .unwrap();
        root
    }

    #[test]
    fn creates_buildable_app_layout_and_registry_entry() {
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
        assert!(root.join(&result.source).exists());
        assert!(root.join(&result.helpers).exists());
        assert!(root.join(&result.scenario).exists());
        assert!(root.join(&result.icon).exists());

        let source_text = std::fs::read_to_string(root.join(&result.source)).unwrap();
        assert!(source_text.contains("include \"helpers.koto\";"));

        let manifest_text = std::fs::read_to_string(root.join(&result.manifest)).unwrap();
        assert!(manifest_text.contains("\"app_id\": \"dev.koto.test.todo-list\""));
        assert!(manifest_text.contains("\"entry\": \"bytecode/todo_list.kbc\""));
        assert!(manifest_text.contains("\"icon\": \"icons/todo_list.kicon\""));

        let registry_text = std::fs::read_to_string(root.join("apps/apps.json")).unwrap();
        assert!(registry_text.contains("\"source\": \"apps/todo_list/src/main.koto\""));
        assert!(registry_text.contains("\"output\": \"sdcard_mock/bytecode/todo_list.kbc\""));

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
    fn refuses_duplicate_registry_ids() {
        let root = temp_root("duplicate");
        std::fs::write(
            root.join("apps/apps.json"),
            "{\n  \"comment\": \"test registry\",\n  \"apps\": [{\"app_id\":\"dev.koto.test.dup\",\"kind\":\"koto\",\"source\":\"apps/dup/src/main.koto\",\"output\":\"sdcard_mock/bytecode/dup.kbc\",\"manifest\":\"sdcard_mock/apps/dup.kpa.json\"}]\n}\n",
        )
        .unwrap();

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
        let output = root.join("sdcard_mock/bytecode/launch.kbc");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        std::fs::write(output, bytecode).unwrap();

        let scenario = std::fs::read_to_string(root.join(&result.scenario)).unwrap();
        let inputs = koto_sim::parse_app_script(&scenario).unwrap();
        let report =
            koto_sim::run_app_scenario(root.join("sdcard_mock"), &result.app_id, &inputs).unwrap();

        assert_eq!(report.result, koto_core::VmRunResult::Exited(0));
        assert_eq!(report.app_id, "dev.koto.test.launch");
        let _ = std::fs::remove_dir_all(root);
    }
}
