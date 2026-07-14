//! Page allocator with freelist for the embedded storage engine.
//!
//! Each page is a fixed-size block (default 4096 bytes). Page 0 is the file
//! header. Data pages start at page 1. Free pages form a linked list.
//!
//! CRC32 integrity check on every page header (IEEE polynomial).

use crate::error::{HoraError, Result};

/// Default page size in bytes.
pub const DEFAULT_PAGE_SIZE: usize = 4096;

/// Page header size in bytes.
pub const PAGE_HEADER_SIZE: usize = 8;

/// Usable bytes per page (page_size - header).
pub const fn usable_bytes(page_size: usize) -> usize {
    page_size - PAGE_HEADER_SIZE
}

/// Maximum free page IDs stored in a single freelist page.
/// Layout: next_freelist_page(4) + count(2) + page_ids(count * 4)
pub const fn freelist_capacity(page_size: usize) -> usize {
    (usable_bytes(page_size) - 6) / 4
}

// ── Page types ────────────────────────────────────────────

/// Type tag stored in the first byte of every page header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PageType {
    /// Page is unallocated and available for reuse via the freelist.
    Free = 0,
    /// B+ tree leaf page holding entity records.
    EntityLeaf = 1,
    /// B+ tree interior (routing) page for the entity index.
    EntityInterior = 2,
    /// Page storing serialized edge (fact) records.
    EdgeData = 3,
    /// Interned string pool page for deduplicating long strings.
    StringPool = 4,
    /// Page storing raw float32 embedding vectors.
    VectorData = 5,
    /// BM25 inverted-index posting list page.
    Bm25Posting = 6,
    /// BM25 dictionary (term → posting list pointer) page.
    Bm25Dict = 7,
    /// Bi-temporal index page for time-range queries.
    TemporalIndex = 8,
    /// Column-store page for entity property values.
    PropertyColumn = 9,
    /// Page storing episode (interaction snapshot) records.
    EpisodeData = 10,
    /// Page storing ACT-R activation log entries.
    ActivationLog = 11,
    /// Overflow page for data that exceeds a single page.
    Overflow = 12,
}

impl PageType {
    /// Convert a raw byte into a `PageType`, returning `None` for unknown values.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Free),
            1 => Some(Self::EntityLeaf),
            2 => Some(Self::EntityInterior),
            3 => Some(Self::EdgeData),
            4 => Some(Self::StringPool),
            5 => Some(Self::VectorData),
            6 => Some(Self::Bm25Posting),
            7 => Some(Self::Bm25Dict),
            8 => Some(Self::TemporalIndex),
            9 => Some(Self::PropertyColumn),
            10 => Some(Self::EpisodeData),
            11 => Some(Self::ActivationLog),
            12 => Some(Self::Overflow),
            _ => None,
        }
    }
}

// ── Page header ───────────────────────────────────────────

/// 8-byte header at the start of every page.
///
/// ```text
/// [page_type: u8][flags: u8][item_count: u16][checksum: u32]
/// ```
#[derive(Debug, Clone, Copy)]
pub struct PageHeader {
    /// Type tag identifying the page's role (entity, edge, free, etc.).
    pub page_type: PageType,
    /// Reserved flags byte (currently unused, must be 0).
    pub flags: u8,
    /// Number of items stored in this page.
    pub item_count: u16,
    /// CRC32-IEEE checksum of the page payload (everything after the header).
    pub checksum: u32,
}

impl PageHeader {
    /// Encode the header into the first 8 bytes of `buf`.
    pub fn write_to(&self, buf: &mut [u8]) {
        buf[0] = self.page_type as u8;
        buf[1] = self.flags;
        buf[2..4].copy_from_slice(&self.item_count.to_le_bytes());
        buf[4..8].copy_from_slice(&self.checksum.to_le_bytes());
    }

    /// Decode a header from the first 8 bytes of `buf`.
    pub fn read_from(buf: &[u8]) -> Option<Self> {
        if buf.len() < PAGE_HEADER_SIZE {
            return None;
        }
        let page_type = PageType::from_u8(buf[0])?;
        let flags = buf[1];
        let item_count = u16::from_le_bytes([buf[2], buf[3]]);
        let checksum = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        Some(Self {
            page_type,
            flags,
            item_count,
            checksum,
        })
    }
}

