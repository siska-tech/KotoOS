//! Type-erased future stored in a caller-owned bounded byte arena.

use core::{
    future::Future,
    marker::PhantomData,
    mem::{align_of, size_of, MaybeUninit},
    pin::Pin,
    task::{Context, Poll},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArenaFutureError {
    InsufficientStorage,
    UnsupportedAlignment,
}

/// Volatile overwrite used before a shared arena changes owners. The caller
/// must first drop the [`ArenaFuture`] borrowing this storage.
pub fn zeroize_arena(storage: &mut [MaybeUninit<u8>]) {
    for byte in storage {
        unsafe { byte.as_mut_ptr().write_volatile(0) };
    }
}

/// Poll handle for one future whose concrete frame lives in `storage`.
///
/// Dropping or cancelling this handle drops the concrete future first, so its
/// peripheral handles and arena borrows are released before the caller reuses
/// the backing bytes.
pub struct ArenaFuture<'storage> {
    pointer: *mut u8,
    poll_fn: unsafe fn(*mut u8, &mut Context<'_>) -> Poll<()>,
    drop_fn: unsafe fn(*mut u8),
    active: bool,
    _storage: PhantomData<&'storage mut [MaybeUninit<u8>]>,
}

impl<'storage> ArenaFuture<'storage> {
    pub fn try_new<F>(
        storage: &'storage mut [MaybeUninit<u8>],
        future: F,
    ) -> Result<Self, ArenaFutureError>
    where
        F: Future<Output = ()> + 'storage,
    {
        let alignment = align_of::<F>();
        if !alignment.is_power_of_two() {
            return Err(ArenaFutureError::UnsupportedAlignment);
        }
        let start = storage.as_mut_ptr() as usize;
        let aligned = start
            .checked_add(alignment - 1)
            .map(|address| address & !(alignment - 1))
            .ok_or(ArenaFutureError::InsufficientStorage)?;
        let offset = aligned.saturating_sub(start);
        let required = size_of::<F>().max(1);
        if offset
            .checked_add(required)
            .is_none_or(|end| end > storage.len())
        {
            return Err(ArenaFutureError::InsufficientStorage);
        }

        let pointer = aligned as *mut u8;
        unsafe { pointer.cast::<F>().write(future) };
        Ok(Self {
            pointer,
            poll_fn: poll_future::<F>,
            drop_fn: drop_future::<F>,
            active: true,
            _storage: PhantomData,
        })
    }

    pub const fn is_active(&self) -> bool {
        self.active
    }

    pub fn poll_once(&mut self, context: &mut Context<'_>) -> Poll<()> {
        if !self.active {
            return Poll::Ready(());
        }
        let result = unsafe { (self.poll_fn)(self.pointer, context) };
        if result.is_ready() {
            self.drop_active();
        }
        result
    }

    pub fn cancel(&mut self) {
        self.drop_active();
    }

    fn drop_active(&mut self) {
        if self.active {
            unsafe { (self.drop_fn)(self.pointer) };
            self.active = false;
        }
    }
}

impl Drop for ArenaFuture<'_> {
    fn drop(&mut self) {
        self.drop_active();
    }
}

unsafe fn poll_future<F>(pointer: *mut u8, context: &mut Context<'_>) -> Poll<()>
where
    F: Future<Output = ()>,
{
    let future = unsafe { &mut *pointer.cast::<F>() };
    unsafe { Pin::new_unchecked(future) }.poll(context)
}

unsafe fn drop_future<F>(pointer: *mut u8) {
    unsafe { core::ptr::drop_in_place(pointer.cast::<F>()) };
}
