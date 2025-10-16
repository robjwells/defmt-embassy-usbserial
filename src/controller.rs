//! Logger buffers and the buffer controller

use core::{cell::UnsafeCell, sync::atomic::Ordering};

use portable_atomic::{AtomicBool, AtomicUsize};

use crate::buffer::LogBuffer;

/// The buffer controller of the logger.
pub(super) static CONTROLLER: Controller = Controller::new();

/// Controller of the buffers of the logger.
pub struct Controller {
    /// Index of the currently active buffer.
    current_idx: AtomicUsize,
    /// The controller is enabled.
    enabled: AtomicBool,
    /// Alternating buffers holding defmt frames.
    //
    // SAFETY: These are OK to be unsynchronised UnsafeCells because they are only written to from
    // within a critical section, and taken out of use by that critical section (marked as
    // flushing). They are only put back into use by the asynchronous logger task outside of the
    // critical sections where writing occurs.
    buffers: [UnsafeCell<LogBuffer>; 2],
}

// Sync is required for types in static variables.
//
// SAFETY: This is safe to implement because mutation of the LogBuffers only occurs within a
// critical section, preventing concurrent modification.
unsafe impl Sync for Controller {}

impl Controller {
    /// Static initializer.
    pub const fn new() -> Self {
        Self {
            current_idx: AtomicUsize::new(0),
            enabled: AtomicBool::new(true),
            buffers: [
                UnsafeCell::new(LogBuffer::new()),
                UnsafeCell::new(LogBuffer::new()),
            ],
        }
    }

    /// Enables the controller.
    #[inline]
    pub(super) fn enable(&self) {
        self.enabled.store(true, Ordering::Relaxed);
    }

    /// Disables the controller.
    ///
    /// A disabled controller silently ignores any defmt logging.
    ///
    /// The internal buffers are reset when the controller is disabled to prevent any
    /// partial frames being transmitted when the controller is re-enabled.
    #[inline]
    pub(super) fn disable(&self) {
        self.enabled.store(false, Ordering::Relaxed);
        let first = self.buffers[0].get();
        let second = self.buffers[1].get();
        critical_section::with(|_| {
            // SAFETY: We are in a critical section, and this function is only called on
            // EndpointError::Disabled when flushing a buffer. It cannot disturb any ongoing defmt
            // writes because they take their own critical section, and the controller is already
            // marked as disabled so any new defmt writes (or flushes) will be ignored.
            unsafe { &mut *first }.reset();
            unsafe { &mut *second }.reset();
        });
    }

    /// Mark the current buffer as flushing and set the other to be active.
    ///
    /// # Safety
    ///
    /// Callers must ensure they are inside a critical section and there are no conflicting updates
    /// made to the buffer index or the current buffer's state enum.
    pub(super) unsafe fn swap(&self) {
        // Do nothing if not enabled.
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }

        let current_idx = self.current_idx.load(Ordering::Relaxed);

        // SAFETY: We are OK to get a &mut to the current buffer because we are in a critical
        // section, and it is held only for the purposes of changing the buffer's state enum,
        // and in the critical section we only ever change the state to mark it as flushing.
        unsafe {
            let current = &mut *self.buffers[current_idx].get();
            // Mark the current buffer as flushing.
            current.flush();
        }

        // 'Swap' the buffers by xor-ing the current index with 1.
        // This is the only place where current_idx is changed.
        self.current_idx.store(current_idx ^ 1, Ordering::Relaxed);
    }

    /// Write defmt-encoded bytes to the current buffer.
    ///
    /// # Safety
    ///
    /// This writes to the underlying buffers, so the caller must ensure they are
    /// inside a critical section.
    #[inline]
    pub(super) unsafe fn write(&self, bytes: &[u8]) {
        // Do nothing if not enabled.
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }

        let current_idx = self.current_idx.load(Ordering::Relaxed);
        let other_idx = current_idx ^ 1;

        // SAFETY: This function is only called while a critical section is held by the defmt
        // logger, so we are OK to mutate the buffers. This is also the only place where the
        // buffers' underlying store is changed.
        let current = unsafe { &mut *(self.buffers[current_idx].get()) };
        let other = unsafe { &mut *(self.buffers[other_idx].get()) };
        // If the current buffer accepts the necessary bytes, write to it.
        if current.accepts(bytes.len()) {
            // Write to the buffer the data.
            current.write(bytes);
        } else {
            // If it doesn't accept the bytes, mark it as flushing and swap buffers.
            // TODO: What if the alternate buffer _does not_ accept the bytes?
            // TODO: Document safety of this.
            self.swap();

            if other.accepts(bytes.len()) {
                // Write to the buffer the data.
                other.write(bytes);
            }
        }
    }

    /// Get a buffer that needs to be flushed to USB.
    ///
    /// Should _both_ buffers need flushing, it will flush the one at index 0 first.
    ///
    /// This is a purely a convenience for use in `flush`.
    fn get_flushing(&self) -> Option<(usize, &LogBuffer)> {
        for (idx, cell) in self.buffers.iter().enumerate() {
            // SAFETY: swap, used in the defmt critical section, only ever marks a buffer as
            // flushing (*never* as active), so if a buffer is marked as flushing it will not
            // change until the caller of this function requests it to be reset.
            let buf = unsafe { &*cell.get() };
            if buf.is_flushing() {
                return Some((idx, buf));
            }
        }
        None
    }

    /// Return a buffer to service after it has been flushed.
    ///
    /// This mutates the buffer state, and is only to be used inside the controller.
    fn reset_buffer(&self, buf_idx: usize) {
        // We use a critical section here to ensure that the buffer is never in a state where it
        // has not fully reset itself.
        let cell = self.buffers[buf_idx].get();
        critical_section::with(|_| {
            // SAFETY: We are in a critical section.
            unsafe { &mut *cell }.reset();
        });
    }

    pub(crate) async fn flush<F, E>(&self, mut flusher: F) -> Result<(), E>
    where
        F: AsyncFnMut(&[u8]) -> Result<(), E>,
    {
        if let Some((buf_idx, buffer)) = self.get_flushing() {
            // Only provide the used portion of the buffer.
            let bytes = &buffer.data[..buffer.cursor];
            let res = flusher(bytes).await;
            // Always reset the buffer: this is the desired action in case of success,
            // and unavoidable in case of error, because we cannot know how much of
            // the buffer was sent.
            self.reset_buffer(buf_idx);
            // Propagate any error to the caller.
            res?;
        }
        // Nothing to flush, or flush completed without issue.
        Ok(())
    }
}