// ── CRC32 (IEEE polynomial, zero-dep) ─────────────────────

/// CRC32 lookup table (IEEE 802.3 polynomial 0xEDB88320).
const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0u32;
    while i < 256 {
        let mut crc = i;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i as usize] = crc;
        i += 1;
    }
    table
};

/// Compute CRC32 checksum of a byte slice.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        let index = ((crc ^ byte as u32) & 0xFF) as usize;
        crc = (crc >> 8) ^ CRC32_TABLE[index];
    }
    crc ^ 0xFFFF_FFFF
}

// ── Page allocator ────────────────────────────────────────

/// In-memory page allocator with freelist.
///
/// Pages are stored as `Vec<u8>` in a flat vector. Page 0 is reserved
/// for the file header. The freelist is a linked chain of Free pages.
pub struct PageAllocator {
    page_size: usize,
    /// All pages (index = page number). Page 0 = header.
    pages: Vec<Vec<u8>>,
    /// Page number of the first freelist page (0 = no freelist).
    freelist_head: u32,
    /// Total number of free pages across all freelist pages.
    freelist_count: u32,
}

impl PageAllocator {
    /// Create a new allocator with a header page (page 0).
    pub fn new(page_size: usize) -> Self {
        let header_page = vec![0u8; page_size];
        Self {
            page_size,
            pages: vec![header_page],
            freelist_head: 0,
            freelist_count: 0,
        }
    }

    /// Total number of pages (including the header page 0).
    pub fn page_count(&self) -> u32 {
        self.pages.len() as u32
    }

    /// Number of free pages in the freelist.
    pub fn freelist_count(&self) -> u32 {
        self.freelist_count
    }

    /// Page size in bytes.
    pub fn page_size(&self) -> usize {
        self.page_size
    }

    /// Head of the freelist chain (page number, 0 = empty).
    pub fn freelist_head(&self) -> u32 {
        self.freelist_head
    }

    /// Construct from raw file data (for recovery/deserialization).
    ///
    /// Splits `data` into pages of `page_size` bytes each.
    /// Trailing bytes shorter than a full page are discarded.
    pub fn from_file_data(
        page_size: usize,
        data: &[u8],
        freelist_head: u32,
        freelist_count: u32,
    ) -> Self {
        let page_count = data.len() / page_size;
        let mut pages = Vec::with_capacity(page_count);
        for i in 0..page_count {
            let offset = i * page_size;
            pages.push(data[offset..offset + page_size].to_vec());
        }
        Self {
            page_size,
            pages,
            freelist_head,
            freelist_count,
        }
    }

    /// Allocate a new page of the given type.
    ///
    /// Prefers reusing a free page from the freelist. If the freelist is
    /// empty, appends a new page at the end.
    ///
    /// Returns the page number.
    pub fn alloc_page(&mut self, page_type: PageType) -> u32 {
        let page_num = if let Some(free_num) = self.pop_free_page() {
            // Reuse a free page
            self.pages[free_num as usize] = vec![0u8; self.page_size];
            free_num
        } else {
            // Extend: append a new page
            let num = self.pages.len() as u32;
            self.pages.push(vec![0u8; self.page_size]);
            num
        };

        // Write the page header
        let header = PageHeader {
            page_type,
            flags: 0,
            item_count: 0,
            checksum: 0, // will be set on write_page
        };
        header.write_to(&mut self.pages[page_num as usize]);

        page_num
    }

