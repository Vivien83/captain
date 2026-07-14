//! Crash recovery and database lifecycle management.
//!
//! `Database` ties together PageAllocator, WAL, and file I/O into a single
//! entry point with automatic crash recovery on open.
//!
//! Recovery sequence (on open):
//! 1. Read file header → verify magic "HORA", version, CRC32
//! 2. Load pages into PageAllocator
//! 3. If WAL file exists → scan frames, verify checksums, replay valid ones
//! 4. Return a ready-to-use Database

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::error::{HoraError, Result};

use super::mmap::write_pages_to_file;
use super::page::{crc32, PageAllocator, PageType};
use super::wal::{WalFrame, WalHeader, WriteAheadLog};

// ── File header ──────────────────────────────────────────

const FILE_MAGIC: [u8; 4] = *b"HORA";
const FILE_VERSION: u16 = 1;

/// Size of the file header (bytes 0..32 of page 0).
const FILE_HEADER_SIZE: usize = 32;

/// Database file header stored in the first 32 bytes of page 0.
///
/// ```text
/// [0..4]   magic "HORA"
/// [4..6]   version (u16 LE)
/// [6..10]  page_size (u32 LE)
/// [10..14] page_count (u32 LE)
/// [14..18] freelist_page (u32 LE)
/// [18..22] freelist_count (u32 LE)
/// [22..26] header_checksum (u32 LE) — CRC32 of bytes 0..22
/// [26..32] reserved
/// ```
#[derive(Debug, Clone)]
pub struct FileHeader {
    /// Magic bytes identifying a valid hora database file (`"HORA"`).
    pub magic: [u8; 4],
    /// File format version number.
    pub version: u16,
    /// Page size in bytes used by this database.
    pub page_size: u32,
    /// Total number of pages in the database file.
    pub page_count: u32,
    /// Page number of the first freelist page (0 = no freelist).
    pub freelist_page: u32,
    /// Total number of free pages tracked in the freelist chain.
    pub freelist_count: u32,
    /// CRC32 checksum of header bytes 0..22.
    pub header_checksum: u32,
}

impl FileHeader {
    /// Create a new header for a fresh database with the given page size.
    pub fn new(page_size: u32) -> Self {
        Self {
            magic: FILE_MAGIC,
            version: FILE_VERSION,
            page_size,
            page_count: 1,
            freelist_page: 0,
            freelist_count: 0,
            header_checksum: 0,
        }
    }

    /// Write the header into the first 32 bytes of `buf`.
    pub fn write_to(&self, buf: &mut [u8]) {
        buf[0..4].copy_from_slice(&self.magic);
        buf[4..6].copy_from_slice(&self.version.to_le_bytes());
        buf[6..10].copy_from_slice(&self.page_size.to_le_bytes());
        buf[10..14].copy_from_slice(&self.page_count.to_le_bytes());
        buf[14..18].copy_from_slice(&self.freelist_page.to_le_bytes());
        buf[18..22].copy_from_slice(&self.freelist_count.to_le_bytes());
        let checksum = crc32(&buf[0..22]);
        buf[22..26].copy_from_slice(&checksum.to_le_bytes());
        // bytes 26..32 reserved (zero)
        buf[26..32].fill(0);
    }

    /// Read and verify a header from the first bytes of `buf`.
    pub fn read_from(buf: &[u8]) -> Result<Self> {
        if buf.len() < FILE_HEADER_SIZE {
            return Err(HoraError::InvalidFile {
                reason: "file too short for header",
            });
        }

        let magic: [u8; 4] = buf[0..4].try_into().unwrap();
        if magic != FILE_MAGIC {
            return Err(HoraError::InvalidFile {
                reason: "invalid magic bytes (expected HORA)",
            });
        }

        let version = u16::from_le_bytes([buf[4], buf[5]]);
        if version != FILE_VERSION {
            return Err(HoraError::InvalidFile {
                reason: "unsupported file version",
            });
        }

        let page_size = u32::from_le_bytes(buf[6..10].try_into().unwrap());
        let page_count = u32::from_le_bytes(buf[10..14].try_into().unwrap());
        let freelist_page = u32::from_le_bytes(buf[14..18].try_into().unwrap());
        let freelist_count = u32::from_le_bytes(buf[18..22].try_into().unwrap());
        let stored_checksum = u32::from_le_bytes(buf[22..26].try_into().unwrap());

        let computed_checksum = crc32(&buf[0..22]);
        if stored_checksum != computed_checksum {
            return Err(HoraError::InvalidFile {
                reason: "file header checksum mismatch",
            });
        }

        Ok(Self {
            magic,
            version,
            page_size,
            page_count,
            freelist_page,
            freelist_count,
            header_checksum: stored_checksum,
        })
    }
}

