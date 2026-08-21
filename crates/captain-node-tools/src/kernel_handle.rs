//! Compile-time placeholder for the Full runtime hook accepted by shared file handlers.
//!
//! The lightweight Node always passes `None`; no kernel implementation exists
//! in this crate and therefore no broader workspace roots can be granted here.

pub(crate) trait KernelHandle: Send + Sync {}
