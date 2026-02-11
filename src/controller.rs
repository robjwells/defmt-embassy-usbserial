//! Logger buffers and the buffer controller

use core::{cell::UnsafeCell, sync::atomic::Ordering};

use loopq::embassy::{AsyncBuffer, AsyncProducer};
use portable_atomic::AtomicBool;

/// The buffer size.
#[cfg(feature = "buffersize-64")]
const BUFFERSIZE: usize = 64;

#[cfg(feature = "buffersize-128")]
const BUFFERSIZE: usize = 128;

#[cfg(feature = "buffersize-256")]
const BUFFERSIZE: usize = 256;

#[cfg(feature = "buffersize-512")]
const BUFFERSIZE: usize = 512;

#[cfg(feature = "buffersize-1024")]
const BUFFERSIZE: usize = 1024;

/// The global ring buffer.
pub(super) static RING_BUFFER: AsyncBuffer<BUFFERSIZE> = AsyncBuffer::new();

/// The buffer controller of the logger.
pub(super) static CONTROLLER: Controller = Controller::new();

/// Controller of the buffers of the logger.
pub struct Controller {
    /// The producer handle.
    ///
    /// The producer is initialized lazily on the first write.
    /// It is wrapped in an `UnsafeCell` to allow interior mutability required to get a mutable
    /// reference from a shared reference in `write`.
    ///
    /// SAFETY: Write access to this is only obtained within a critical section (guaranteed by
    /// `defmt::Logger`), so it is safe to act as if we have exclusive access.
    producer: UnsafeCell<Option<AsyncProducer<'static, BUFFERSIZE>>>,
}

unsafe impl Sync for Controller {}

impl Controller {
    /// Static initializer.
    pub const fn new() -> Self {
        Self {
            producer: UnsafeCell::new(None),
        }
    }

    /// Write defmt-encoded bytes to the ring buffer.
    ///
    /// # Safety
    ///
    /// This writes to the underlying buffers, so the caller must ensure they are
    /// inside a critical section.
    #[inline]
    pub(super) unsafe fn write(&self, bytes: &[u8]) {
        // SAFETY: We are in a critical section, so we have exclusive access to the producer.
        // We wrap the dereference in an unsafe block to satisfy the `unsafe_op_in_unsafe_fn` lint.
        let producer_opt = unsafe { &mut *self.producer.get() };

        // Lazily initialize the producer if it hasn't been already.
        let producer = producer_opt.get_or_insert_with(|| RING_BUFFER.producer());

        let mut remaining = bytes;
        while !remaining.is_empty() {
            // Get writable bytes.
            // We use try_writable_bytes because this is a synchronous context and we cannot await.
            let mut writable = producer.try_writable_bytes();
            // We can only write as much as is available in the contiguous slice.
            if writable.is_empty() {
                // Buffer full.
                break;
            }

            let chunk_len = core::cmp::min(writable.len(), remaining.len());
            writable[..chunk_len].copy_from_slice(&remaining[..chunk_len]);
            writable.commit(chunk_len);

            remaining = &remaining[chunk_len..];
        }
    }
}
