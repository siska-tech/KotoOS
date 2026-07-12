pub const MAX_VIRTUAL_PATH_LEN: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FsError {
    EmptyPath,
    PathTooLong,
    Traversal,
    InvalidComponent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SandboxPath {
    bytes: [u8; MAX_VIRTUAL_PATH_LEN],
    len: usize,
}

impl SandboxPath {
    pub const fn empty() -> Self {
        Self {
            bytes: [0; MAX_VIRTUAL_PATH_LEN],
            len: 0,
        }
    }

    pub fn resolve(input: &str) -> Result<Self, FsError> {
        let mut path = Self::empty();
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(FsError::EmptyPath);
        }

        for component in trimmed.split('/') {
            match component {
                "" | "." => continue,
                ".." => return Err(FsError::Traversal),
                value => path.push_component(value)?,
            }
        }

        if path.len == 0 {
            return Err(FsError::EmptyPath);
        }

        Ok(path)
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }

    fn push_component(&mut self, component: &str) -> Result<(), FsError> {
        if !is_valid_component(component) {
            return Err(FsError::InvalidComponent);
        }

        let separator_len = usize::from(self.len > 0);
        let needed = self.len + separator_len + component.len();
        if needed > MAX_VIRTUAL_PATH_LEN {
            return Err(FsError::PathTooLong);
        }

        if self.len > 0 {
            self.bytes[self.len] = b'/';
            self.len += 1;
        }
        self.bytes[self.len..self.len + component.len()].copy_from_slice(component.as_bytes());
        self.len += component.len();
        Ok(())
    }
}

fn is_valid_component(component: &str) -> bool {
    !component.is_empty()
        && component
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b'\\' && byte != b':')
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sandbox {
    app_id: SandboxPath,
}

impl Sandbox {
    pub fn new(app_id: &str) -> Result<Self, FsError> {
        Ok(Self {
            app_id: SandboxPath::resolve(app_id)?,
        })
    }

    pub fn app_id(&self) -> &str {
        self.app_id.as_str()
    }

    pub fn resolve(&self, virtual_path: &str) -> Result<SandboxPath, FsError> {
        SandboxPath::resolve(virtual_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_absolute_virtual_path_inside_sandbox() {
        let path = SandboxPath::resolve("/data/save.dat").unwrap();
        assert_eq!(path.as_str(), "data/save.dat");
    }

    #[test]
    fn normalizes_redundant_separators_and_current_dir() {
        let path = SandboxPath::resolve("data//./memo.txt").unwrap();
        assert_eq!(path.as_str(), "data/memo.txt");
    }

    #[test]
    fn rejects_parent_traversal() {
        assert_eq!(
            SandboxPath::resolve("../other/save.dat"),
            Err(FsError::Traversal)
        );
        assert_eq!(SandboxPath::resolve("/../x"), Err(FsError::Traversal));
    }

    #[test]
    fn rejects_empty_or_root_only_paths() {
        assert_eq!(SandboxPath::resolve(""), Err(FsError::EmptyPath));
        assert_eq!(SandboxPath::resolve("/"), Err(FsError::EmptyPath));
    }

    #[test]
    fn rejects_windows_drive_or_backslash_components() {
        assert_eq!(
            SandboxPath::resolve("C:/Users/save.dat"),
            Err(FsError::InvalidComponent)
        );
        assert_eq!(
            SandboxPath::resolve("data\\save.dat"),
            Err(FsError::InvalidComponent)
        );
    }

    #[test]
    fn sandbox_keeps_app_id_separate_from_virtual_path() {
        let sandbox = Sandbox::new("dev.koto.memo").unwrap();
        let path = sandbox.resolve("/data/save.dat").unwrap();
        assert_eq!(sandbox.app_id(), "dev.koto.memo");
        assert_eq!(path.as_str(), "data/save.dat");
    }
}