// ── Path helpers ─────────────────────────────────────────

fn wal_path(db_path: &Path) -> PathBuf {
    db_path.with_extension("wal")
}

fn lock_path(db_path: &Path) -> PathBuf {
    db_path.with_extension("lock")
}

// ── Process-alive check ──────────────────────────────────

#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    // SAFETY: kill(pid, 0) only checks process existence, no signal sent.
    // pid as i32: PIDs > i32::MAX are not used by any real OS.
    unsafe { kill(pid as i32, 0) == 0 }
}

#[cfg(not(unix))]
fn is_process_alive(_pid: u32) -> bool {
    false // conservative: assume dead → allow recovery
}

// ── WAL file I/O ─────────────────────────────────────────

/// Persist WAL frames to disk (header + given frames).
fn write_wal_file(path: &Path, header: &WalHeader, frames: &[WalFrame]) -> std::io::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(&header.to_bytes())?;
    for frame in frames {
        file.write_all(&frame.to_bytes())?;
    }
    file.sync_all()?;
    Ok(())
}

/// Read a WAL file and return valid frames. Partial/corrupted frames are skipped.
fn read_wal_file(path: &Path, page_size: usize) -> std::io::Result<Vec<WalFrame>> {
    let mut data = Vec::new();
    File::open(path)?.read_to_end(&mut data)?;

    // Need at least the WAL header (26 bytes)
    if data.len() < 26 {
        return Ok(Vec::new());
    }

    let header = match WalHeader::from_bytes(&data) {
        Some(h) => h,
        None => return Ok(Vec::new()),
    };

    let frame_size = WalFrame::HEADER_SIZE + page_size;
    let mut offset = 26;
    let mut frames = Vec::new();

    while offset + frame_size <= data.len() {
        if let Some(frame) = WalFrame::from_bytes(&data[offset..], page_size) {
            if frame.verify() && frame.salt == header.salt {
                frames.push(frame);
            }
            // Invalid/stale frames silently skipped
        }
        offset += frame_size;
    }
    // Trailing bytes < frame_size → partial frame from crash → ignored

    Ok(frames)
}

// ── Database ─────────────────────────────────────────────

/// Embedded database file with integrated crash recovery.
///
/// Combines PageAllocator (page store) and WriteAheadLog (crash safety)
/// with file I/O and a write lock.
pub struct Database {
    path: PathBuf,
    alloc: PageAllocator,
    wal: WriteAheadLog,
}

impl Database {
    /// Open or create a database at `path` with the given page size.
    ///
    /// If the file does not exist, a new database is created.
    /// If a WAL file exists (from a previous crash), it is replayed automatically.
    pub fn open(path: impl AsRef<Path>, page_size: usize) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        // Acquire write lock
        Self::acquire_lock(&path)?;

        let result = if path.exists() {
            Self::open_existing(&path, page_size)
        } else {
            Self::create_new(&path, page_size)
        };

