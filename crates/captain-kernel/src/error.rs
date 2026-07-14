//! Kernel-specific error types.

use captain_types::error::CaptainError;
use thiserror::Error;

/// Kernel error type wrapping CaptainError with kernel-specific context.
#[derive(Error, Debug)]
pub enum KernelError {
    /// A wrapped CaptainError.
    #[error(transparent)]
    Captain(#[from] CaptainError),

    /// The kernel failed to boot.
    #[error("Boot failed: {0}")]
    BootFailed(String),
}

/// Alias for kernel results.
pub type KernelResult<T> = Result<T, KernelError>;
