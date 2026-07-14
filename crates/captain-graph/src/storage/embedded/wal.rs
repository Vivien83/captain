//! Write-Ahead Log (WAL) for crash-safe page writes.
//!
//! All page modifications go through the WAL first. The main page store
//! is only updated during checkpoint. This ensures atomicity: if a crash
//! occurs mid-write, only complete WAL frames (verified by CRC32) are
//! replayed on recovery.
//!
//! Inspired by SQLite's WAL mode.

use std::collections::HashMap;

use super::page::{crc32, PageAllocator};

/// WAL magic bytes.
const WAL_MAGIC: [u8; 4] = *b"WLOG";

/// WAL format version.
const WAL_VERSION: u16 = 1;

/// Default auto-checkpoint threshold (number of frames).
const DEFAULT_AUTO_CHECKPOINT: usize = 1000;

/// WAL header (in-memory representation).
#[derive(Debug, Clone)]
pub struct WalHeader {
    /// Magic bytes identifying a valid WAL file (`"WLOG"`).
    pub magic: [u8; 4],
    /// WAL format version number.
    pub version: u16,
    /// Page size in bytes this WAL was written for.
    pub page_size: u32,
    /// Monotonically increasing checkpoint sequence number.
    pub checkpoint_seq: u64,
    /// Per-checkpoint salt used to detect stale frames from previous eras.
    pub salt: [u8; 8],
}

impl WalHeader {
    /// Create a new WAL header for the given page size.
    pub fn new(page_size: u32) -> Self {
        Self {
            magic: WAL_MAGIC,
            version: WAL_VERSION,
            page_size,
            checkpoint_seq: 0,
            salt: Self::generate_salt(0),
        }
    }

    /// Deterministic salt from checkpoint sequence (for reproducible tests).
    /// In production, this would use random bytes.
    fn generate_salt(seq: u64) -> [u8; 8] {
        // Mix the sequence number into a salt via simple hash
        let mut salt = [0u8; 8];
        let mixed = seq.wrapping_mul(0x517cc1b727220a95);
        salt.copy_from_slice(&mixed.to_le_bytes());
        salt
    }

    /// Serialize the header to bytes (26 bytes).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(26);
        buf.extend_from_slice(&self.magic);
        buf.extend_from_slice(&self.version.to_le_bytes());
        buf.extend_from_slice(&self.page_size.to_le_bytes());
        buf.extend_from_slice(&self.checkpoint_seq.to_le_bytes());
        buf.extend_from_slice(&self.salt);
        buf
    }

    /// Deserialize from bytes. Returns None if magic/version mismatch.
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 26 {
            return None;
        }
        let magic: [u8; 4] = data[0..4].try_into().ok()?;
        if magic != WAL_MAGIC {
            return None;
        }
        let version = u16::from_le_bytes([data[4], data[5]]);
        if version != WAL_VERSION {
            return None;
        }
        let page_size = u32::from_le_bytes([data[6], data[7], data[8], data[9]]);
        let checkpoint_seq = u64::from_le_bytes(data[10..18].try_into().ok()?);
        let salt: [u8; 8] = data[18..26].try_into().ok()?;

        Some(Self {
            magic,
            version,
            page_size,
            checkpoint_seq,
            salt,
        })
    }
}

/// A single WAL frame representing a page write.
#[derive(Debug, Clone)]
pub struct WalFrame {
    /// Page number this frame applies to.
    pub page_number: u32,
    /// Database page count at the time this frame was written.
    pub db_size: u32,
    /// Salt copied from the WAL header at write time (for stale-frame detection).
    pub salt: [u8; 8],
    /// CRC32 checksum of the frame header fields and page data.
    pub checksum: u32,
    /// Raw page data (exactly `page_size` bytes).
    pub data: Vec<u8>,
}

impl WalFrame {
    /// Frame header size (without page data): 4 + 4 + 8 + 4 = 20 bytes.
    pub const HEADER_SIZE: usize = 20;