        if result.is_err() {
            let _ = std::fs::remove_file(lock_path(&path));
        }
        result
    }

    fn acquire_lock(db_path: &Path) -> Result<()> {
        let lock = lock_path(db_path);

        // Atomically create the lock file (fails if it already exists — O_EXCL
        // semantics). This closes the exists()+write() TOCTOU window where two
        // processes could both pass the exists() check and then both write the
        // lock file concurrently.
        match Self::create_lock_file(&lock) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => {
                return Err(HoraError::InvalidFile {
                    reason: "cannot create lock file",
                })
            }
        }

        // The lock file already exists: check whether the owning PID is still alive.
        if let Ok(pid_str) = std::fs::read_to_string(&lock) {
            if let Ok(pid) = pid_str.trim().parse::<u32>() {
                if is_process_alive(pid) {
                    return Err(HoraError::InvalidFile {
                        reason: "database is locked by another process",
                    });
                }
            }
        }

        // Stale lock (dead PID or unreadable) → remove and retry once, atomically.
        let _ = std::fs::remove_file(&lock);
        Self::create_lock_file(&lock).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                HoraError::InvalidFile {
                    reason: "database is locked by another process",
                }
            } else {
                HoraError::InvalidFile {
                    reason: "cannot create lock file",
                }
            }
        })
    }

    /// Create the lock file atomically, failing with `AlreadyExists` if another
    /// process already holds it (equivalent to `open(..., O_CREAT | O_EXCL)`).
    fn create_lock_file(lock: &Path) -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lock)?;
        file.write_all(std::process::id().to_string().as_bytes())?;
        file.sync_all()
    }

    fn create_new(path: &Path, page_size: usize) -> Result<Self> {
        let mut alloc = PageAllocator::new(page_size);

        // Write file header into page 0
        let fh = FileHeader {
            page_count: alloc.page_count(),
            ..FileHeader::new(page_size as u32)
        };
        fh.write_to(alloc.write_page(0)?);

        // Persist to disk
        write_pages_to_file(&alloc, path).map_err(|_| HoraError::InvalidFile {
            reason: "cannot write new database file",
        })?;

        Ok(Self {
            path: path.to_path_buf(),
            alloc,
            wal: WriteAheadLog::new(page_size as u32),
        })
    }

    fn open_existing(path: &Path, page_size: usize) -> Result<Self> {
        let data = std::fs::read(path).map_err(|_| HoraError::InvalidFile {
            reason: "cannot read database file",
        })?;

        if data.len() < page_size {
            return Err(HoraError::InvalidFile {
                reason: "database file too small",
            });
        }

        let fh = FileHeader::read_from(&data)?;
        if fh.page_size as usize != page_size {
            return Err(HoraError::InvalidFile {
                reason: "page size mismatch",
            });
        }

        let mut alloc =
            PageAllocator::from_file_data(page_size, &data, fh.freelist_page, fh.freelist_count);

        // WAL recovery
        let wal_p = wal_path(path);
        if wal_p.exists() {
            let frames = read_wal_file(&wal_p, page_size).map_err(|_| HoraError::InvalidFile {
                reason: "cannot read WAL file",
            })?;
            Self::replay_frames(&mut alloc, &frames);
            let _ = std::fs::remove_file(&wal_p);
        }

        Ok(Self {
            path: path.to_path_buf(),
            alloc,
            wal: WriteAheadLog::new(page_size as u32),
        })
    }

    /// Maximum page number accepted during WAL replay (prevents OOM from crafted WAL).
    const MAX_REPLAY_PAGE: u32 = 16_000_000; // ~64 GB at 4K pages

    /// Replay WAL frames into the allocator.
    fn replay_frames(alloc: &mut PageAllocator, frames: &[WalFrame]) {
        for frame in frames {
            if frame.page_number > Self::MAX_REPLAY_PAGE {
                continue; // skip implausibly large page numbers
            }
            let page_num = frame.page_number as usize;
            // Extend allocator if WAL references pages beyond current size
            while alloc.page_count() as usize <= page_num {
                alloc.alloc_page(PageType::Free);
            }
            if let Ok(page) = alloc.write_page(frame.page_number) {
                let len = frame.data.len().min(page.len());
                page[..len].copy_from_slice(&frame.data[..len]);
            }
        }
    }

    // ── Public API ───────────────────────────────────────

    /// Access the page allocator.
    pub fn alloc(&self) -> &PageAllocator {
        &self.alloc
    }

    /// Mutable access to the page allocator.
    pub fn alloc_mut(&mut self) -> &mut PageAllocator {
        &mut self.alloc
    }

    /// Access the WAL.
    pub fn wal(&self) -> &WriteAheadLog {
        &self.wal
    }

    /// Mutable access to the WAL.
    pub fn wal_mut(&mut self) -> &mut WriteAheadLog {
        &mut self.wal
    }

    /// Write a page through the WAL. Returns true if auto-checkpoint threshold reached.
    pub fn write_frame(&mut self, page_num: u32, data: Vec<u8>) -> bool {
        let db_size = self.alloc.page_count();
        self.wal.write_frame(page_num, db_size, data)
    }

    /// Read a page (WAL-first, fallback to allocator).
    pub fn read_page(&self, page_num: u32) -> Result<&[u8]> {
        if let Some(data) = self.wal.read_page(page_num) {
            Ok(data)
        } else {
            self.alloc.read_page(page_num)
        }
    }

    /// Checkpoint: replay WAL into allocator, write everything to disk, clear WAL.
    ///
    /// Cannot be called during an active transaction.
    pub fn checkpoint(&mut self) -> Result<()> {
        if self.wal.in_transaction() {
            return Err(HoraError::InvalidFile {
                reason: "cannot checkpoint during active transaction",
            });
        }

        // Replay WAL into allocator
        self.wal.checkpoint(&mut self.alloc);

        // Update file header in page 0
        let fh = FileHeader {
            page_count: self.alloc.page_count(),
            freelist_page: self.alloc.freelist_head(),
            freelist_count: self.alloc.freelist_count(),
            ..FileHeader::new(self.alloc.page_size() as u32)
        };
        fh.write_to(self.alloc.write_page(0)?);

        // Write all pages to disk
        write_pages_to_file(&self.alloc, &self.path).map_err(|_| HoraError::InvalidFile {
            reason: "cannot write database file during checkpoint",
        })?;

        // Remove WAL file
        let _ = std::fs::remove_file(wal_path(&self.path));

        Ok(())
    }

    /// Flush committed WAL frames to disk without checkpointing.
    ///
    /// Only committed frames are written. Uncommitted transaction frames
    /// are not persisted — a crash mid-transaction is an automatic rollback.
    pub fn flush_wal(&self) -> Result<()> {
        let committed = self.wal.committed_frames();
        if committed.is_empty() {
            return Ok(());
        }
        write_wal_file(&wal_path(&self.path), self.wal.header(), committed).map_err(|_| {
            HoraError::InvalidFile {
                reason: "cannot write WAL file",
            }
        })
    }

    /// Database file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    // ── Transactions ────────────────────────────────────

    /// Begin a multi-statement transaction.
    ///
    /// Writes within a transaction are visible to the current writer but
    /// not flushed to the WAL file until `commit()`. A crash before commit
    /// is an automatic rollback.
    pub fn begin_transaction(&mut self) -> Result<()> {
        if !self.wal.begin_transaction() {
            return Err(HoraError::InvalidFile {
                reason: "transaction already active",
            });
        }
        Ok(())
    }

    /// Commit the current transaction.
    pub fn commit(&mut self) -> Result<()> {
        if !self.wal.commit_transaction() {
            return Err(HoraError::InvalidFile {
                reason: "no active transaction to commit",
            });
        }
        Ok(())
    }

    /// Rollback the current transaction, discarding all uncommitted writes.
    pub fn rollback(&mut self) -> Result<()> {
        if !self.wal.rollback_transaction() {
            return Err(HoraError::InvalidFile {
                reason: "no active transaction to rollback",
            });
        }
        Ok(())
    }

    /// Whether a transaction is currently active.
    pub fn in_transaction(&self) -> bool {
        self.wal.in_transaction()
    }

    // ── Compaction ───────────────────────────────────────

    /// Incremental compact: fill holes, truncate trailing free pages.
    ///
    /// 1. Checkpoints pending WAL
    /// 2. Two-pointer compaction on the allocator
    /// 3. Persists to disk
    ///
    /// Returns stats including relocations for updating external references.
    pub fn compact(&mut self) -> Result<CompactStats> {
        self.checkpoint()?;

        let old_count = self.alloc.page_count();
        let relocations = self.alloc.compact();
        let new_count = self.alloc.page_count();

        // Update file header
        let fh = FileHeader {
            page_count: new_count,
            freelist_page: self.alloc.freelist_head(),
            freelist_count: self.alloc.freelist_count(),
            ..FileHeader::new(self.alloc.page_size() as u32)
        };
        fh.write_to(self.alloc.write_page(0)?);

        write_pages_to_file(&self.alloc, &self.path).map_err(|_| HoraError::InvalidFile {
            reason: "cannot write database file during compact",
        })?;

        Ok(CompactStats {
            pages_relocated: relocations.len(),
            pages_freed: (old_count - new_count) as usize,
            old_page_count: old_count,
            new_page_count: new_count,
            relocations,
        })
    }

    /// Full vacuum: rebuild into a new file, then atomic rename.
    ///
    /// Copies all non-free pages contiguously into `.hora.tmp`, then
    /// renames over the original. Safe against crashes (original stays
    /// intact until rename succeeds).
    pub fn full_vacuum(&mut self) -> Result<CompactStats> {
        self.checkpoint()?;

        let old_count = self.alloc.page_count();
        let page_size = self.alloc.page_size();
        let mut new_alloc = PageAllocator::new(page_size);
        let mut relocations = Vec::new();

        for old_num in 1..old_count {
            let page_data = self.alloc.read_page(old_num)?;
            if let Some(hdr) = super::page::PageHeader::read_from(page_data) {
                if hdr.page_type == PageType::Free {
                    continue;
                }
            }
            let new_num = new_alloc.push_raw_page(page_data.to_vec());
            if old_num != new_num {
                relocations.push((old_num, new_num));
            }
        }

        // Write file header into page 0
        let fh = FileHeader {
            page_count: new_alloc.page_count(),
            ..FileHeader::new(page_size as u32)
        };
        fh.write_to(new_alloc.write_page(0)?);

        // Write to temp file
        let tmp_path = self.path.with_extension("tmp");
        write_pages_to_file(&new_alloc, &tmp_path).map_err(|_| HoraError::InvalidFile {
            reason: "cannot write temp file during full vacuum",
        })?;

        // Atomic rename
        std::fs::rename(&tmp_path, &self.path).map_err(|_| HoraError::InvalidFile {
            reason: "cannot rename temp file during full vacuum",
        })?;

        let new_count = new_alloc.page_count();
        self.alloc = new_alloc;

        Ok(CompactStats {
            pages_relocated: relocations.len(),
            pages_freed: (old_count - new_count) as usize,
            old_page_count: old_count,
            new_page_count: new_count,
            relocations,
        })
    }
}

