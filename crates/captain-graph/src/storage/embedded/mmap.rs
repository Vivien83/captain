//! Memory-mapped and read-based page readers for the embedded storage.
//!
//! `MmapReader` provides zero-copy page reads via OS mmap (Unix only, zero deps).
//! `ReadReader` provides a portable fallback that reads the file into memory.
//! Both implement the `PageReader` trait for uniform access.

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use super::page::PageAllocator;

/// Trait for reading pages from a database file.
pub trait PageReader {
    /// Read a page by number. Returns None if out of bounds.
    fn read_page(&self, page_num: u32) -> Option<&[u8]>;

    /// Total number of complete pages in the backing store.
    fn page_count(&self) -> u32;

    /// Page size in bytes.
    fn page_size(&self) -> usize;
}

// ── File I/O ─────────────────────────────────────────────

/// Write all pages from a PageAllocator to a file.
pub fn write_pages_to_file(alloc: &PageAllocator, path: &Path) -> std::io::Result<()> {
    let mut file = File::create(path)?;
    for i in 0..alloc.page_count() {
        if let Ok(page) = alloc.read_page(i) {
            file.write_all(page)?;
        }
    }
    file.sync_all()?;
    Ok(())
}

// ── ReadReader (portable fallback) ───────────────────────

/// Page reader that loads the file into a `Vec<u8>`.
/// Works on all platforms including WASM.
pub struct ReadReader {
    data: Vec<u8>,
    page_sz: usize,
}

impl ReadReader {
    /// Open a file and read all contents into memory.
    pub fn open(path: &Path, page_size: usize) -> std::io::Result<Self> {
        let mut file = File::open(path)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;
        Ok(Self {
            data,
            page_sz: page_size,
        })
    }

    /// Reload from file (after checkpoint grew the file).
    pub fn reload(&mut self, path: &Path) -> std::io::Result<()> {
        let mut file = File::open(path)?;
        self.data.clear();
        file.read_to_end(&mut self.data)?;
        Ok(())
    }
}

impl PageReader for ReadReader {
    fn read_page(&self, page_num: u32) -> Option<&[u8]> {
        let offset = page_num as usize * self.page_sz;
        let end = offset + self.page_sz;
        if end <= self.data.len() {
            Some(&self.data[offset..end])
        } else {
            None
        }
    }

    fn page_count(&self) -> u32 {
        (self.data.len() / self.page_sz) as u32
    }

    fn page_size(&self) -> usize {
        self.page_sz
    }
}

// ── MmapReader (Unix zero-copy, zero deps) ───────────────

#[cfg(unix)]
mod mmap_unix {
    use super::*;
    use std::os::unix::io::AsRawFd;

    const PROT_READ: i32 = 1;
    const MAP_PRIVATE: i32 = 2;