    /// Create a new frame with computed checksum.
    pub fn new(page_number: u32, db_size: u32, salt: [u8; 8], data: Vec<u8>) -> Self {
        let checksum = Self::compute_checksum(page_number, db_size, &salt, &data);
        Self {
            page_number,
            db_size,
            salt,
            checksum,
            data,
        }
    }

    /// Compute CRC32 over frame header fields + page data.
    fn compute_checksum(page_number: u32, db_size: u32, salt: &[u8; 8], data: &[u8]) -> u32 {
        let mut buf = Vec::with_capacity(16 + data.len());
        buf.extend_from_slice(&page_number.to_le_bytes());
        buf.extend_from_slice(&db_size.to_le_bytes());
        buf.extend_from_slice(salt);
        buf.extend_from_slice(data);
        crc32(&buf)
    }

    /// Verify the frame's checksum.
    pub fn verify(&self) -> bool {
        let expected =
            Self::compute_checksum(self.page_number, self.db_size, &self.salt, &self.data);
        self.checksum == expected
    }

    /// Serialize to bytes (header + data).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(Self::HEADER_SIZE + self.data.len());
        buf.extend_from_slice(&self.page_number.to_le_bytes());
        buf.extend_from_slice(&self.db_size.to_le_bytes());
        buf.extend_from_slice(&self.salt);
        buf.extend_from_slice(&self.checksum.to_le_bytes());
        buf.extend_from_slice(&self.data);
        buf
    }

    /// Deserialize from bytes. Returns None if too short.
    pub fn from_bytes(data: &[u8], page_size: usize) -> Option<Self> {
        if data.len() < Self::HEADER_SIZE + page_size {
            return None;
        }
        let page_number = u32::from_le_bytes(data[0..4].try_into().ok()?);
        let db_size = u32::from_le_bytes(data[4..8].try_into().ok()?);
        let salt: [u8; 8] = data[8..16].try_into().ok()?;
        let checksum = u32::from_le_bytes(data[16..20].try_into().ok()?);
        let page_data = data[20..20 + page_size].to_vec();

        Some(Self {
            page_number,
            db_size,
            salt,
            checksum,
            data: page_data,
        })
    }
}

/// In-memory Write-Ahead Log.
pub struct WriteAheadLog {
    header: WalHeader,
    /// All frames in append order.
    frames: Vec<WalFrame>,
    /// Index: page_number → index into `frames` (last write wins).
    index: HashMap<u32, usize>,
    /// Auto-checkpoint threshold.
    auto_checkpoint_threshold: usize,
    /// Number of committed frames (0..committed_count are durable).
    committed_count: usize,
    /// Savepoint: frame count when transaction began. None = no active transaction.
    savepoint: Option<usize>,
}

impl WriteAheadLog {
    /// Create a new empty WAL.
    pub fn new(page_size: u32) -> Self {
        Self {
            header: WalHeader::new(page_size),
            frames: Vec::new(),
            index: HashMap::new(),
            auto_checkpoint_threshold: DEFAULT_AUTO_CHECKPOINT,
            committed_count: 0,
            savepoint: None,
        }
    }

    /// Set the auto-checkpoint threshold (0 = disabled).
    pub fn set_auto_checkpoint(&mut self, threshold: usize) {
        self.auto_checkpoint_threshold = threshold;
    }

    /// Number of frames in the WAL.
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Current checkpoint sequence number.
    pub fn checkpoint_seq(&self) -> u64 {
        self.header.checkpoint_seq
    }

    /// Access the WAL header.
    pub fn header(&self) -> &WalHeader {
        &self.header
    }

    /// Access all frames in the WAL.
    pub fn frames(&self) -> &[WalFrame] {
        &self.frames
    }

    /// Write a page to the WAL.
    ///
    /// Returns `true` if auto-checkpoint threshold was reached (caller
    /// should call `checkpoint()` soon).
    pub fn write_frame(&mut self, page_number: u32, db_size: u32, data: Vec<u8>) -> bool {
        let frame = WalFrame::new(page_number, db_size, self.header.salt, data);
        let idx = self.frames.len();
        self.frames.push(frame);
        self.index.insert(page_number, idx);

        // Auto-commit when not in a transaction
        if self.savepoint.is_none() {
            self.committed_count = self.frames.len();
        }

        self.auto_checkpoint_threshold > 0 && self.committed_count >= self.auto_checkpoint_threshold
    }