/// Statistics from a compaction operation.
pub struct CompactStats {
    /// Number of pages that were moved to a new location.
    pub pages_relocated: usize,
    /// Number of free pages removed (old_count - new_count).
    pub pages_freed: usize,
    /// Page count before compaction.
    pub old_page_count: u32,
    /// Page count after compaction.
    pub new_page_count: u32,
    /// List of (old_page, new_page) relocations.
    pub relocations: Vec<(u32, u32)>,
}

impl Drop for Database {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(lock_path(&self.path));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::embedded::page::{PageType, DEFAULT_PAGE_SIZE, PAGE_HEADER_SIZE};

    #[test]
    fn test_file_header_roundtrip() {
        let fh = FileHeader {
            page_count: 42,
            freelist_page: 5,
            freelist_count: 3,
            ..FileHeader::new(4096)
        };
        let mut buf = [0u8; 32];
        fh.write_to(&mut buf);

        let decoded = FileHeader::read_from(&buf).unwrap();
        assert_eq!(decoded.magic, *b"HORA");
        assert_eq!(decoded.version, 1);
        assert_eq!(decoded.page_size, 4096);
        assert_eq!(decoded.page_count, 42);
        assert_eq!(decoded.freelist_page, 5);
        assert_eq!(decoded.freelist_count, 3);
    }

