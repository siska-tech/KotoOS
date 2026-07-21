//! Host regression tests for the KOTO-0227 arena-owned future slot.

#[path = "../src/koto-pico/src/firmware/arena_future.rs"]
mod arena_future;

mod firmware {
    pub mod arena_future {
        pub use crate::arena_future::*;
    }
}

#[path = "../src/koto-pico/src/firmware/wifi_residency.rs"]
mod wifi_residency;

use std::{
    cell::Cell,
    future::Future,
    mem::MaybeUninit,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

use arena_future::{zeroize_arena, ArenaFuture, ArenaFutureError};
use wifi_residency::{
    wifi_lifecycle_phase, wifi_spi_telemetry, WifiLifecycleController, WifiLifecycleError,
    WifiLifecyclePhase, WifiSpiTelemetry, WIFI_RESIDENCY_BYTES,
};

struct CountedFuture {
    polls: Rc<Cell<u32>>,
    drops: Rc<Cell<u32>>,
    ready_after: u32,
}

impl Future for CountedFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<()> {
        let polls = self.polls.get() + 1;
        self.polls.set(polls);
        if polls >= self.ready_after {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

impl Drop for CountedFuture {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

struct LargeFuture {
    _bytes: [u8; 256],
}

impl Future for LargeFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<()> {
        Poll::Pending
    }
}

#[test]
fn completion_drops_future_before_storage_reuse() {
    let polls = Rc::new(Cell::new(0));
    let drops = Rc::new(Cell::new(0));
    let mut storage = [MaybeUninit::uninit(); 128];
    let mut future = ArenaFuture::try_new(
        &mut storage,
        CountedFuture {
            polls: polls.clone(),
            drops: drops.clone(),
            ready_after: 2,
        },
    )
    .unwrap();
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);

    assert_eq!(future.poll_once(&mut context), Poll::Pending);
    assert!(future.is_active());
    assert_eq!(future.poll_once(&mut context), Poll::Ready(()));
    assert!(!future.is_active());
    assert_eq!(polls.get(), 2);
    assert_eq!(drops.get(), 1);

    future.cancel();
    drop(future);
    assert_eq!(drops.get(), 1);
}

#[test]
fn cancellation_drops_pending_future_exactly_once() {
    let polls = Rc::new(Cell::new(0));
    let drops = Rc::new(Cell::new(0));
    let mut storage = [MaybeUninit::uninit(); 128];
    let mut future = ArenaFuture::try_new(
        &mut storage,
        CountedFuture {
            polls: polls.clone(),
            drops: drops.clone(),
            ready_after: u32::MAX,
        },
    )
    .unwrap();

    future.cancel();
    assert!(!future.is_active());
    assert_eq!(drops.get(), 1);
    drop(future);
    assert_eq!(drops.get(), 1);
}

#[test]
fn released_workspace_is_overwritten_before_reuse() {
    let mut storage = [MaybeUninit::new(0xa5); 128];
    zeroize_arena(&mut storage);
    assert!(storage
        .iter()
        .all(|byte| unsafe { byte.assume_init() } == 0));
}

#[test]
fn oversized_future_is_rejected_without_leaking() {
    let mut storage = [MaybeUninit::uninit(); 64];
    let mut controller = WifiLifecycleController::new();
    let result = controller.try_install(1, &mut storage, LargeFuture { _bytes: [0; 256] });
    assert_eq!(
        result,
        Err(WifiLifecycleError::Future(
            ArenaFutureError::InsufficientStorage
        ))
    );
}

#[test]
fn lifecycle_join_is_published_after_future_drop() {
    assert_eq!(WIFI_RESIDENCY_BYTES, 36 * 1024);
    assert_eq!(wifi_lifecycle_phase(), WifiLifecyclePhase::Offline);
    assert_eq!(
        wifi_spi_telemetry(),
        WifiSpiTelemetry {
            reads: 0,
            writes: 0,
            last_status: 0,
            last_word: 0,
            power_highs: 0,
            power_latch_high: false,
            power_input_high: false,
            gpio_in: 0,
            pin_funcs: 0,
            pio_ctrl: 0,
            pio_fstat: 0,
            pio_fdebug: 0,
            pio_padout: 0,
            pio_padoe: 0,
            pio_sm0_addr: 0,
        }
    );
    let polls = Rc::new(Cell::new(0));
    let drops = Rc::new(Cell::new(0));
    let mut storage = [MaybeUninit::uninit(); 128];
    let future = ArenaFuture::try_new(
        &mut storage,
        CountedFuture {
            polls: polls.clone(),
            drops: drops.clone(),
            ready_after: 1,
        },
    )
    .unwrap();
    let mut controller = WifiLifecycleController::new();
    controller.install(7, future).unwrap();

    controller.service();

    assert!(!controller.is_active());
    assert_eq!(controller.joined_generation(), 7);
    assert_eq!(controller.polls(), 1);
    assert_eq!(drops.get(), 1);
}

#[test]
fn stale_cancel_does_not_drop_active_lifecycle() {
    let polls = Rc::new(Cell::new(0));
    let drops = Rc::new(Cell::new(0));
    let mut storage = [MaybeUninit::uninit(); 128];
    let future = ArenaFuture::try_new(
        &mut storage,
        CountedFuture {
            polls,
            drops: drops.clone(),
            ready_after: u32::MAX,
        },
    )
    .unwrap();
    let mut controller = WifiLifecycleController::new();
    controller.install(11, future).unwrap();

    assert_eq!(
        controller.cancel(10),
        Err(WifiLifecycleError::StaleGeneration)
    );
    assert!(controller.is_active());
    assert_eq!(drops.get(), 0);

    controller.cancel(11).unwrap();
    assert!(!controller.is_active());
    assert_eq!(controller.joined_generation(), 11);
    assert_eq!(drops.get(), 1);
}
