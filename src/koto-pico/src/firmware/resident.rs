//! Mutually-exclusive resident storage for the launcher and app code cache.

use core::{
    mem::{needs_drop, size_of, ManuallyDrop},
    slice,
};

use koto_core::ShellState;

/// Read-only object representation used for the exact PSRAM snapshot.
pub fn shell_value_bytes(shell: &ShellState) -> &[u8] {
    // SAFETY: the returned slice shares the input lifetime and is read-only.
    unsafe { slice::from_raw_parts((shell as *const ShellState).cast(), size_of::<ShellState>()) }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ActiveResident {
    Shell,
    Code,
}

#[repr(C)]
union ResidentStorage<const CODE_BYTES: usize> {
    shell: ManuallyDrop<ShellState>,
    code: ManuallyDrop<[u8; CODE_BYTES]>,
}

/// One SRAM slot whose active value is either the shell or the app code cache.
pub struct ShellCodeResident<const CODE_BYTES: usize> {
    storage: ResidentStorage<CODE_BYTES>,
    active: ActiveResident,
}

impl<const CODE_BYTES: usize> ShellCodeResident<CODE_BYTES> {
    const LAYOUT_VALID: () = {
        assert!(size_of::<ShellState>() <= CODE_BYTES);
        // Raw preservation is valid only while ShellState owns no resources
        // requiring a destructor. Its current fields are bounded value types.
        assert!(!needs_drop::<ShellState>());
    };

    pub const fn new() -> Self {
        let () = Self::LAYOUT_VALID;
        Self {
            storage: ResidentStorage {
                shell: ManuallyDrop::new(ShellState::empty()),
            },
            active: ActiveResident::Shell,
        }
    }

    pub fn shell_mut(&mut self) -> Option<&mut ShellState> {
        if self.active != ActiveResident::Shell {
            return None;
        }
        // SAFETY: the state tag proves that `shell` is the active union field.
        Some(unsafe { &mut *(&mut self.storage.shell as *mut ManuallyDrop<ShellState>).cast() })
    }

    pub fn shell_bytes(&self) -> Option<&[u8]> {
        if self.active != ActiveResident::Shell {
            return None;
        }
        // SAFETY: the shell field is active and the slice covers exactly its
        // initialized representation. The caller only copies these bytes.
        Some(unsafe {
            slice::from_raw_parts(
                (&self.storage.shell as *const ManuallyDrop<ShellState>).cast::<u8>(),
                size_of::<ShellState>(),
            )
        })
    }

    pub fn begin_code(&mut self) -> Option<&mut [u8; CODE_BYTES]> {
        if self.active != ActiveResident::Shell {
            return None;
        }
        self.active = ActiveResident::Code;
        // SAFETY: `[u8; N]` accepts every bit pattern. Changing the tag makes
        // the code field active before the mutable reference escapes.
        Some(unsafe {
            &mut *(&mut self.storage.code as *mut ManuallyDrop<[u8; CODE_BYTES]>).cast()
        })
    }

    /// Restores the exact shell representation into the inactive code slot.
    pub fn restore_shell_with(&mut self, fill: impl FnOnce(&mut [u8]) -> bool) -> bool {
        if self.active != ActiveResident::Code {
            return false;
        }
        // SAFETY: the code field is byte-addressable and is at least as large
        // as ShellState. No shell reference exists during this callback.
        let bytes = unsafe {
            slice::from_raw_parts_mut(
                (&mut self.storage.code as *mut ManuallyDrop<[u8; CODE_BYTES]>).cast::<u8>(),
                size_of::<ShellState>(),
            )
        };
        if !fill(bytes) {
            return false;
        }
        self.active = ActiveResident::Shell;
        true
    }

    pub const fn shell_bytes_len() -> usize {
        size_of::<ShellState>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use koto_core::shell::StorageStatus;

    const TEST_CODE_BYTES: usize = size_of::<ShellState>() + 16;

    #[test]
    fn swaps_shell_bytes_through_the_code_slot() {
        let mut resident = ShellCodeResident::<TEST_CODE_BYTES>::new();
        resident
            .shell_mut()
            .unwrap()
            .set_storage_status(StorageStatus::Present);
        let mut saved = std::vec![0; ShellCodeResident::<TEST_CODE_BYTES>::shell_bytes_len()];
        saved.copy_from_slice(resident.shell_bytes().unwrap());

        resident.begin_code().unwrap().fill(0xa5);
        assert!(resident.shell_mut().is_none());
        assert!(resident.restore_shell_with(|dst| {
            dst.copy_from_slice(&saved);
            true
        }));
        assert_eq!(
            resident.shell_mut().unwrap().storage_status(),
            StorageStatus::Present
        );
    }

    #[test]
    fn failed_restore_does_not_activate_shell() {
        let mut resident = ShellCodeResident::<TEST_CODE_BYTES>::new();
        let _ = resident.begin_code().unwrap();
        assert!(!resident.restore_shell_with(|_| false));
        assert!(resident.shell_mut().is_none());
    }
}
