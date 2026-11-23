//! Byte pipe used by [`crate::Device`].
//!
//! The USB backend implements this trait. Tests implement it with a script.

use std::future::Future;
use std::time::Duration;

use crate::error::Result;

/// One bulk OUT pipe and one bulk IN pipe.
///
/// `read` places the next USB transfer into `buf` and returns how many bytes
/// arrived. A timeout returns [`crate::Error::IoTimeout`] and leaves no
/// transfer pending.
pub trait Transport: Send {
    /// Send one already-encoded frame.
    fn write(&mut self, data: &[u8], timeout: Duration) -> impl Future<Output = Result<()>> + Send;

    /// Receive the next IN transfer into `buf`.
    fn read(
        &mut self,
        buf: &mut [u8],
        timeout: Duration,
    ) -> impl Future<Output = Result<usize>> + Send;
}
