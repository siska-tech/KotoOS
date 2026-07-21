//! KOTO-0236 compile-time asset sizes.
//!
//! `asset_len("path", ...)` folds to the byte size of a packaged asset (the
//! maximum size with several arguments). KOTO-0237 text helpers inspect the
//! same verbatim asset bytes to fold line counts and line-range payload sizes.
//! Their path namespace is *exactly* the `asset_load` namespace: the package
//! asset paths declared as `output` in the app manifest's `assets` block,
//! which is the set the runtime host permits.
//! The compiler therefore resolves paths against the manifest — discovered as
//! the nearest `app.json` above the root source file — never against the
//! including source file's directory, so any literal `asset_len` accepts is a
//! valid `asset_load` argument and the folded size is the size of the bytes
//! that ship in the package.
//!
//! Resolution is injected (like [`crate::IncludeLoader`]) so unit tests and
//! editors fold `asset_len` without an on-disk `app.json`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Compile-time lookup for manifest-declared verbatim assets. `asset_len`
/// stays metadata-only; text helpers request cached bytes. `Err` carries the
/// complete human-readable reason; the parser prefixes the offending call.
pub trait AssetResolver {
    fn asset_len(&mut self, path: &str) -> Result<u64, String>;
    fn asset_bytes(&mut self, path: &str) -> Result<Vec<u8>, String>;
}

/// The production resolver: lazy nearest-`app.json` discovery upward from the
/// root source file. Programs that never call `asset_len` never touch the
/// filesystem, so in-memory compilations (tests, editors) stay hermetic.
pub struct ManifestAssets {
    root_file: PathBuf,
    table: Option<Result<ManifestTable, String>>,
    bytes: BTreeMap<String, Result<Vec<u8>, String>>,
}

struct ManifestTable {
    /// Display path of the discovered manifest, for diagnostics.
    manifest: String,
    /// Verbatim `assets` entries: package `output` path -> on-disk source.
    verbatim: BTreeMap<String, PathBuf>,
    /// Outputs of pipeline-transformed blocks (`images`, `audio`): declared
    /// package paths whose packaged size the compiler cannot see (V1).
    transformed: Vec<String>,
}

impl ManifestAssets {
    pub fn for_root(root_file: &str) -> Self {
        Self {
            root_file: PathBuf::from(root_file),
            table: None,
            bytes: BTreeMap::new(),
        }
    }

    fn table(&mut self) -> &Result<ManifestTable, String> {
        if self.table.is_none() {
            self.table = Some(discover(&self.root_file));
        }
        self.table.as_ref().unwrap()
    }
}

impl AssetResolver for ManifestAssets {
    fn asset_len(&mut self, path: &str) -> Result<u64, String> {
        let table = match self.table() {
            Ok(table) => table,
            Err(message) => return Err(message.clone()),
        };
        if let Some(source) = table.verbatim.get(path) {
            let metadata = std::fs::metadata(source).map_err(|error| {
                format!(
                    "cannot read asset source {} for \"{path}\": {error}",
                    source.display()
                )
            })?;
            return Ok(metadata.len());
        }
        if table.transformed.iter().any(|output| output == path) {
            return Err(format!(
                "\"{path}\" is a pipeline-transformed package asset; `asset_len` \
                 folds verbatim `assets` entries only"
            ));
        }
        Err(format!(
            "\"{path}\" is not declared as an `assets` output in {}",
            table.manifest
        ))
    }

    fn asset_bytes(&mut self, path: &str) -> Result<Vec<u8>, String> {
        if let Some(result) = self.bytes.get(path) {
            return result.clone();
        }
        let source = match self.table() {
            Ok(table) => match table.verbatim.get(path) {
                Some(source) => source.clone(),
                None if table.transformed.iter().any(|output| output == path) => {
                    let error = format!(
                        "\"{path}\" is a pipeline-transformed package asset; text asset helpers \
                         inspect verbatim `assets` entries only"
                    );
                    self.bytes.insert(path.to_string(), Err(error.clone()));
                    return Err(error);
                }
                None => {
                    let error = format!(
                        "\"{path}\" is not declared as an `assets` output in {}",
                        table.manifest
                    );
                    self.bytes.insert(path.to_string(), Err(error.clone()));
                    return Err(error);
                }
            },
            Err(message) => return Err(message.clone()),
        };
        let result = std::fs::read(&source).map_err(|error| {
            format!(
                "cannot read asset source {} for \"{path}\": {error}",
                source.display()
            )
        });
        self.bytes.insert(path.to_string(), result.clone());
        result
    }
}

/// Walk up from the root source file to the nearest `app.json` and index its
/// declared package outputs. The manifest, not the filesystem, defines the
/// namespace — an undeclared file next to a declared one stays undeclared.
fn discover(root_file: &Path) -> Result<ManifestTable, String> {
    let mut dir = root_file.parent();
    while let Some(current) = dir {
        let manifest = current.join("app.json");
        if manifest.is_file() {
            return parse_manifest(&manifest);
        }
        dir = current.parent();
    }
    Err(format!(
        "no app.json manifest found above {} to define the package asset namespace",
        root_file.display()
    ))
}

fn parse_manifest(path: &Path) -> Result<ManifestTable, String> {
    let display = path.display().to_string().replace('\\', "/");
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("cannot read {display}: {error}"))?;
    let json: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| format!("cannot parse {display}: {error}"))?;
    let app_dir = path.parent().unwrap_or_else(|| Path::new(""));

    let mut verbatim = BTreeMap::new();
    for asset in json["assets"].as_array().into_iter().flatten() {
        // Mirror `harness/build_apps.py` `register_data_assets`: `output`
        // defaults to `source`, and both are app-relative `/` paths.
        let Some(source) = asset["source"].as_str() else {
            continue;
        };
        let output = asset["output"].as_str().unwrap_or(source);
        verbatim.insert(output.to_string(), app_dir.join(source));
    }
    let mut transformed = Vec::new();
    for image in json["images"].as_array().into_iter().flatten() {
        for key in ["output", "tilemap_output"] {
            if let Some(output) = image[key].as_str() {
                transformed.push(output.to_string());
            }
        }
    }
    for audio in json["audio"].as_array().into_iter().flatten() {
        if let Some(output) = audio["output"].as_str() {
            transformed.push(output.to_string());
        }
    }
    Ok(ManifestTable {
        manifest: display,
        verbatim,
        transformed,
    })
}