    extern "C" {
        fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, offset: i64) -> *mut u8;
        fn munmap(addr: *mut u8, len: usize) -> i32;
    }

    /// Sentinel returned by `mmap` on failure.
    const MAP_FAILED: *mut u8 = !0usize as *mut u8;

    /// Memory-mapped page reader. Zero-copy reads backed by the OS page cache.
    ///
    /// Available on Unix (macOS, Linux, BSDs). WASM uses `ReadReader` instead.
    pub struct MmapReader {
        ptr: *const u8,
        len: usize,
        page_sz: usize,
        /// Keep the file handle alive for the lifetime of the mapping.
        _file: File,
    }

    impl MmapReader {
        /// Open and mmap a database file for read-only access.
        pub fn open(path: &Path, page_size: usize) -> std::io::Result<Self> {
            let file = File::open(path)?;
            let len = file.metadata()?.len() as usize;

            if len == 0 {
                return Ok(Self {
                    ptr: std::ptr::null(),
                    len: 0,
                    page_sz: page_size,
                    _file: file,
                });
            }

            let fd = file.as_raw_fd();
            // SAFETY: fd is a valid open file descriptor, len > 0 (checked above).
            // Return value is checked against MAP_FAILED before use.
            let ptr = unsafe { mmap(std::ptr::null_mut(), len, PROT_READ, MAP_PRIVATE, fd, 0) };
            if ptr == MAP_FAILED {
                return Err(std::io::Error::last_os_error());
            }

            Ok(Self {
                ptr: ptr as *const u8,
                len,
                page_sz: page_size,
                _file: file,
            })
        }

        /// Re-mmap after the file has grown (e.g., after checkpoint).
        pub fn remap(&mut self, path: &Path) -> std::io::Result<()> {
            // SAFETY: munmap is called only when ptr is non-null and len > 0.
            // We immediately null the pointer to prevent UAF if a later step fails.
            if !self.ptr.is_null() && self.len > 0 {
                unsafe {
                    munmap(self.ptr as *mut u8, self.len);
                }
                self.ptr = std::ptr::null();
                self.len = 0;
            }

            let file = File::open(path)?;
            let new_len = file.metadata()?.len() as usize;

            if new_len == 0 {
                self.ptr = std::ptr::null();
                self.len = 0;
                self._file = file;
                return Ok(());
            }

            let fd = file.as_raw_fd();
            // SAFETY: fd is valid, new_len > 0 (checked above). Old mapping already unmapped.
            let ptr = unsafe { mmap(std::ptr::null_mut(), new_len, PROT_READ, MAP_PRIVATE, fd, 0) };
            if ptr == MAP_FAILED {
                return Err(std::io::Error::last_os_error());
            }

            self.ptr = ptr as *const u8;
            self.len = new_len;
            self._file = file;
            Ok(())
        }
    }

    impl PageReader for MmapReader {
        fn read_page(&self, page_num: u32) -> Option<&[u8]> {
            let offset = page_num as usize * self.page_sz;
            let end = offset + self.page_sz;
            if end <= self.len && !self.ptr.is_null() {
                // SAFETY: ptr is non-null and end <= len (checked above), so the
                // range [ptr+offset .. ptr+offset+page_sz] is within the mapped region.
                Some(unsafe { std::slice::from_raw_parts(self.ptr.add(offset), self.page_sz) })
            } else {
                None
            }
        }

        fn page_count(&self) -> u32 {
            (self.len / self.page_sz) as u32
        }

        fn page_size(&self) -> usize {
            self.page_sz
        }
    }

    impl Drop for MmapReader {
        fn drop(&mut self) {
            if !self.ptr.is_null() && self.len > 0 {
                // SAFETY: ptr is non-null, len > 0, and this is the only Drop so
                // the region is unmapped exactly once.
                unsafe {
                    munmap(self.ptr as *mut u8, self.len);
                }
            }
        }
    }

    // SAFETY: The mmap region is read-only (PROT_READ) and MAP_PRIVATE.
    // Multiple threads can safely read from the same immutable memory region.
    // The File handle is kept alive for the lifetime of the mapping.
    unsafe impl Send for MmapReader {}
    unsafe impl Sync for MmapReader {}
}

