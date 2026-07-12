//! KOTO-0183 textual include expansion.
//!
//! Before lexing, `include "relative/path.koto";` directive lines are replaced
//! by the named file's contents (paths resolve against the *including* file's
//! directory). The result compiles exactly like a single-file program, so a
//! split is provably free: the token stream — and therefore the emitted
//! bytecode — is identical to the unsplit source. A [`SourceMap`] records the
//! originating file and line for every expanded line, and the crate boundary
//! remaps every diagnostic through it, so errors report real `file:line:col`
//! across include boundaries. Design note: `docs/spec/KOTO_LANGUAGE_INCLUDE.md`.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use crate::CompileError;

/// Nested-include depth cap; deeper is almost certainly a mistake at the
/// current app scale (the proving case uses depth 1).
const MAX_INCLUDE_DEPTH: usize = 16;

/// File loading is injected so unit tests run hermetically and callers that
/// pass sources with no `include` directives never touch the filesystem.
pub trait IncludeLoader {
    /// Read the file at `path`, returning a human-readable message on failure.
    fn load(&mut self, path: &Path) -> Result<String, String>;
}

/// The production loader: plain filesystem reads.
pub struct FsLoader;

impl IncludeLoader for FsLoader {
    fn load(&mut self, path: &Path) -> Result<String, String> {
        std::fs::read_to_string(path).map_err(|error| format!("{error}"))
    }
}

/// Maps expanded-source lines back to their originating file and line. File 0
/// is always the root source, under the exact name the caller passed (which
/// also keeps `.debug_file` and single-file error text unchanged).
#[derive(Debug)]
pub struct SourceMap {
    files: Vec<String>,
    /// One entry per expanded line (1-based line N is `lines[N - 1]`):
    /// `(files index, 1-based original line)`.
    lines: Vec<(usize, usize)>,
}

impl SourceMap {
    /// Resolve an expanded 1-based line to `(file, original line)`. Lines
    /// outside the map (e.g. the 0 of internal errors) fall back to the root.
    pub fn resolve(&self, expanded_line: usize) -> (&str, usize) {
        match expanded_line
            .checked_sub(1)
            .and_then(|index| self.lines.get(index))
        {
            Some(&(file, line)) => (&self.files[file], line),
            None => (&self.files[0], expanded_line),
        }
    }
}

/// Expand all `include` directives in `root_source`. Returns the expanded
/// source and the [`SourceMap`] to attribute its lines.
pub fn expand(
    root_file: &str,
    root_source: &str,
    loader: &mut dyn IncludeLoader,
) -> Result<(String, SourceMap), CompileError> {
    let mut expansion = Expansion {
        loader,
        out: String::new(),
        files: vec![root_file.to_string()],
        lines: Vec::new(),
        included: HashSet::new(),
    };
    expansion.included.insert(normalize(Path::new(root_file)));
    expansion.file(0, root_source, 1)?;
    Ok((
        expansion.out,
        SourceMap {
            files: expansion.files,
            lines: expansion.lines,
        },
    ))
}

struct Expansion<'a> {
    loader: &'a mut dyn IncludeLoader,
    out: String,
    files: Vec<String>,
    lines: Vec<(usize, usize)>,
    /// Normalized paths already spliced in (the root included). Each file may
    /// appear once per program; a re-include or cycle errors at its site.
    included: HashSet<PathBuf>,
}

impl Expansion<'_> {
    /// Splice `source` (the contents of `self.files[file_index]`) into the
    /// expansion, recursing into its include directives.
    fn file(&mut self, file_index: usize, source: &str, depth: usize) -> Result<(), CompileError> {
        let pieces: Vec<&str> = source.split('\n').collect();
        for (index, raw_line) in pieces.iter().enumerate() {
            let line_no = index + 1;
            // `split('\n')` yields one trailing empty piece when the source
            // ends with a newline; skip it so the expansion doesn't grow a
            // phantom line per included file.
            if line_no == pieces.len() && raw_line.is_empty() {
                break;
            }
            let error = |col: usize, message: String| CompileError {
                file: self.files[file_index].clone(),
                line: line_no,
                col,
                message,
            };
            match parse_include_line(raw_line) {
                IncludeLine::NotADirective => {
                    self.out.push_str(raw_line);
                    self.out.push('\n');
                    self.lines.push((file_index, line_no));
                }
                IncludeLine::Malformed { col, message } => return Err(error(col, message)),
                IncludeLine::Include { col, path } => {
                    if depth >= MAX_INCLUDE_DEPTH {
                        return Err(error(
                            col,
                            format!("includes nested deeper than {MAX_INCLUDE_DEPTH} levels"),
                        ));
                    }
                    if path.is_empty() {
                        return Err(error(col, "include path is empty".to_string()));
                    }
                    if path.contains('\\') {
                        return Err(error(
                            col,
                            "include paths must use `/` separators".to_string(),
                        ));
                    }
                    if Path::new(&path).is_absolute() || path.starts_with('/') {
                        return Err(error(
                            col,
                            format!("include path must be relative, got \"{path}\""),
                        ));
                    }
                    let parent = Path::new(&self.files[file_index])
                        .parent()
                        .unwrap_or_else(|| Path::new(""));
                    let resolved = normalize(&parent.join(&path));
                    if !self.included.insert(resolved.clone()) {
                        return Err(error(
                            col,
                            format!(
                                "\"{path}\" is already included; each file may be \
                                 included once per program"
                            ),
                        ));
                    }
                    let source = self.loader.load(&resolved).map_err(|message| {
                        error(col, format!("cannot read include \"{path}\": {message}"))
                    })?;
                    let display = resolved.to_string_lossy().replace('\\', "/");
                    self.files.push(display);
                    let included_index = self.files.len() - 1;
                    self.file(included_index, &source, depth + 1)?;
                }
            }
        }
        Ok(())
    }
}

