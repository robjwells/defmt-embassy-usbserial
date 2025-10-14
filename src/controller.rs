//! Logger buffers and the buffer controller

use crate::buffer::LogBuffer;

/// The buffer controller of the logger.
pub(super) static mut CONTROLLER: Controller = Controller::new();

/// Controller of the buffers of the logger.
pub struct Controller {
    /// Index of the currently active buffer.
    current_idx: usize,
    /// The controller is enabled.
    enabled: bool,
    /// Alternating buffers holding defmt frames.
    buffers: [LogBuffer; 2],
}

impl Controller {
    /// Static initializer.
    pub const fn new() -> Self {
        Self {
            current_idx: 0,
            enabled: true,
            buffers: [LogBuffer::new(), LogBuffer::new()],
        }
    }

    /// Enables the controller.
    #[inline]
    pub(super) fn enable(&mut self) {
        self.enabled = true;
    }

    /// Disables the controller.
    #[inline]
    pub(super) fn disable(&mut self) {
        self.enabled = false;
    }

    /// Get a mutable reference to the currently active buffer.
    fn current_buffer(&mut self) -> &mut LogBuffer {
        &mut self.buffers[self.current_idx]
    }

    /// Mark the current buffer as flushing and set the other to be active.
    pub(super) fn swap(&mut self) {
        // Do nothing if not enabled.
        if !self.enabled {
            return;
        }

        // Mark the current buffer as flushing.
        self.current_buffer().flush();

        // 'Swap' the buffers by xor-ing the current index with 1.
        self.current_idx ^= 1;
    }

    /// Writes to the current buffer.
    #[inline]
    pub(super) fn write(&mut self, bytes: &[u8]) {
        // Do nothing if not enabled.
        if !self.enabled {
            return;
        }

        // If the current buffer accepts the necessary bytes, write to it.
        if self.current_buffer().accepts(bytes.len()) {
            // Write to the buffer the data.
            self.current_buffer().write(bytes);
        } else {
            // If it doesn't accept the bytes, mark it as flushing and swap buffers.
            self.swap();

            // TODO: What if the alternate buffer _does not_ accept the bytes?

            // Attempt to write to the newly-active buffer.
            if self.current_buffer().accepts(bytes.len()) {
                // Write to the buffer the data.
                self.current_buffer().write(bytes);
            }
        }
    }

    pub(super) fn get_flushing(&mut self) -> Option<&mut LogBuffer> {
        self.buffers.iter_mut().find(|b| b.is_flushing())
    }
}
