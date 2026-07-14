//! Embedded storage engine — page allocator, B+ tree, WAL, mmap, crash recovery.

pub mod btree;
pub mod mmap;
pub mod page;
pub mod recovery;
pub mod wal;