    /// Free a page, adding it to the freelist.
    ///
    /// Returns an error if page_num is 0 (header) or out of bounds.
    pub fn free_page(&mut self, page_num: u32) -> Result<()> {
        if page_num == 0 || page_num as usize >= self.pages.len() {
            return Err(HoraError::InvalidFile {
                reason: "cannot free page 0 or out-of-bounds page",
            });
        }

        // If there's room in the current freelist head page, add to it.
        // Otherwise, turn this page into a new freelist head.
        if self.freelist_head != 0 {
            let cap = freelist_capacity(self.page_size);
            let head = self.freelist_head as usize;
            let count = self.read_freelist_count(head);

            if (count as usize) < cap {
                // Add to existing freelist page
                self.write_freelist_entry(head, count, page_num);
                self.write_freelist_count(head, count + 1);
                self.freelist_count += 1;
                // Zero out the freed page
                self.pages[page_num as usize] = vec![0u8; self.page_size];
                let hdr = PageHeader {
                    page_type: PageType::Free,
                    flags: 0,
                    item_count: 0,
                    checksum: 0,
                };
                hdr.write_to(&mut self.pages[page_num as usize]);
                return Ok(());
            }
        }

        // Turn page_num into a new freelist head page
        self.pages[page_num as usize] = vec![0u8; self.page_size];
        let hdr = PageHeader {
            page_type: PageType::Free,
            flags: 0,
            item_count: 0,
            checksum: 0,
        };
        hdr.write_to(&mut self.pages[page_num as usize]);

        // Write next pointer = old head, count = 0
        let buf = &mut self.pages[page_num as usize];
        buf[PAGE_HEADER_SIZE..PAGE_HEADER_SIZE + 4]
            .copy_from_slice(&self.freelist_head.to_le_bytes());
        buf[PAGE_HEADER_SIZE + 4..PAGE_HEADER_SIZE + 6].copy_from_slice(&0u16.to_le_bytes());

        self.freelist_head = page_num;
        self.freelist_count += 1;
        Ok(())
    }

    /// Read a page's raw bytes. Returns an error if out of bounds.
    pub fn read_page(&self, page_num: u32) -> Result<&[u8]> {
        self.pages
            .get(page_num as usize)
            .map(|p| p.as_slice())
            .ok_or(HoraError::InvalidFile {
                reason: "page number out of bounds",
            })
    }

    /// Get a mutable reference to a page's raw bytes.
    pub fn write_page(&mut self, page_num: u32) -> Result<&mut [u8]> {
        self.pages
            .get_mut(page_num as usize)
            .map(|p| p.as_mut_slice())
            .ok_or(HoraError::InvalidFile {
                reason: "page number out of bounds",
            })
    }

    /// Compute and store the CRC32 checksum for a page's data content.
    pub fn seal_page(&mut self, page_num: u32) -> Result<()> {
        let page = self
            .pages
            .get(page_num as usize)
            .ok_or(HoraError::InvalidFile {
                reason: "page number out of bounds",
            })?;
        let checksum = crc32(&page[PAGE_HEADER_SIZE..]);
        let page = &mut self.pages[page_num as usize];
        page[4..8].copy_from_slice(&checksum.to_le_bytes());
        Ok(())
    }

    /// Verify the CRC32 checksum of a page.
    pub fn verify_page(&self, page_num: u32) -> Result<bool> {
        let page = self.read_page(page_num)?;
        let stored = u32::from_le_bytes([page[4], page[5], page[6], page[7]]);
        let computed = crc32(&page[PAGE_HEADER_SIZE..]);
        Ok(stored == computed)
    }

    // ── Compaction ─────────────────────────────────────────

    /// Compact: relocate pages from the tail into free slots, then truncate.
    ///
    /// Two-pointer algorithm: `dst` scans forward for free slots, `src` scans
    /// backward for used pages. Pages are swapped, then trailing free pages
    /// are truncated. The freelist is reset (no free pages remain).
    ///
    /// Returns a list of relocations `(old_page, new_page)` so the caller
    /// can update any external references (e.g., B+ tree pointers).
    pub fn compact(&mut self) -> Vec<(u32, u32)> {
        let mut relocations = Vec::new();
        if self.pages.len() <= 1 {
            return relocations;
        }

        // Identify free pages by scanning headers
        let mut is_free = vec![false; self.pages.len()];
        for (i, free) in is_free.iter_mut().enumerate().skip(1) {
            if let Some(hdr) = PageHeader::read_from(&self.pages[i]) {
                *free = hdr.page_type == PageType::Free;
            }
        }

        // Two-pointer: fill holes from front with pages from back
        let mut dst = 1; // skip header page 0
        let mut src = self.pages.len() - 1;

        while dst < src {
            if !is_free[dst] {
                dst += 1;
                continue;
            }
            if is_free[src] {
                src -= 1;
                continue;
            }

            // Swap used page from src into free slot at dst
            self.pages.swap(dst, src);
            is_free[dst] = false;
            is_free[src] = true;
            relocations.push((src as u32, dst as u32));

            dst += 1;
            src -= 1;
        }

        // Truncate trailing free pages
        let mut new_len = self.pages.len();
        while new_len > 1 && is_free[new_len - 1] {
            new_len -= 1;
        }
        self.pages.truncate(new_len);

        // Freelist is now empty (all holes filled and truncated)
        self.freelist_head = 0;
        self.freelist_count = 0;

        relocations
    }