#[cfg(unix)]
pub use mmap_unix::MmapReader;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::embedded::page::{
        PageAllocator, PageType, DEFAULT_PAGE_SIZE, PAGE_HEADER_SIZE,
    };

    fn make_allocator_with_data() -> PageAllocator {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        let p1 = alloc.alloc_page(PageType::EntityLeaf);
        let p2 = alloc.alloc_page(PageType::EdgeData);
        let p3 = alloc.alloc_page(PageType::VectorData);

        alloc.write_page(p1).unwrap()[PAGE_HEADER_SIZE] = 0xAA;
        alloc.write_page(p2).unwrap()[PAGE_HEADER_SIZE] = 0xBB;
        alloc.write_page(p3).unwrap()[PAGE_HEADER_SIZE] = 0xCC;

        alloc
    }

    #[test]
    fn test_write_and_read_reader() {
        let alloc = make_allocator_with_data();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        write_pages_to_file(&alloc, &path).unwrap();

        let reader = ReadReader::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        assert_eq!(reader.page_count(), 4); // header + 3

        for i in 0..alloc.page_count() {
            let expected = alloc.read_page(i).unwrap();
            let actual = reader.read_page(i).unwrap();
            assert_eq!(actual, expected, "page {i} mismatch");
        }
    }

    #[test]
    fn test_read_reader_out_of_bounds() {
        let alloc = make_allocator_with_data();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        write_pages_to_file(&alloc, &path).unwrap();
        let reader = ReadReader::open(&path, DEFAULT_PAGE_SIZE).unwrap();

        assert!(reader.read_page(99).is_none());
    }

    #[test]
    fn test_read_reader_reload_sees_new_pages() {
        let mut alloc = make_allocator_with_data();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        write_pages_to_file(&alloc, &path).unwrap();
        let mut reader = ReadReader::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        assert_eq!(reader.page_count(), 4);

        // Simulate checkpoint: add pages and rewrite
        let p4 = alloc.alloc_page(PageType::StringPool);
        alloc.write_page(p4).unwrap()[PAGE_HEADER_SIZE] = 0xDD;
        write_pages_to_file(&alloc, &path).unwrap();

        assert_eq!(reader.page_count(), 4); // stale
        reader.reload(&path).unwrap();
        assert_eq!(reader.page_count(), 5);
        assert_eq!(reader.read_page(4).unwrap()[PAGE_HEADER_SIZE], 0xDD);
    }

    #[test]
    fn test_page_reader_trait_object() {
        let alloc = make_allocator_with_data();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        write_pages_to_file(&alloc, &path).unwrap();
        let reader = ReadReader::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        let trait_obj: &dyn PageReader = &reader;

        assert_eq!(trait_obj.page_count(), 4);
        assert_eq!(trait_obj.read_page(1).unwrap()[PAGE_HEADER_SIZE], 0xAA);
    }

    #[cfg(unix)]
    #[test]
    fn test_mmap_matches_read_reader() {
        let alloc = make_allocator_with_data();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        write_pages_to_file(&alloc, &path).unwrap();

        let mmap_r = MmapReader::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        let read_r = ReadReader::open(&path, DEFAULT_PAGE_SIZE).unwrap();

        assert_eq!(mmap_r.page_count(), read_r.page_count());

        for i in 0..mmap_r.page_count() {
            assert_eq!(
                mmap_r.read_page(i).unwrap(),
                read_r.read_page(i).unwrap(),
                "page {i} mismatch mmap vs read"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_mmap_remap_sees_new_pages() {
        let mut alloc = make_allocator_with_data();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        write_pages_to_file(&alloc, &path).unwrap();
        let mut reader = MmapReader::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        assert_eq!(reader.page_count(), 4);

        // Simulate checkpoint: add 2 pages and rewrite
        let p4 = alloc.alloc_page(PageType::Bm25Posting);
        alloc.write_page(p4).unwrap()[PAGE_HEADER_SIZE] = 0xEE;
        let p5 = alloc.alloc_page(PageType::TemporalIndex);
        alloc.write_page(p5).unwrap()[PAGE_HEADER_SIZE] = 0xFF;
        write_pages_to_file(&alloc, &path).unwrap();

        assert_eq!(reader.page_count(), 4); // stale
        assert!(reader.read_page(4).is_none());

        reader.remap(&path).unwrap();
        assert_eq!(reader.page_count(), 6);
        assert_eq!(reader.read_page(4).unwrap()[PAGE_HEADER_SIZE], 0xEE);
        assert_eq!(reader.read_page(5).unwrap()[PAGE_HEADER_SIZE], 0xFF);
    }

    #[cfg(unix)]
    #[test]
    fn test_mmap_out_of_bounds() {
        let alloc = make_allocator_with_data();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        write_pages_to_file(&alloc, &path).unwrap();
        let reader = MmapReader::open(&path, DEFAULT_PAGE_SIZE).unwrap();

        assert!(reader.read_page(99).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn test_mmap_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.hora");
        File::create(&path).unwrap();

        let reader = MmapReader::open(&path, DEFAULT_PAGE_SIZE).unwrap();
        assert_eq!(reader.page_count(), 0);
        assert!(reader.read_page(0).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn test_mmap_data_integrity() {
        let alloc = make_allocator_with_data();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        write_pages_to_file(&alloc, &path).unwrap();
        let reader = MmapReader::open(&path, DEFAULT_PAGE_SIZE).unwrap();

        // Verify every byte of every page matches the allocator
        for i in 0..alloc.page_count() {
            let expected = alloc.read_page(i).unwrap();
            let actual = reader.read_page(i).unwrap();
            assert_eq!(actual.len(), expected.len());
            assert_eq!(actual, expected, "page {i} byte mismatch");
        }
    }
}