enum IncludeLine {
    NotADirective,
    Include { col: usize, path: String },
    Malformed { col: usize, message: String },
}

/// Classify one raw source line. A directive is a whole line of the form
/// `include "path";` (plus optional trailing `//` comment). The `include`
/// keyword must be followed by whitespace and then a `"`, so identifiers like
/// `include_x` or calls like `include(x)` are ordinary code; but once a line
/// commits to the directive prefix, malformed syntax is an error rather than
/// silently compiling as an expression.
fn parse_include_line(raw_line: &str) -> IncludeLine {
    let trimmed = raw_line.trim_start();
    let col = raw_line.len() - trimmed.len() + 1;
    let Some(rest) = trimmed.strip_prefix("include") else {
        return IncludeLine::NotADirective;
    };
    if !rest.starts_with(char::is_whitespace) {
        return IncludeLine::NotADirective;
    }
    let rest = rest.trim_start();
    let Some(quoted) = rest.strip_prefix('"') else {
        return IncludeLine::NotADirective;
    };
    let Some(end) = quoted.find('"') else {
        return IncludeLine::Malformed {
            col,
            message: "unterminated include path string".to_string(),
        };
    };
    let path = quoted[..end].to_string();
    let after = quoted[end + 1..].trim_start();
    let Some(after) = after.strip_prefix(';') else {
        return IncludeLine::Malformed {
            col,
            message: "expected `;` after include path".to_string(),
        };
    };
    // `trim` (not `trim_start`) so CRLF sources' trailing `\r` is tolerated.
    let after = after.trim();
    if !after.is_empty() && !after.starts_with("//") {
        return IncludeLine::Malformed {
            col,
            message: "unexpected text after include directive".to_string(),
        };
    }
    IncludeLine::Include { col, path }
}