    /// Read a page from the WAL. Returns None if the page is not in the WAL.
    ///
    /// Returns the latest write (including uncommitted transaction data for the
    /// current writer).
    pub fn read_page(&self, page_number: u32) -> Option<&[u8]> {
        self.index
            .get(&page_number)
            .map(|&idx| self.frames[idx].data.as_slice())
    }

    // ── Transactions ──────────────────────────────────────

    /// Begin a transaction. Returns false if already in a transaction (no nesting).
    pub fn begin_transaction(&mut self) -> bool {
        if self.savepoint.is_some() {
            return false;
        }
        self.savepoint = Some(self.frames.len());
        true
    }

    /// Commit the current transaction. Returns false if not in a transaction.
    pub fn commit_transaction(&mut self) -> bool {
        if self.savepoint.is_none() {
            return false;
        }
        self.committed_count = self.frames.len();
        self.savepoint = None;
        true
    }

    /// Rollback the current transaction. Returns false if not in a transaction.
    pub fn rollback_transaction(&mut self) -> bool {
        let savepoint = match self.savepoint.take() {
            Some(sp) => sp,
            None => return false,
        };
        self.frames.truncate(savepoint);
        // Rebuild index for remaining frames
        self.index.clear();
        for (i, frame) in self.frames.iter().enumerate() {
            self.index.insert(frame.page_number, i);
        }
        true
    }

    /// Whether a transaction is currently active.
    pub fn in_transaction(&self) -> bool {
        self.savepoint.is_some()
    }

    /// Access only the committed frames (for flushing to disk).
    pub fn committed_frames(&self) -> &[WalFrame] {
        &self.frames[..self.committed_count]
    }

    /// Checkpoint: replay all valid WAL frames into the PageAllocator, then clear.
    ///
    /// Frames with bad checksums or mismatched salt are skipped.
    /// Returns the number of pages written to the allocator.
    pub fn checkpoint(&mut self, alloc: &mut PageAllocator) -> usize {
        // Cannot checkpoint during an active transaction
        if self.savepoint.is_some() {
            return 0;
        }

        let mut written = 0;

        // Process frames in order (last write to each page wins via the alloc)
        for frame in &self.frames {
            // Verify integrity
            if !frame.verify() {
                continue;
            }
            // Verify salt matches current WAL header
            if frame.salt != self.header.salt {
                continue;
            }

            let page_num = frame.page_number as usize;

            // Ensure allocator has enough pages
            while alloc.page_count() as usize <= page_num {
                alloc.alloc_page(super::page::PageType::Free);
            }

            // Write frame data to the allocator page
            if let Ok(page) = alloc.write_page(frame.page_number) {
                let copy_len = frame.data.len().min(page.len());
                page[..copy_len].copy_from_slice(&frame.data[..copy_len]);
                written += 1;
            }
        }

        // Clear the WAL and rotate salt
        self.frames.clear();
        self.index.clear();
        self.committed_count = 0;
        self.header.checkpoint_seq += 1;
        self.header.salt = WalHeader::generate_salt(self.header.checkpoint_seq);

        written
    }

    /// Scan raw frames, verifying checksums. Returns (valid_count, invalid_count).
    pub fn verify_all_frames(&self) -> (usize, usize) {
        let mut valid = 0;
        let mut invalid = 0;
        for frame in &self.frames {
            if frame.verify() && frame.salt == self.header.salt {
                valid += 1;
            } else {
                invalid += 1;
            }
        }
        (valid, invalid)
    }

