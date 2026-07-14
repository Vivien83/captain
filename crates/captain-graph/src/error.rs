//! Error types for hora-graph-core operations.

use std::fmt;

/// All errors returned by hora-graph-core.
#[derive(Debug)]
#[non_exhaustive]
pub enum HoraError {
    // Storage
    /// An I/O error occurred.
    Io(std::io::Error),
    /// A page failed its CRC32 integrity check.
    CorruptedFile {
        /// Page number that failed verification.
        page: u32,
        /// Checksum stored in the page header.
        expected_checksum: u32,
        /// Checksum computed from the page data.
        actual_checksum: u32,
    },
    /// The file is structurally invalid (bad magic, header, etc.).
    InvalidFile {
        /// Human-readable reason for the failure.
        reason: &'static str,
    },
    /// The file was written by an unsupported version of the engine.
    VersionMismatch {
        /// Version number found in the file.
        file_version: u16,
        /// Minimum version this build can read.
        min_supported: u16,
        /// Maximum version this build can read.
        max_supported: u16,
    },

    // Schema / Input
    /// No entity exists with the given ID.
    EntityNotFound(u64),
    /// No edge exists with the given ID.
    EdgeNotFound(u64),
    /// Embedding dimension does not match the configured value.
    DimensionMismatch {
        /// Dimension expected by the engine.
        expected: usize,
        /// Dimension provided by the caller.
        got: usize,
    },
    /// The fact has already been invalidated (bi-temporal soft-delete).
    AlreadyInvalidated(u64),

    // Capacity
    /// A string value exceeds the maximum allowed byte length.
    StringTooLong {
        /// Maximum allowed byte length.
        max: usize,
        /// Actual byte length provided.
        got: usize,
    },
    /// The storage backend has no space left to allocate new pages.
    StorageFull,

    // SQLite backend
    /// A SQLite backend error.
    #[cfg(feature = "sqlite")]
    Sqlite(String),

    // PostgreSQL backend
    /// A PostgreSQL backend error.
    #[cfg(feature = "postgres")]
    Postgres(String),
}

impl fmt::Display for HoraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {}", e),
            Self::CorruptedFile {
                page,
                expected_checksum,
                actual_checksum,
            } => write!(
                f,
                "corrupted page {}: expected checksum 0x{:08x}, got 0x{:08x}",
                page, expected_checksum, actual_checksum
            ),
            Self::InvalidFile { reason } => write!(f, "invalid .hora file: {}", reason),
            Self::VersionMismatch {
                file_version,
                min_supported,
                max_supported,
            } => write!(
                f,
                "version {} not supported (supported: {}-{})",
                file_version, min_supported, max_supported
            ),
            Self::EntityNotFound(id) => write!(f, "entity {} not found", id),
            Self::EdgeNotFound(id) => write!(f, "edge {} not found", id),
            Self::DimensionMismatch { expected, got } => {
                write!(
                    f,
                    "embedding dimension mismatch: expected {}, got {}",
                    expected, got
                )
            }
            Self::AlreadyInvalidated(id) => write!(f, "fact {} is already invalidated", id),
            Self::StringTooLong { max, got } => {
                write!(f, "string too long: max {} bytes, got {}", max, got)
            }
            Self::StorageFull => write!(f, "storage is full"),
            #[cfg(feature = "sqlite")]
            Self::Sqlite(msg) => write!(f, "SQLite error: {}", msg),
            #[cfg(feature = "postgres")]
            Self::Postgres(msg) => write!(f, "PostgreSQL error: {}", msg),
        }
    }
}

impl std::error::Error for HoraError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for HoraError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Result type alias for hora-graph-core operations.
pub type Result<T> = std::result::Result<T, HoraError>;