    /// Append a raw page (for full vacuum rebuild). Returns the page number.
    pub fn push_raw_page(&mut self, data: Vec<u8>) -> u32 {
        let num = self.pages.len() as u32;
        self.pages.push(data);
        num
    }

    // ── Freelist internals ────────────────────────────────

    /// Pop one page number from the freelist. Returns None if empty.
    fn pop_free_page(&mut self) -> Option<u32> {
        if self.freelist_head == 0 || self.freelist_count == 0 {
            return None;
        }

        let head = self.freelist_head as usize;
        let count = self.read_freelist_count(head);

        if count > 0 {
            // Pop the last entry
            let page_num = self.read_freelist_entry(head, count - 1);
            self.write_freelist_count(head, count - 1);
            self.freelist_count -= 1;
            Some(page_num)
        } else {
            // This freelist page itself is free — reclaim it
            let reclaimed = self.freelist_head;
            let next = self.read_freelist_next(head);
            self.freelist_head = next;
            self.freelist_count -= 1;
            Some(reclaimed)
        }
    }

    /// Read the `next_freelist_page` pointer from a freelist page.
    fn read_freelist_next(&self, page_idx: usize) -> u32 {
        let buf = &self.pages[page_idx];
        let offset = PAGE_HEADER_SIZE;
        u32::from_le_bytes([
            buf[offset],
            buf[offset + 1],
            buf[offset + 2],
            buf[offset + 3],
        ])
    }

    /// Read the entry count in a freelist page.
    fn read_freelist_count(&self, page_idx: usize) -> u16 {
        let buf = &self.pages[page_idx];
        let offset = PAGE_HEADER_SIZE + 4;
        u16::from_le_bytes([buf[offset], buf[offset + 1]])
    }

    /// Write the entry count in a freelist page.
    fn write_freelist_count(&mut self, page_idx: usize, count: u16) {
        let offset = PAGE_HEADER_SIZE + 4;
        self.pages[page_idx][offset..offset + 2].copy_from_slice(&count.to_le_bytes());
    }

    /// Read a free page ID at position `idx` in the freelist entries.
    fn read_freelist_entry(&self, page_idx: usize, idx: u16) -> u32 {
        let offset = PAGE_HEADER_SIZE + 6 + (idx as usize) * 4;
        let buf = &self.pages[page_idx];
        u32::from_le_bytes([
            buf[offset],
            buf[offset + 1],
            buf[offset + 2],
            buf[offset + 3],
        ])
    }