    #[test]
    fn test_file_header_bad_magic() {
        let mut buf = [0u8; 32];
        buf[0..4].copy_from_slice(b"NOPE");
        assert!(FileHeader::read_from(&buf).is_err());
    }

    #[test]
    fn test_file_header_bad_checksum() {
        let fh = FileHeader::new(4096);
        let mut buf = [0u8; 32];
        fh.write_to(&mut buf);
        buf[10] = 0xFF; // corrupt page_count
        assert!(FileHeader::read_from(&buf).is_err());
    }

    #[test]
    fn test_create_new_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        let db = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        assert!(path.exists());
        assert_eq!(db.alloc().page_count(), 1); // header page only

        // Verify file header
        let data = std::fs::read(&path).unwrap();
        let fh = FileHeader::read_from(&data).unwrap();
        assert_eq!(fh.page_size, DEFAULT_PAGE_SIZE as u32);
        assert_eq!(fh.page_count, 1);
    }

    #[test]
    fn test_checkpoint_persists_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        {
            let mut db = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
            let p = db.alloc_mut().alloc_page(PageType::EntityLeaf);
            db.alloc_mut().write_page(p).unwrap()[PAGE_HEADER_SIZE] = 0xAA;

            // Write through WAL
            let page_data = db.alloc().read_page(p).unwrap().to_vec();
            db.write_frame(p, page_data);

            db.checkpoint().unwrap();
        } // Drop releases lock

        // Reopen — data should persist
        let db2 = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        assert_eq!(db2.alloc().page_count(), 2);
        assert_eq!(db2.alloc().read_page(1).unwrap()[PAGE_HEADER_SIZE], 0xAA);
    }

    #[test]
    fn test_wal_recovery_replays_frames() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        {
            let mut db = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
            db.checkpoint().unwrap(); // clean baseline

            // Allocate and write a page
            let p = db.alloc_mut().alloc_page(PageType::EntityLeaf);
            db.alloc_mut().write_page(p).unwrap()[PAGE_HEADER_SIZE] = 0xBB;

            // Log to WAL and flush (but do NOT checkpoint)
            let page_data = db.alloc().read_page(p).unwrap().to_vec();
            db.write_frame(p, page_data);
            db.flush_wal().unwrap();

            // Simulate crash: WAL persisted, main file NOT updated
        }

        // Reopen → WAL should be replayed
        let db2 = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        assert_eq!(db2.alloc().read_page(1).unwrap()[PAGE_HEADER_SIZE], 0xBB);
        // WAL file should be removed after recovery
        assert!(!wal_path(&path).exists());
    }

    #[test]
    fn test_partial_wal_frame_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        {
            let mut db = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
            db.checkpoint().unwrap();

            // Write two frames via WAL
            let p1 = db.alloc_mut().alloc_page(PageType::EntityLeaf);
            db.alloc_mut().write_page(p1).unwrap()[PAGE_HEADER_SIZE] = 0x11;
            let data1 = db.alloc().read_page(p1).unwrap().to_vec();
            db.write_frame(p1, data1);

            let p2 = db.alloc_mut().alloc_page(PageType::EntityLeaf);
            db.alloc_mut().write_page(p2).unwrap()[PAGE_HEADER_SIZE] = 0x22;
            let data2 = db.alloc().read_page(p2).unwrap().to_vec();
            db.write_frame(p2, data2);

            db.flush_wal().unwrap();
        }

        // Truncate WAL file to simulate crash mid-write of frame 2
        let wal_p = wal_path(&path);
        let wal_data = std::fs::read(&wal_p).unwrap();
        let frame_size = WalFrame::HEADER_SIZE + DEFAULT_PAGE_SIZE;
        // Keep header (26) + full frame 1, truncate frame 2 by half
        let truncated_len = 26 + frame_size + frame_size / 2;
        std::fs::write(&wal_p, &wal_data[..truncated_len]).unwrap();

        // Reopen — only frame 1 should be recovered
        let db2 = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        assert_eq!(db2.alloc().read_page(1).unwrap()[PAGE_HEADER_SIZE], 0x11);
        // Page 2 should NOT have the WAL data (frame was partial)
        // It may or may not exist depending on whether allocator extended
    }

    #[test]
    fn test_corrupted_wal_frame_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        {
            let mut db = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
            db.checkpoint().unwrap();

            let p = db.alloc_mut().alloc_page(PageType::EntityLeaf);
            db.alloc_mut().write_page(p).unwrap()[PAGE_HEADER_SIZE] = 0xCC;
            let page_data = db.alloc().read_page(p).unwrap().to_vec();
            db.write_frame(p, page_data);
            db.flush_wal().unwrap();
        }

        // Corrupt the WAL frame data (byte after the frame header)
        let wal_p = wal_path(&path);
        let mut wal_data = std::fs::read(&wal_p).unwrap();
        // WAL header = 26 bytes, frame header = 20 bytes, then page data
        wal_data[26 + WalFrame::HEADER_SIZE + PAGE_HEADER_SIZE] = 0xFF;
        std::fs::write(&wal_p, &wal_data).unwrap();

        // Reopen — corrupted frame should be skipped
        let db2 = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        // Page 1 should NOT have 0xCC (frame was corrupted)
        let page = db2.alloc().read_page(1);
        if let Ok(p) = page {
            assert_ne!(p[PAGE_HEADER_SIZE], 0xCC);
        }
    }

    #[test]
    fn test_no_wal_opens_normally() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        {
            let mut db = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
            let p = db.alloc_mut().alloc_page(PageType::EntityLeaf);
            db.alloc_mut().write_page(p).unwrap()[PAGE_HEADER_SIZE] = 0xDD;

            let page_data = db.alloc().read_page(p).unwrap().to_vec();
            db.write_frame(p, page_data);
            db.checkpoint().unwrap();
        }

        // Verify no WAL file
        assert!(!wal_path(&path).exists());

        // Reopen normally
        let db2 = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        assert_eq!(db2.alloc().page_count(), 2);
        assert_eq!(db2.alloc().read_page(1).unwrap()[PAGE_HEADER_SIZE], 0xDD);
    }

    #[test]
    fn test_invalid_magic_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        // Write garbage
        std::fs::write(&path, vec![0u8; DEFAULT_PAGE_SIZE]).unwrap();

        let result = Database::open(&path, DEFAULT_PAGE_SIZE);
        match result {
            Err(HoraError::InvalidFile { reason }) => {
                assert!(reason.contains("magic"));
            }
            _ => panic!("expected InvalidFile error with magic"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_write_lock_prevents_double_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        let _db1 = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();

        // Second open should fail (same PID, but lock exists)
        // Note: since it's the same process, is_process_alive returns true
        let result = Database::open(&path, DEFAULT_PAGE_SIZE);
        assert!(result.is_err());
    }

    #[test]
    fn test_stale_lock_with_dead_pid_is_recovered() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        // Simulate a lock file left behind by a crashed process (PID unlikely to be alive).
        let lock = lock_path(&path);
        std::fs::write(&lock, b"999999999").unwrap();

        // acquire_lock should detect the dead PID, remove the stale lock, and
        // atomically recreate it rather than failing.
        let db = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        assert!(lock.exists());
        let pid_str = std::fs::read_to_string(&lock).unwrap();
        assert_eq!(pid_str.trim(), std::process::id().to_string());
        drop(db);
        assert!(!lock.exists());
    }

    #[test]
    fn test_create_lock_file_fails_if_already_present() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("test.lock");

        // First creation succeeds.
        Database::create_lock_file(&lock).unwrap();

        // A second attempt must fail atomically (O_EXCL semantics), proving
        // there is no exists()+write() TOCTOU window.
        let err = Database::create_lock_file(&lock).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn test_lock_released_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        {
            let _db = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
            assert!(lock_path(&path).exists());
        } // _db dropped → lock released

        assert!(!lock_path(&path).exists());

        // Should be able to open again
        let _db2 = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
    }

    #[test]
    fn test_wal_recovery_extends_allocator() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        {
            let mut db = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
            db.checkpoint().unwrap(); // 1 page on disk

            // Write to page 5 via WAL (way beyond current page count)
            let mut page_data = vec![0u8; DEFAULT_PAGE_SIZE];
            page_data[PAGE_HEADER_SIZE] = 0xEE;
            db.wal_mut().write_frame(5, 6, page_data);
            db.flush_wal().unwrap();
        }

        // Reopen — allocator should have grown to include page 5
        let db2 = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        assert!(db2.alloc().page_count() > 5);
        assert_eq!(db2.alloc().read_page(5).unwrap()[PAGE_HEADER_SIZE], 0xEE);
    }

    #[test]
    fn test_read_page_wal_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        let mut db = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        let p = db.alloc_mut().alloc_page(PageType::EntityLeaf);
        db.alloc_mut().write_page(p).unwrap()[PAGE_HEADER_SIZE] = 0x01;

        // Write different data via WAL
        let mut wal_data = vec![0u8; DEFAULT_PAGE_SIZE];
        wal_data[PAGE_HEADER_SIZE] = 0x02;
        db.write_frame(p, wal_data);

        // read_page should return WAL version
        let read = db.read_page(p).unwrap();
        assert_eq!(read[PAGE_HEADER_SIZE], 0x02);
    }

    #[test]
    fn test_database_compact_reduces_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        {
            let mut db = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();

            // Alloc 6 pages with data
            for i in 0u8..6 {
                let p = db.alloc_mut().alloc_page(PageType::EntityLeaf);
                db.alloc_mut().write_page(p).unwrap()[PAGE_HEADER_SIZE] = 0x10 + i;
            }
            // Free pages 2 and 4 (50% holes in the middle)
            db.alloc_mut().free_page(2).unwrap();
            db.alloc_mut().free_page(4).unwrap();

            let stats = db.compact().unwrap();
            assert!(stats.pages_freed > 0);
            assert!(stats.new_page_count < stats.old_page_count);
            assert_eq!(db.alloc().freelist_count(), 0);
        }

        // Reopen — should still be valid
        let db2 = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        assert_eq!(db2.alloc().page_count(), 5); // header + 4 used
    }

    #[test]
    fn test_database_compact_data_intact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        {
            let mut db = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();

            let p1 = db.alloc_mut().alloc_page(PageType::EntityLeaf);
            db.alloc_mut().write_page(p1).unwrap()[PAGE_HEADER_SIZE] = 0xAA;
            let p2 = db.alloc_mut().alloc_page(PageType::EdgeData);
            db.alloc_mut().write_page(p2).unwrap()[PAGE_HEADER_SIZE] = 0xBB;
            let p3 = db.alloc_mut().alloc_page(PageType::VectorData);
            db.alloc_mut().write_page(p3).unwrap()[PAGE_HEADER_SIZE] = 0xCC;

            // Free p2 (middle hole)
            db.alloc_mut().free_page(p2).unwrap();
            db.compact().unwrap();
        }

        // Reopen and verify all surviving data
        let db2 = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        let mut data_bytes: Vec<u8> = Vec::new();
        for i in 1..db2.alloc().page_count() {
            data_bytes.push(db2.alloc().read_page(i).unwrap()[PAGE_HEADER_SIZE]);
        }
        data_bytes.sort();
        assert_eq!(data_bytes, vec![0xAA, 0xCC]);
    }

    #[test]
    fn test_database_full_vacuum() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        {
            let mut db = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();

            for i in 0u8..5 {
                let p = db.alloc_mut().alloc_page(PageType::EntityLeaf);
                db.alloc_mut().write_page(p).unwrap()[PAGE_HEADER_SIZE] = 0x50 + i;
            }
            // Free pages 1, 3 (scattered holes)
            db.alloc_mut().free_page(1).unwrap();
            db.alloc_mut().free_page(3).unwrap();

            let stats = db.full_vacuum().unwrap();
            assert_eq!(stats.pages_freed, 2);
            assert_eq!(stats.new_page_count, 4); // header + 3 used
            assert_eq!(db.alloc().freelist_count(), 0);
        }

        // Reopen after full vacuum
        let db2 = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        assert_eq!(db2.alloc().page_count(), 4);

        // Verify data (3 surviving pages)
        let mut data_bytes: Vec<u8> = Vec::new();
        for i in 1..db2.alloc().page_count() {
            data_bytes.push(db2.alloc().read_page(i).unwrap()[PAGE_HEADER_SIZE]);
        }
        data_bytes.sort();
        assert_eq!(data_bytes, vec![0x51, 0x53, 0x54]);
    }

    // ── Transaction tests ────────────────────────────────

    #[test]
    fn test_database_begin_commit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        {
            let mut db = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();

            db.begin_transaction().unwrap();
            assert!(db.in_transaction());

            let p = db.alloc_mut().alloc_page(PageType::EntityLeaf);
            db.alloc_mut().write_page(p).unwrap()[PAGE_HEADER_SIZE] = 0xAA;
            let page_data = db.alloc().read_page(p).unwrap().to_vec();
            db.write_frame(p, page_data);

            db.commit().unwrap();
            assert!(!db.in_transaction());

            db.checkpoint().unwrap();
        }

        // Reopen — committed data persists
        let db2 = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        assert_eq!(db2.alloc().read_page(1).unwrap()[PAGE_HEADER_SIZE], 0xAA);
    }

    #[test]
    fn test_database_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        let mut db = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();

        // Write committed data first
        let p1 = db.alloc_mut().alloc_page(PageType::EntityLeaf);
        db.alloc_mut().write_page(p1).unwrap()[PAGE_HEADER_SIZE] = 0x11;
        let data1 = db.alloc().read_page(p1).unwrap().to_vec();
        db.write_frame(p1, data1);
        db.checkpoint().unwrap();

        // Begin transaction, write, rollback
        db.begin_transaction().unwrap();
        let p2 = db.alloc_mut().alloc_page(PageType::EntityLeaf);
        db.alloc_mut().write_page(p2).unwrap()[PAGE_HEADER_SIZE] = 0x22;
        let data2 = db.alloc().read_page(p2).unwrap().to_vec();
        db.write_frame(p2, data2);

        db.rollback().unwrap();

        // Page 2's WAL frame is gone
        assert!(db.wal().read_page(p2).is_none());
    }

    #[test]
    fn test_checkpoint_blocked_during_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        let mut db = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        db.begin_transaction().unwrap();

        let result = db.checkpoint();
        assert!(result.is_err());
    }

    #[test]
    fn test_flush_only_committed_frames() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        {
            let mut db = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();

            // Write committed data
            let p1 = db.alloc_mut().alloc_page(PageType::EntityLeaf);
            db.alloc_mut().write_page(p1).unwrap()[PAGE_HEADER_SIZE] = 0xAA;
            let data1 = db.alloc().read_page(p1).unwrap().to_vec();
            db.write_frame(p1, data1);

            // Begin transaction, write uncommitted data
            db.begin_transaction().unwrap();
            let p2 = db.alloc_mut().alloc_page(PageType::EntityLeaf);
            db.alloc_mut().write_page(p2).unwrap()[PAGE_HEADER_SIZE] = 0xBB;
            let data2 = db.alloc().read_page(p2).unwrap().to_vec();
            db.write_frame(p2, data2);

            // Flush WAL (only committed frames should be written)
            db.flush_wal().unwrap();
            // Simulate crash (don't commit, don't checkpoint)
        }

        // Reopen — only committed frame should be recovered
        let db2 = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        assert_eq!(db2.alloc().read_page(1).unwrap()[PAGE_HEADER_SIZE], 0xAA);
        // Page 2 should NOT have uncommitted data
        if db2.alloc().page_count() > 2 {
            assert_ne!(db2.alloc().read_page(2).unwrap()[PAGE_HEADER_SIZE], 0xBB);
        }
    }

    #[test]
    fn test_nested_transaction_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        let mut db = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        db.begin_transaction().unwrap();
        assert!(db.begin_transaction().is_err());
    }

    #[test]
    fn test_commit_without_transaction_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        let mut db = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        assert!(db.commit().is_err());
    }

    #[test]
    fn test_rollback_without_transaction_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        let mut db = Database::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        assert!(db.rollback().is_err());
    }
}