    /// Inject a raw frame (for testing crash recovery with corrupted frames).
    #[cfg(test)]
    pub fn inject_raw_frame(&mut self, frame: WalFrame) {
        let idx = self.frames.len();
        let page_number = frame.page_number;
        self.frames.push(frame);
        self.index.insert(page_number, idx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::embedded::page::{
        PageAllocator, PageType, DEFAULT_PAGE_SIZE, PAGE_HEADER_SIZE,
    };

    fn make_page_data(byte: u8) -> Vec<u8> {
        let mut data = vec![0u8; DEFAULT_PAGE_SIZE];
        // Write a recognizable pattern after the page header
        data[PAGE_HEADER_SIZE] = byte;
        data
    }

    #[test]
    fn test_wal_header_roundtrip() {
        let header = WalHeader::new(4096);
        let bytes = header.to_bytes();
        let decoded = WalHeader::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.magic, *b"WLOG");
        assert_eq!(decoded.version, 1);
        assert_eq!(decoded.page_size, 4096);
        assert_eq!(decoded.checkpoint_seq, 0);
    }

    #[test]
    fn test_wal_header_bad_magic() {
        let mut bytes = WalHeader::new(4096).to_bytes();
        bytes[0] = b'X';
        assert!(WalHeader::from_bytes(&bytes).is_none());
    }

    #[test]
    fn test_frame_checksum_verify() {
        let frame = WalFrame::new(1, 10, [0; 8], vec![1, 2, 3]);
        assert!(frame.verify());
    }

    #[test]
    fn test_frame_corrupted_checksum() {
        let mut frame = WalFrame::new(1, 10, [0; 8], vec![1, 2, 3]);
        frame.data[0] = 99; // corrupt
        assert!(!frame.verify());
    }

    #[test]
    fn test_frame_roundtrip() {
        let data = make_page_data(0xAB);
        let frame = WalFrame::new(5, 20, [1, 2, 3, 4, 5, 6, 7, 8], data.clone());
        let bytes = frame.to_bytes();
        let decoded = WalFrame::from_bytes(&bytes, DEFAULT_PAGE_SIZE).unwrap();
        assert_eq!(decoded.page_number, 5);
        assert_eq!(decoded.db_size, 20);
        assert_eq!(decoded.data, data);
        assert!(decoded.verify());
    }

    #[test]
    fn test_write_and_read_from_wal() {
        let mut wal = WriteAheadLog::new(DEFAULT_PAGE_SIZE as u32);
        let data = make_page_data(0x42);
        wal.write_frame(3, 10, data.clone());

        let read = wal.read_page(3).unwrap();
        assert_eq!(read[PAGE_HEADER_SIZE], 0x42);
        assert!(wal.read_page(99).is_none());
    }

    #[test]
    fn test_wal_last_write_wins() {
        let mut wal = WriteAheadLog::new(DEFAULT_PAGE_SIZE as u32);
        wal.write_frame(1, 10, make_page_data(0x01));
        wal.write_frame(1, 10, make_page_data(0x02));
        wal.write_frame(1, 10, make_page_data(0x03));

        let read = wal.read_page(1).unwrap();
        assert_eq!(read[PAGE_HEADER_SIZE], 0x03);
        assert_eq!(wal.frame_count(), 3);
    }

    #[test]
    fn test_checkpoint_writes_to_allocator() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        // Allocate page 1
        alloc.alloc_page(PageType::EntityLeaf);

        let mut wal = WriteAheadLog::new(DEFAULT_PAGE_SIZE as u32);
        wal.write_frame(1, 2, make_page_data(0xBE));

        let written = wal.checkpoint(&mut alloc);
        assert_eq!(written, 1);

        // WAL should be empty after checkpoint
        assert_eq!(wal.frame_count(), 0);
        assert!(wal.read_page(1).is_none());

        // Data should now be in the allocator
        let page = alloc.read_page(1).unwrap();
        assert_eq!(page[PAGE_HEADER_SIZE], 0xBE);
    }

    #[test]
    fn test_checkpoint_clears_wal_and_rotates_salt() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        alloc.alloc_page(PageType::Free);

        let mut wal = WriteAheadLog::new(DEFAULT_PAGE_SIZE as u32);
        let old_salt = wal.header.salt;

        wal.write_frame(1, 2, make_page_data(0x01));
        wal.checkpoint(&mut alloc);