    /// Write a free page ID at position `idx` in the freelist entries.
    fn write_freelist_entry(&mut self, page_idx: usize, idx: u16, page_num: u32) {
        let offset = PAGE_HEADER_SIZE + 6 + (idx as usize) * 4;
        self.pages[page_idx][offset..offset + 4].copy_from_slice(&page_num.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc32_known_value() {
        // CRC32 of "123456789" = 0xCBF43926 (IEEE standard test vector)
        let data = b"123456789";
        assert_eq!(crc32(data), 0xCBF4_3926);
    }

    #[test]
    fn test_crc32_empty() {
        assert_eq!(crc32(b""), 0x0000_0000);
    }

    #[test]
    fn test_page_header_roundtrip() {
        let header = PageHeader {
            page_type: PageType::EntityLeaf,
            flags: 0x42,
            item_count: 85,
            checksum: 0xDEAD_BEEF,
        };
        let mut buf = [0u8; 8];
        header.write_to(&mut buf);
        let decoded = PageHeader::read_from(&buf).unwrap();
        assert_eq!(decoded.page_type, PageType::EntityLeaf);
        assert_eq!(decoded.flags, 0x42);
        assert_eq!(decoded.item_count, 85);
        assert_eq!(decoded.checksum, 0xDEAD_BEEF);
    }

    #[test]
    fn test_alloc_pages_increases_count() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        assert_eq!(alloc.page_count(), 1); // header only

        alloc.alloc_page(PageType::EntityLeaf);
        alloc.alloc_page(PageType::EntityLeaf);
        alloc.alloc_page(PageType::EdgeData);
        assert_eq!(alloc.page_count(), 4); // header + 3

        // Verify page types
        let p1 = alloc.read_page(1).unwrap();
        assert_eq!(
            PageHeader::read_from(p1).unwrap().page_type,
            PageType::EntityLeaf
        );
        let p3 = alloc.read_page(3).unwrap();
        assert_eq!(
            PageHeader::read_from(p3).unwrap().page_type,
            PageType::EdgeData
        );
    }

    #[test]
    fn test_free_and_realloc_from_freelist() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        let p1 = alloc.alloc_page(PageType::EntityLeaf);
        let p2 = alloc.alloc_page(PageType::EntityLeaf);
        let p3 = alloc.alloc_page(PageType::EntityLeaf);
        assert_eq!(alloc.page_count(), 4);
        assert_eq!(alloc.freelist_count(), 0);

        // Free pages 2 and 3
        alloc.free_page(p2).unwrap();
        alloc.free_page(p3).unwrap();
        assert_eq!(alloc.freelist_count(), 2);

        // Re-allocate should reuse freed pages (not grow)
        let p4 = alloc.alloc_page(PageType::EdgeData);
        assert_eq!(alloc.freelist_count(), 1);
        assert!(p4 == p2 || p4 == p3, "should reuse freed page, got {p4}");

        let p5 = alloc.alloc_page(PageType::EdgeData);
        assert_eq!(alloc.freelist_count(), 0);
        assert!(p5 == p2 || p5 == p3, "should reuse freed page, got {p5}");

        // No more free pages → should extend
        let p6 = alloc.alloc_page(PageType::VectorData);
        assert_eq!(p6, 4); // new page appended
        assert_eq!(alloc.page_count(), 5);

        // All should have correct types
        assert_eq!(
            PageHeader::read_from(alloc.read_page(p1).unwrap())
                .unwrap()
                .page_type,
            PageType::EntityLeaf
        );
        assert_eq!(
            PageHeader::read_from(alloc.read_page(p4).unwrap())
                .unwrap()
                .page_type,
            PageType::EdgeData
        );
        assert_eq!(
            PageHeader::read_from(alloc.read_page(p6).unwrap())
                .unwrap()
                .page_type,
            PageType::VectorData
        );
    }