/// Lexically normalize a path (fold `.` away, resolve `..` against named
/// components). Used for both the duplicate-include key and the display name,
/// so `a/./b.koto` and `a/b.koto` count as the same file.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// In-memory loader keyed by normalized forward-slash paths.
    struct MapLoader(HashMap<&'static str, &'static str>);

    impl IncludeLoader for MapLoader {
        fn load(&mut self, path: &Path) -> Result<String, String> {
            let key = path.to_string_lossy().replace('\\', "/");
            self.0
                .get(key.as_str())
                .map(|source| source.to_string())
                .ok_or_else(|| "no such file".to_string())
        }
    }

    fn expand_with(
        root_file: &str,
        root_source: &str,
        files: &[(&'static str, &'static str)],
    ) -> Result<(String, SourceMap), CompileError> {
        let mut loader = MapLoader(files.iter().copied().collect());
        expand(root_file, root_source, &mut loader)
    }

    #[test]
    fn source_without_includes_is_unchanged() {
        let source = "fn main() {\n    exit(0);\n}\n";
        let (expanded, map) = expand_with("test.koto", source, &[]).expect("expands");
        assert_eq!(expanded, source);
        assert_eq!(map.resolve(2), ("test.koto", 2));
    }

    #[test]
    fn include_splices_and_maps_lines() {
        let root = "const A = 1;\ninclude \"util.koto\";\nfn main() { exit(A + B); }\n";
        let (expanded, map) =
            expand_with("test.koto", root, &[("util.koto", "const B = 2;\n")]).expect("expands");
        assert_eq!(
            expanded,
            "const A = 1;\nconst B = 2;\nfn main() { exit(A + B); }\n"
        );
        assert_eq!(map.resolve(1), ("test.koto", 1));
        assert_eq!(map.resolve(2), ("util.koto", 1));
        assert_eq!(map.resolve(3), ("test.koto", 3));
    }

    #[test]
    fn includes_resolve_relative_to_the_including_file() {
        let root = "include \"sub/a.koto\";\nfn main() { exit(0); }\n";
        let (expanded, map) = expand_with(
            "apps/demo/src/main.koto",
            root,
            &[
                ("apps/demo/src/sub/a.koto", "include \"b.koto\";\n"),
                ("apps/demo/src/sub/b.koto", "const B = 1;\n"),
            ],
        )
        .expect("expands");
        assert_eq!(expanded, "const B = 1;\nfn main() { exit(0); }\n");
        assert_eq!(map.resolve(1), ("apps/demo/src/sub/b.koto", 1));
        assert_eq!(map.resolve(2), ("apps/demo/src/main.koto", 2));
    }

    #[test]
    fn trailing_comment_is_allowed() {
        let root = "include \"util.koto\"; // helpers\n";
        let (expanded, _) =
            expand_with("test.koto", root, &[("util.koto", "const B = 2;\n")]).expect("expands");
        assert_eq!(expanded, "const B = 2;\n");
    }

    #[test]
    fn crlf_sources_expand_and_keep_their_line_endings() {
        let root = "include \"util.koto\";\r\nfn main() { exit(B); }\r\n";
        let (expanded, map) =
            expand_with("test.koto", root, &[("util.koto", "const B = 0;\r\n")]).expect("expands");
        assert_eq!(expanded, "const B = 0;\r\nfn main() { exit(B); }\r\n");
        assert_eq!(map.resolve(1), ("util.koto", 1));
        assert_eq!(map.resolve(2), ("test.koto", 2));
    }

    #[test]
    fn include_like_identifiers_are_ordinary_code() {
        let source = "fn main() { let include_x = 1; exit(include_x); }\n";
        let (expanded, _) = expand_with("test.koto", source, &[]).expect("expands");
        assert_eq!(expanded, source);
    }

    #[test]
    fn duplicate_include_errors_at_second_site() {
        let root = "include \"util.koto\";\ninclude \"util.koto\";\n";
        let error = expand_with("test.koto", root, &[("util.koto", "const B = 2;\n")])
            .expect_err("duplicate include");
        assert_eq!((error.file.as_str(), error.line), ("test.koto", 2));
        assert!(error.message.contains("already included"), "{error}");
    }

    #[test]
    fn include_cycle_errors() {
        let root = "include \"a.koto\";\n";
        let error = expand_with("test.koto", root, &[("a.koto", "include \"test.koto\";\n")])
            .expect_err("cycle");
        assert_eq!((error.file.as_str(), error.line), ("a.koto", 1));
        assert!(error.message.contains("already included"), "{error}");
    }

    #[test]
    fn missing_include_errors_with_site() {
        let error =
            expand_with("test.koto", "include \"nope.koto\";\n", &[]).expect_err("missing include");
        assert_eq!(
            (error.file.as_str(), error.line, error.col),
            ("test.koto", 1, 1)
        );
        assert!(error.message.contains("cannot read include"), "{error}");
    }

    #[test]
    fn malformed_directives_error() {
        for (source, expected) in [
            ("include \"a.koto\"\n", "expected `;`"),
            ("include \"a.koto;\n", "unterminated include path"),
            ("include \"a.koto\"; extra\n", "unexpected text"),
            ("include \"\";\n", "include path is empty"),
            ("include \"/abs/a.koto\";\n", "must be relative"),
            ("include \"a\\\\b.koto\";\n", "must use `/`"),
        ] {
            let error = expand_with("test.koto", source, &[("a.koto", "")]).expect_err(source);
            assert!(error.message.contains(expected), "{source:?}: {error}");
        }
    }

    #[test]
    fn dot_segments_normalize_to_the_same_file() {
        let root = "include \"./util.koto\";\ninclude \"util.koto\";\n";
        let error = expand_with("test.koto", root, &[("util.koto", "const B = 2;\n")])
            .expect_err("normalized duplicate");
        assert!(error.message.contains("already included"), "{error}");
    }

    #[test]
    fn depth_cap_errors() {
        // Root at depth 1 includes a.koto, which includes itself... build a
        // 17-deep chain of distinct files to trip the cap.
        let mut files: Vec<(&'static str, &'static str)> = Vec::new();
        let names: Vec<String> = (0..MAX_INCLUDE_DEPTH + 1)
            .map(|index| format!("f{index}.koto"))
            .collect();
        let sources: Vec<String> = (0..MAX_INCLUDE_DEPTH)
            .map(|index| format!("include \"{}\";\n", names[index + 1]))
            .collect();
        // Leak to satisfy the 'static loader in this test only.
        for index in 0..MAX_INCLUDE_DEPTH {
            files.push((
                Box::leak(names[index].clone().into_boxed_str()),
                Box::leak(sources[index].clone().into_boxed_str()),
            ));
        }
        files.push((
            Box::leak(names[MAX_INCLUDE_DEPTH].clone().into_boxed_str()),
            "const Z = 1;\n",
        ));
        let root = "include \"f0.koto\";\n";
        let error = expand_with("test.koto", root, &files).expect_err("depth cap");
        assert!(error.message.contains("nested deeper"), "{error}");
    }
}
