use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use koto_core::{FileHandle, FileMode, FsHal, HalError, SandboxPath};

#[derive(Debug, Default)]
pub struct HostFs {
    root: Option<PathBuf>,
}

impl HostFs {
    pub fn new() -> Self {
        Self { root: None }
    }

    pub fn mounted(root: impl AsRef<Path>) -> Result<Self, HalError> {
        let mut fs = Self::new();
        fs.mount_path(root)?;
        Ok(fs)
    }

    pub fn mount_path(&mut self, root: impl AsRef<Path>) -> Result<(), HalError> {
        let root = root.as_ref();
        if root.as_os_str().is_empty() {
            return Err(HalError::InvalidArgument);
        }
        self.root = Some(root.to_path_buf());
        Ok(())
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn read_dir(&self, path: &str) -> Result<Vec<HostDirEntry>, HalError> {
        let virtual_dir = SandboxPath::resolve(path).map_err(|_| HalError::InvalidArgument)?;
        let host_dir = self.resolve_virtual_path(virtual_dir)?;
        let mut entries = Vec::new();

        for entry in fs::read_dir(host_dir).map_err(|_| HalError::Io)? {
            let entry = entry.map_err(|_| HalError::Io)?;
            let name = entry.file_name();
            let name = name.to_str().ok_or(HalError::InvalidArgument)?;
            let virtual_path = join_virtual_path(virtual_dir.as_str(), name)?;
            entries.push(HostDirEntry { virtual_path });
        }

        Ok(entries)
    }

    fn resolve_path(&self, path: &str) -> Result<PathBuf, HalError> {
        let virtual_path = SandboxPath::resolve(path).map_err(|_| HalError::InvalidArgument)?;
        self.resolve_virtual_path(virtual_path)
    }

    fn resolve_virtual_path(&self, virtual_path: SandboxPath) -> Result<PathBuf, HalError> {
        let root = self.root.as_ref().ok_or(HalError::InvalidArgument)?;
        Ok(root.join(virtual_path.as_str()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostDirEntry {
    virtual_path: String,
}

impl HostDirEntry {
    pub fn virtual_path(&self) -> &str {
        &self.virtual_path
    }
}

#[derive(Debug)]
pub struct HostFile {
    file: fs::File,
}

impl FileHandle for HostFile {
    fn read(&mut self, dst: &mut [u8]) -> Result<usize, HalError> {
        self.file.read(dst).map_err(|_| HalError::Io)
    }

    fn write(&mut self, src: &[u8]) -> Result<usize, HalError> {
        self.file.write(src).map_err(|_| HalError::Io)
    }

    fn seek(&mut self, offset: u64) -> Result<(), HalError> {
        self.file
            .seek(SeekFrom::Start(offset))
            .map(|_| ())
            .map_err(|_| HalError::Io)
    }
}

impl FsHal for HostFs {
    type File = HostFile;

    fn mount(&mut self, root: &str) -> Result<(), HalError> {
        self.mount_path(root)
    }

    fn open(&mut self, path: &str, mode: FileMode) -> Result<Self::File, HalError> {
        let path = self.resolve_path(path)?;
        let mut options = fs::OpenOptions::new();
        match mode {
            FileMode::Read => {
                options.read(true);
            }
            FileMode::Write => {
                options.write(true).create(true).truncate(true);
            }
            FileMode::ReadWrite => {
                options.read(true).write(true).create(true);
            }
        }

        Ok(HostFile {
            file: options.open(path).map_err(|_| HalError::Io)?,
        })
    }
}

fn join_virtual_path(parent: &str, child: &str) -> Result<String, HalError> {
    let path = format!("{parent}/{child}");
    SandboxPath::resolve(&path).map_err(|_| HalError::InvalidArgument)?;
    Ok(path)
}