    #[test]
    fn test_free_page_0_errors() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        assert!(alloc.free_page(0).is_err());
    }

    #[test]
    fn test_free_page_out_of_bounds_errors() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        assert!(alloc.free_page(999).is_err());
    }

    #[test]
    fn test_seal_and_verify_page() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        let p = alloc.alloc_page(PageType::EntityLeaf);

        // Write some data
        let page = alloc.write_page(p).unwrap();
        page[PAGE_HEADER_SIZE] = 0xAB;
        page[PAGE_HEADER_SIZE + 1] = 0xCD;

        // Seal (compute CRC32)
        alloc.seal_page(p).unwrap();
        assert!(alloc.verify_page(p).unwrap());

        // Corrupt the data
        alloc.write_page(p).unwrap()[PAGE_HEADER_SIZE] = 0xFF;
        assert!(!alloc.verify_page(p).unwrap());
    }

    #[test]
    fn test_freelist_capacity_default() {
        let cap = freelist_capacity(DEFAULT_PAGE_SIZE);
        // (4096 - 8 - 6) / 4 = 4082 / 4 = 1020
        assert_eq!(cap, 1020);
    }

    #[test]
    fn test_alloc_10_pages_count() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        for _ in 0..10 {
            alloc.alloc_page(PageType::EntityLeaf);
        }
        // header (page 0) + 10 data pages = 11
        assert_eq!(alloc.page_count(), 11);
    }

    #[test]
    fn test_free_5_realloc_from_freelist_first() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        let mut pages = Vec::new();
        for _ in 0..10 {
            pages.push(alloc.alloc_page(PageType::EntityLeaf));
        }

        // Free 5 pages
        for &p in &pages[0..5] {
            alloc.free_page(p).unwrap();
        }
        assert_eq!(alloc.freelist_count(), 5);

        // Re-alloc 5 → should all come from freelist
        for _ in 0..5 {
            alloc.alloc_page(PageType::EdgeData);
        }
        assert_eq!(alloc.freelist_count(), 0);
        // page_count should not have grown (reused freed pages)
        assert_eq!(alloc.page_count(), 11);
    }

    #[test]
    fn test_page_type_roundtrip() {
        for v in 0..=12u8 {
            let pt = PageType::from_u8(v).unwrap();
            assert_eq!(pt as u8, v);
        }
        assert!(PageType::from_u8(13).is_none());
        assert!(PageType::from_u8(255).is_none());
    }

    #[test]
    fn test_usable_bytes() {
        assert_eq!(usable_bytes(4096), 4088);
        assert_eq!(usable_bytes(8192), 8184);
    }

    #[test]
    fn test_compact_removes_free_pages() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        for _ in 0..5 {
            alloc.alloc_page(PageType::EntityLeaf);
        }
        assert_eq!(alloc.page_count(), 6); // header + 5

        // Free pages 2 and 4 (creating holes)
        alloc.free_page(2).unwrap();
        alloc.free_page(4).unwrap();
        assert_eq!(alloc.freelist_count(), 2);

        let relocations = alloc.compact();
        // 2 holes filled, trailing pages truncated
        assert_eq!(alloc.page_count(), 4); // header + 3 used
        assert_eq!(alloc.freelist_count(), 0);
        assert!(!relocations.is_empty());
    }

    #[test]
    fn test_compact_preserves_data() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);

        // Alloc 4 pages with recognizable data
        let p1 = alloc.alloc_page(PageType::EntityLeaf);
        alloc.write_page(p1).unwrap()[PAGE_HEADER_SIZE] = 0xAA;
        let p2 = alloc.alloc_page(PageType::EdgeData);
        alloc.write_page(p2).unwrap()[PAGE_HEADER_SIZE] = 0xBB;
        let p3 = alloc.alloc_page(PageType::VectorData);
        alloc.write_page(p3).unwrap()[PAGE_HEADER_SIZE] = 0xCC;
        let p4 = alloc.alloc_page(PageType::StringPool);
        alloc.write_page(p4).unwrap()[PAGE_HEADER_SIZE] = 0xDD;

        // Free p2 (hole in the middle)
        alloc.free_page(p2).unwrap();

        let relocations = alloc.compact();

        // p4 (last used) should have moved to p2's slot
        assert_eq!(alloc.page_count(), 4); // header + 3 used

        // Collect all data bytes from surviving pages
        let mut data_bytes: Vec<u8> = Vec::new();
        for i in 1..alloc.page_count() {
            data_bytes.push(alloc.read_page(i).unwrap()[PAGE_HEADER_SIZE]);
        }
        data_bytes.sort();
        // All original data should be present (except freed page 0xBB)
        assert_eq!(data_bytes, vec![0xAA, 0xCC, 0xDD]);
        assert_eq!(relocations.len(), 1);
    }

    #[test]
    fn test_compact_no_free_pages() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        for _ in 0..3 {
            alloc.alloc_page(PageType::EntityLeaf);
        }
        let relocations = alloc.compact();
        assert!(relocations.is_empty());
        assert_eq!(alloc.page_count(), 4);
    }

    #[test]
    fn test_compact_all_free() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        let p1 = alloc.alloc_page(PageType::EntityLeaf);
        let p2 = alloc.alloc_page(PageType::EntityLeaf);
        alloc.free_page(p1).unwrap();
        alloc.free_page(p2).unwrap();

        alloc.compact();
        assert_eq!(alloc.page_count(), 1); // only header remains
        assert_eq!(alloc.freelist_count(), 0);
    }
}