        assert_eq!(wal.frame_count(), 0);
        assert_eq!(wal.checkpoint_seq(), 1);
        assert_ne!(wal.header.salt, old_salt);
    }

    #[test]
    fn test_corrupted_frame_skipped_on_checkpoint() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        alloc.alloc_page(PageType::EntityLeaf);
        alloc.alloc_page(PageType::EntityLeaf);

        let mut wal = WriteAheadLog::new(DEFAULT_PAGE_SIZE as u32);

        // Good frame
        wal.write_frame(1, 3, make_page_data(0xAA));

        // Inject a corrupted frame
        let mut bad_frame = WalFrame::new(2, 3, wal.header.salt, make_page_data(0xBB));
        bad_frame.data[0] = 0xFF; // corrupt data → checksum mismatch
        wal.inject_raw_frame(bad_frame);

        let written = wal.checkpoint(&mut alloc);
        // Only the good frame should be written
        assert_eq!(written, 1);

        let p1 = alloc.read_page(1).unwrap();
        assert_eq!(p1[PAGE_HEADER_SIZE], 0xAA);
        // Page 2 should NOT have been overwritten
        let p2 = alloc.read_page(2).unwrap();
        assert_ne!(p2[PAGE_HEADER_SIZE], 0xBB);
    }

    #[test]
    fn test_stale_salt_frame_skipped() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        alloc.alloc_page(PageType::EntityLeaf);

        let mut wal = WriteAheadLog::new(DEFAULT_PAGE_SIZE as u32);

        // Inject a frame with wrong salt (from a previous checkpoint era)
        let stale_frame = WalFrame::new(1, 2, [0xFF; 8], make_page_data(0xDD));
        wal.inject_raw_frame(stale_frame);

        let written = wal.checkpoint(&mut alloc);
        assert_eq!(written, 0); // stale frame skipped
    }

    #[test]
    fn test_auto_checkpoint_threshold() {
        let mut wal = WriteAheadLog::new(DEFAULT_PAGE_SIZE as u32);
        wal.set_auto_checkpoint(3);

        assert!(!wal.write_frame(1, 5, make_page_data(0x01)));
        assert!(!wal.write_frame(2, 5, make_page_data(0x02)));
        // Third frame hits threshold
        assert!(wal.write_frame(3, 5, make_page_data(0x03)));
    }

    #[test]
    fn test_verify_all_frames() {
        let mut wal = WriteAheadLog::new(DEFAULT_PAGE_SIZE as u32);
        wal.write_frame(1, 5, make_page_data(0x01));
        wal.write_frame(2, 5, make_page_data(0x02));

        // Inject a bad frame
        let mut bad = WalFrame::new(3, 5, wal.header.salt, make_page_data(0x03));
        bad.data[10] = 0xFF;
        wal.inject_raw_frame(bad);

        let (valid, invalid) = wal.verify_all_frames();
        assert_eq!(valid, 2);
        assert_eq!(invalid, 1);
    }

    #[test]
    fn test_checkpoint_extends_allocator_if_needed() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        // Only page 0 exists
        assert_eq!(alloc.page_count(), 1);

        let mut wal = WriteAheadLog::new(DEFAULT_PAGE_SIZE as u32);
        // Write to page 5 (doesn't exist yet in allocator)
        wal.write_frame(5, 6, make_page_data(0xEE));

        let written = wal.checkpoint(&mut alloc);
        assert_eq!(written, 1);
        // Allocator should have grown to accommodate page 5
        assert!(alloc.page_count() > 5);
        let p5 = alloc.read_page(5).unwrap();
        assert_eq!(p5[PAGE_HEADER_SIZE], 0xEE);
    }

    // ── Transaction tests ────────────────────────────────

    #[test]
    fn test_begin_commit_transaction() {
        let mut wal = WriteAheadLog::new(DEFAULT_PAGE_SIZE as u32);

        assert!(wal.begin_transaction());
        assert!(wal.in_transaction());

        wal.write_frame(1, 5, make_page_data(0x01));
        wal.write_frame(2, 5, make_page_data(0x02));

        // Uncommitted: committed_count still 0
        assert_eq!(wal.committed_frames().len(), 0);
        assert_eq!(wal.frame_count(), 2);

        // Writer can still read uncommitted data
        assert_eq!(wal.read_page(1).unwrap()[PAGE_HEADER_SIZE], 0x01);

        assert!(wal.commit_transaction());
        assert!(!wal.in_transaction());
        assert_eq!(wal.committed_frames().len(), 2);
    }

    #[test]
    fn test_rollback_transaction() {
        let mut wal = WriteAheadLog::new(DEFAULT_PAGE_SIZE as u32);

        // Write a committed frame first (outside transaction)
        wal.write_frame(1, 5, make_page_data(0xAA));
        assert_eq!(wal.committed_frames().len(), 1);

        // Begin transaction, write, rollback
        assert!(wal.begin_transaction());
        wal.write_frame(2, 5, make_page_data(0xBB));
        wal.write_frame(1, 5, make_page_data(0xCC)); // overwrite page 1

        assert_eq!(wal.frame_count(), 3);
        // Read sees uncommitted overwrite
        assert_eq!(wal.read_page(1).unwrap()[PAGE_HEADER_SIZE], 0xCC);

        assert!(wal.rollback_transaction());
        assert!(!wal.in_transaction());

        // Back to 1 committed frame
        assert_eq!(wal.frame_count(), 1);
        assert_eq!(wal.committed_frames().len(), 1);
        // Page 1 back to original
        assert_eq!(wal.read_page(1).unwrap()[PAGE_HEADER_SIZE], 0xAA);
        // Page 2 gone
        assert!(wal.read_page(2).is_none());
    }

    #[test]
    fn test_nested_transaction_rejected() {
        let mut wal = WriteAheadLog::new(DEFAULT_PAGE_SIZE as u32);
        assert!(wal.begin_transaction());
        assert!(!wal.begin_transaction()); // nested → rejected
    }

    #[test]
    fn test_commit_without_transaction_rejected() {
        let mut wal = WriteAheadLog::new(DEFAULT_PAGE_SIZE as u32);
        assert!(!wal.commit_transaction());
    }

    #[test]
    fn test_rollback_without_transaction_rejected() {
        let mut wal = WriteAheadLog::new(DEFAULT_PAGE_SIZE as u32);
        assert!(!wal.rollback_transaction());
    }

    #[test]
    fn test_auto_commit_without_transaction() {
        let mut wal = WriteAheadLog::new(DEFAULT_PAGE_SIZE as u32);
        wal.write_frame(1, 5, make_page_data(0x01));
        wal.write_frame(2, 5, make_page_data(0x02));

        // Without transaction, every write auto-commits
        assert_eq!(wal.committed_frames().len(), 2);
        assert_eq!(wal.frame_count(), 2);
    }

    #[test]
    fn test_checkpoint_blocked_during_transaction() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        alloc.alloc_page(PageType::EntityLeaf);

        let mut wal = WriteAheadLog::new(DEFAULT_PAGE_SIZE as u32);
        wal.begin_transaction();
        wal.write_frame(1, 2, make_page_data(0x01));

        // Checkpoint returns 0 (blocked)
        let written = wal.checkpoint(&mut alloc);
        assert_eq!(written, 0);
        // Frames still there
        assert_eq!(wal.frame_count(), 1);
    }

    #[test]
    fn test_auto_checkpoint_skipped_in_transaction() {
        let mut wal = WriteAheadLog::new(DEFAULT_PAGE_SIZE as u32);
        wal.set_auto_checkpoint(2);

        // In transaction: committed_count stays 0, threshold not reached
        wal.begin_transaction();
        assert!(!wal.write_frame(1, 5, make_page_data(0x01)));
        assert!(!wal.write_frame(2, 5, make_page_data(0x02)));
        // 2 frames but 0 committed → no signal
        wal.commit_transaction();

        // Next write outside transaction: committed_count = 3 >= 2 → signal
        assert!(wal.write_frame(3, 5, make_page_data(0x03)));
    }
}
