//! `defmt` logger and USB transport layer.

#![no_std]

mod buffer;
mod controller;
mod task;

use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, Ordering},
};

pub use task::{logger, run};

static USB_ENCODER: UsbEncoder = UsbEncoder::new();

struct UsbEncoder {
    /// A boolean lock
    ///
    /// Is `true` when `acquire` has been called and we have exclusive access to the
    /// rest of this struct.
    taken: AtomicBool,
    /// Critical section restore state
    ///
    /// Needed to exit a critical section.
    restore: UnsafeCell<critical_section::RestoreState>,
    /// A defmt Encoder for encoding frames
    encoder: UnsafeCell<defmt::Encoder>,
}

unsafe impl Sync for UsbEncoder {}

impl UsbEncoder {
    const fn new() -> Self {
        Self {
            taken: AtomicBool::new(false),
            restore: UnsafeCell::new(critical_section::RestoreState::invalid()),
            encoder: UnsafeCell::new(defmt::Encoder::new()),
        }
    }

    /// Acquire the defmt logger
    ///
    /// This acquires a critical section and begins a defmt frame.
    ///
    /// # Panics
    ///
    /// This will panic if you attempt to acquire the logger re-entrantly.
    fn acquire(&self) {
        // Get in a critical section.
        //
        // SAFETY: Must be paired with a call to release, as it is in the contract of
        // the Logger trait.
        let restore_state = unsafe { critical_section::acquire() };

        // Fail if the logger is acquired re-entrantly, to avoid two places with
        // mutable access to the logger state.
        if self.taken.load(Ordering::Relaxed) {
            panic!("defmt logger taken reentrantly");
        }

        // Set the boolean lock now that we're in a critical section and we know
        // it is not already taken.
        self.taken.store(true, Ordering::Relaxed);

        // SAFETY: Accessing the UnsafeCells is OK because we are in a critical section.
        unsafe {
            // Store the value needed to exit the critical section.
            self.restore.get().write(restore_state);

            // Start the defmt frame.
            let encoder = &mut *self.encoder.get();
            encoder.start_frame(Self::inner);
        }
    }

    /// Release the defmt logger
    ///
    /// This finishes the defmt frame and releases the critical section.
    ///
    /// # Safety
    ///
    /// Must be called exactly once after calling acquire.
    unsafe fn release(&self) {
        // Ensure we are not attempting to release while not in a critical section.
        if !self.taken.load(Ordering::Relaxed) {
            panic!("defmt release outside of critical section.")
        }

        // SAFETY: Accessing the UnsafeCells and finally releasing the critical section
        // is OK because we know we are in a critical section at this point.
        unsafe {
            let encoder = &mut *self.encoder.get();
            encoder.end_frame(Self::inner);

            let restore_state = self.restore.get().read();
            self.taken.store(false, Ordering::Relaxed);
            critical_section::release(restore_state);
        }
    }

    /// Flush the current buffer.
    ///
    /// # Safety
    ///
    /// Must be called after calling `acquire` and before calling `release`.
    unsafe fn flush(&self) {
        // SAFETY: Only called while the critical section is held.
        #[allow(static_mut_refs)]
        controller::CONTROLLER.swap()
    }

    /// Write bytes to the defmt encoder.
    ///
    /// # Safety
    ///
    /// Must be called after calling `acquire` and before calling `release`.
    unsafe fn write(&self, bytes: &[u8]) {
        let encoder = &mut *self.encoder.get();
        encoder.write(bytes, Self::inner)
    }

    fn inner(bytes: &[u8]) {
        unsafe {
            // SAFETY: Called by Logger trait methods that ensure a critical section is held.
            #[allow(static_mut_refs)]
            controller::CONTROLLER.write(bytes)
        };
    }
}

/// The logger implementation.
#[defmt::global_logger]
pub struct USBLogger;

unsafe impl defmt::Logger for USBLogger {
    fn acquire() {
        USB_ENCODER.acquire();
    }

    unsafe fn release() {
        USB_ENCODER.release();
    }

    unsafe fn flush() {
        USB_ENCODER.flush();
    }

    unsafe fn write(bytes: &[u8]) {
        USB_ENCODER.write(bytes);
    }
}
