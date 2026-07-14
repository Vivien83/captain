//! B+ tree index backed by the page allocator.
//!
//! Keys are `u64` (EntityId/EdgeId). Values are variable-length byte slices.
//! Interior pages hold separator keys + child page pointers.
//! Leaf pages hold sorted (key, value) pairs and are linked for sequential scan.
//!
//! The tree grows upward: when the root splits, a new root is created.

use super::page::{PageAllocator, PageHeader, PageType, PAGE_HEADER_SIZE};
use crate::error::{HoraError, Result};

// ── Leaf page layout ──────────────────────────────────────
//
// After PageHeader (8B):
//   prev_leaf: u32 (4B)   — 0 = no previous
//   next_leaf: u32 (4B)   — 0 = no next
//   entry_count: u16 (2B)
//   used_bytes: u16 (2B)  — total bytes used by entries (for split decisions)
//   entries: [LeafEntry]*
//
// LeafEntry:
//   key: u64 (8B)
//   value_len: u16 (2B)
//   value: [u8; value_len]
//   deleted: u8 (1B)      — 0 = live, 1 = tombstone

const LEAF_META_SIZE: usize = 12; // prev(4) + next(4) + count(2) + used(2)
const LEAF_ENTRY_OVERHEAD: usize = 11; // key(8) + value_len(2) + deleted(1)

// ── Interior page layout ──────────────────────────────────
//
// After PageHeader (8B):
//   key_count: u16 (2B)
//   children: [u32; key_count + 1]  — page numbers
//   keys: [u64; key_count]          — separator keys
//
// Layout in bytes: [key_count(2)][child_0(4)][key_0(8)][child_1(4)][key_1(8)]...[child_n(4)]
// So: 2 + (key_count + 1) * 4 + key_count * 8 = 2 + 4 + key_count * 12

const INTERIOR_META_SIZE: usize = 6; // key_count(2) + first child(4)

/// Maximum interior keys that fit in a page.
pub const fn max_interior_keys(page_size: usize) -> usize {
    let usable = page_size - PAGE_HEADER_SIZE;
    // usable = 2 + 4 + key_count * 12, so key_count = (usable - 6) / 12
    (usable - INTERIOR_META_SIZE) / 12
}

/// A B+ tree index operating on a `PageAllocator`.
pub struct BPlusTree {
    root_page: u32,
    leaf_type: PageType,
    interior_type: PageType,
}

impl BPlusTree {
    /// Create a new B+ tree with an empty leaf root.
    pub fn new(alloc: &mut PageAllocator, leaf_type: PageType, interior_type: PageType) -> Self {
        let root = alloc.alloc_page(leaf_type);
        let page = alloc.write_page(root).unwrap();
        // Initialize leaf metadata: prev=0, next=0, count=0, used=0
        let off = PAGE_HEADER_SIZE;
        page[off..off + 4].copy_from_slice(&0u32.to_le_bytes()); // prev
        page[off + 4..off + 8].copy_from_slice(&0u32.to_le_bytes()); // next
        page[off + 8..off + 10].copy_from_slice(&0u16.to_le_bytes()); // count
        page[off + 10..off + 12].copy_from_slice(&0u16.to_le_bytes()); // used

        Self {
            root_page: root,
            leaf_type,
            interior_type,
        }
    }

    /// Open an existing B+ tree with a known root page.
    pub fn open(root_page: u32, leaf_type: PageType, interior_type: PageType) -> Self {
        Self {
            root_page,
            leaf_type,
            interior_type,
        }
    }

    /// Root page number.
    pub fn root_page(&self) -> u32 {
        self.root_page
    }

    /// Lookup a key. Returns the value if found and not deleted.
    pub fn get(&self, alloc: &PageAllocator, key: u64) -> Result<Option<Vec<u8>>> {
        let leaf_page = self.find_leaf(alloc, key)?;
        let page = alloc.read_page(leaf_page)?;
        let entries = read_leaf_entries(page, alloc.page_size());

        for entry in &entries {
            if entry.key == key && !entry.deleted {
                return Ok(Some(entry.value.clone()));
            }
        }
        Ok(None)
    }

    /// Insert a key-value pair. If the key already exists, replaces the value.
    pub fn insert(&mut self, alloc: &mut PageAllocator, key: u64, value: &[u8]) -> Result<()> {
        // Find the path from root to the target leaf
        let path = self.find_path(alloc, key)?;
        let leaf_page = *path.last().unwrap();

        // Try to insert into the leaf
        let page = alloc.read_page(leaf_page)?;
        let mut entries = read_leaf_entries(page, alloc.page_size());

        // Check if key already exists → replace
        if let Some(entry) = entries.iter_mut().find(|e| e.key == key) {
            entry.value = value.to_vec();
            entry.deleted = false;
            write_leaf_entries(alloc, leaf_page, &entries)?;
            return Ok(());
        }

        // Insert in sorted position
        let pos = entries.partition_point(|e| e.key < key);
        entries.insert(
            pos,
            LeafEntry {
                key,
                value: value.to_vec(),
                deleted: false,
            },
        );

        // Check if it fits
        let needed = entries_byte_size(&entries);
        let usable = alloc.page_size() - PAGE_HEADER_SIZE - LEAF_META_SIZE;

        if needed <= usable {
            write_leaf_entries(alloc, leaf_page, &entries)?;
            Ok(())
        } else {
            // Split the leaf
            self.split_leaf(alloc, &path, entries)?;
            Ok(())
        }
    }

    /// Delete a key (lazy tombstone). Returns true if the key was found.
    pub fn delete(&mut self, alloc: &mut PageAllocator, key: u64) -> Result<bool> {
        let leaf_page = self.find_leaf(alloc, key)?;
        let page = alloc.read_page(leaf_page)?;
        let mut entries = read_leaf_entries(page, alloc.page_size());

        if let Some(entry) = entries.iter_mut().find(|e| e.key == key && !e.deleted) {
            entry.deleted = true;
            write_leaf_entries(alloc, leaf_page, &entries)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Scan all live entries in key order by following leaf links.
    pub fn scan(&self, alloc: &PageAllocator) -> Result<Vec<(u64, Vec<u8>)>> {
        // Find the leftmost leaf
        let mut page_num = self.root_page;
        loop {
            let page = alloc.read_page(page_num)?;
            let header = PageHeader::read_from(page).ok_or(HoraError::InvalidFile {
                reason: "invalid page header in scan",
            })?;
            if header.page_type == self.leaf_type {
                break;
            }
            // Interior: follow the leftmost child
            page_num = read_interior_child(page, 0);
        }

        // Walk the leftmost leaf and follow next_leaf links
        // But first, walk back to the very first leaf via prev_leaf
        loop {
            let page = alloc.read_page(page_num)?;
            let prev = read_leaf_prev(page);
            if prev == 0 {
                break;
            }
            page_num = prev;
        }

        // Now scan forward
        let mut results = Vec::new();
        let mut current = page_num;
        while current != 0 {
            let page = alloc.read_page(current)?;
            let entries = read_leaf_entries(page, alloc.page_size());
            for entry in &entries {
                if !entry.deleted {
                    results.push((entry.key, entry.value.clone()));
                }
            }
            current = read_leaf_next(page);
        }
        Ok(results)
    }

    // ── Internal helpers ──────────────────────────────────

    /// Find the leaf page that should contain the given key.
    fn find_leaf(&self, alloc: &PageAllocator, key: u64) -> Result<u32> {
        let mut page_num = self.root_page;
        loop {
            let page = alloc.read_page(page_num)?;
            let header = PageHeader::read_from(page).ok_or(HoraError::InvalidFile {
                reason: "invalid page header",
            })?;
            if header.page_type == self.leaf_type {
                return Ok(page_num);
            }
            // Interior: binary search for the right child
            page_num = find_child_for_key(page, key);
        }
    }

    /// Find the path (page numbers) from root to leaf for the given key.
    fn find_path(&self, alloc: &PageAllocator, key: u64) -> Result<Vec<u32>> {
        let mut path = Vec::new();
        let mut page_num = self.root_page;
        loop {
            path.push(page_num);
            let page = alloc.read_page(page_num)?;
            let header = PageHeader::read_from(page).ok_or(HoraError::InvalidFile {
                reason: "invalid page header",
            })?;
            if header.page_type == self.leaf_type {
                return Ok(path);
            }
            page_num = find_child_for_key(page, key);
        }
    }

    /// Split a leaf and propagate upward.
    fn split_leaf(
        &mut self,
        alloc: &mut PageAllocator,
        path: &[u32],
        entries: Vec<LeafEntry>,
    ) -> Result<()> {
        let leaf_page = *path.last().unwrap();
        let mid = entries.len() / 2;
        let left_entries = &entries[..mid];
        let right_entries = &entries[mid..];
        let split_key = right_entries[0].key;

        // Allocate new right leaf
        let right_page = alloc.alloc_page(self.leaf_type);

        // Read old next_leaf from the current leaf
        let old_next = read_leaf_next(alloc.read_page(leaf_page)?);

        // Write left entries to the existing leaf, link to right
        write_leaf_entries(alloc, leaf_page, left_entries)?;
        set_leaf_next(alloc, leaf_page, right_page)?;

        // Write right entries, link prev=left, next=old_next
        write_leaf_entries(alloc, right_page, right_entries)?;
        set_leaf_prev(alloc, right_page, leaf_page)?;
        set_leaf_next(alloc, right_page, old_next)?;

        // Update old_next's prev pointer if it exists
        if old_next != 0 {
            set_leaf_prev(alloc, old_next, right_page)?;
        }

        // Propagate the split up to the parent
        self.insert_into_parent(alloc, path, split_key, right_page)
    }

    /// Insert a new key + right child into the parent interior node.
    /// If the parent is full, split it too.
    fn insert_into_parent(
        &mut self,
        alloc: &mut PageAllocator,
        path: &[u32],
        key: u64,
        right_child: u32,
    ) -> Result<()> {
        if path.len() < 2 {
            // The root was the leaf — create a new interior root
            let new_root = alloc.alloc_page(self.interior_type);
            write_interior_page(alloc, new_root, &[key], &[self.root_page, right_child])?;
            self.root_page = new_root;
            return Ok(());
        }

        let parent_page = path[path.len() - 2];
        let page = alloc.read_page(parent_page)?;
        let (mut keys, mut children) = read_interior_page(page);

        // Find insertion position
        let pos = keys.partition_point(|&k| k < key);
        keys.insert(pos, key);
        children.insert(pos + 1, right_child);

        // Check if interior fits
        let max_keys = max_interior_keys(alloc.page_size());
        if keys.len() <= max_keys {
            write_interior_page(alloc, parent_page, &keys, &children)?;
            Ok(())
        } else {
            // Split the interior node
            let mid = keys.len() / 2;
            let push_up_key = keys[mid];

            let left_keys = &keys[..mid];
            let left_children = &children[..mid + 1];
            let right_keys = &keys[mid + 1..];
            let right_children = &children[mid + 1..];

            write_interior_page(alloc, parent_page, left_keys, left_children)?;
            let new_interior = alloc.alloc_page(self.interior_type);
            write_interior_page(alloc, new_interior, right_keys, right_children)?;

            // Propagate up
            let parent_path = &path[..path.len() - 1];
            self.insert_into_parent(alloc, parent_path, push_up_key, new_interior)
        }
    }
}

// ── Leaf entry ────────────────────────────────────────────

#[derive(Debug, Clone)]
struct LeafEntry {
    key: u64,
    value: Vec<u8>,
    deleted: bool,
}

fn entry_byte_size(entry: &LeafEntry) -> usize {
    LEAF_ENTRY_OVERHEAD + entry.value.len()
}

fn entries_byte_size(entries: &[LeafEntry]) -> usize {
    entries.iter().map(entry_byte_size).sum()
}

// ── Leaf page read/write ──────────────────────────────────

fn read_leaf_prev(page: &[u8]) -> u32 {
    let off = PAGE_HEADER_SIZE;
    u32::from_le_bytes([page[off], page[off + 1], page[off + 2], page[off + 3]])
}

fn read_leaf_next(page: &[u8]) -> u32 {
    let off = PAGE_HEADER_SIZE + 4;
    u32::from_le_bytes([page[off], page[off + 1], page[off + 2], page[off + 3]])
}

fn read_leaf_entries(page: &[u8], page_size: usize) -> Vec<LeafEntry> {
    let off = PAGE_HEADER_SIZE;
    let count = u16::from_le_bytes([page[off + 8], page[off + 9]]) as usize;
    let mut entries = Vec::with_capacity(count);
    let mut cursor = PAGE_HEADER_SIZE + LEAF_META_SIZE;

    for _ in 0..count {
        if cursor + LEAF_ENTRY_OVERHEAD > page_size {
            break;
        }
        let key = u64::from_le_bytes([
            page[cursor],
            page[cursor + 1],
            page[cursor + 2],
            page[cursor + 3],
            page[cursor + 4],
            page[cursor + 5],
            page[cursor + 6],
            page[cursor + 7],
        ]);
        let value_len = u16::from_le_bytes([page[cursor + 8], page[cursor + 9]]) as usize;
        let deleted = page[cursor + 10] != 0;
        cursor += LEAF_ENTRY_OVERHEAD;

        if cursor + value_len > page_size {
            break;
        }
        let value = page[cursor..cursor + value_len].to_vec();
        cursor += value_len;

        entries.push(LeafEntry {
            key,
            value,
            deleted,
        });
    }
    entries
}

fn write_leaf_entries(
    alloc: &mut PageAllocator,
    page_num: u32,
    entries: &[LeafEntry],
) -> Result<()> {
    let page = alloc.write_page(page_num)?;
    let off = PAGE_HEADER_SIZE;

    // Preserve prev/next links
    let prev = u32::from_le_bytes([page[off], page[off + 1], page[off + 2], page[off + 3]]);
    let next = u32::from_le_bytes([page[off + 4], page[off + 5], page[off + 6], page[off + 7]]);

    // Clear data area
    let page_size = page.len();
    page[off..page_size].fill(0);

    // Write metadata
    page[off..off + 4].copy_from_slice(&prev.to_le_bytes());
    page[off + 4..off + 8].copy_from_slice(&next.to_le_bytes());
    page[off + 8..off + 10].copy_from_slice(&(entries.len() as u16).to_le_bytes());

    let mut cursor = PAGE_HEADER_SIZE + LEAF_META_SIZE;
    let mut used = 0u16;

    for entry in entries {
        let entry_size = entry_byte_size(entry);
        page[cursor..cursor + 8].copy_from_slice(&entry.key.to_le_bytes());
        page[cursor + 8..cursor + 10].copy_from_slice(&(entry.value.len() as u16).to_le_bytes());
        page[cursor + 10] = if entry.deleted { 1 } else { 0 };
        cursor += LEAF_ENTRY_OVERHEAD;
        page[cursor..cursor + entry.value.len()].copy_from_slice(&entry.value);
        cursor += entry.value.len();
        used += entry_size as u16;
    }

    // Write used_bytes
    page[off + 10..off + 12].copy_from_slice(&used.to_le_bytes());

    // Update header item_count
    page[2..4].copy_from_slice(&(entries.len() as u16).to_le_bytes());

    Ok(())
}

fn set_leaf_prev(alloc: &mut PageAllocator, page_num: u32, prev: u32) -> Result<()> {
    let page = alloc.write_page(page_num)?;
    let off = PAGE_HEADER_SIZE;
    page[off..off + 4].copy_from_slice(&prev.to_le_bytes());
    Ok(())
}

fn set_leaf_next(alloc: &mut PageAllocator, page_num: u32, next: u32) -> Result<()> {
    let page = alloc.write_page(page_num)?;
    let off = PAGE_HEADER_SIZE + 4;
    page[off..off + 4].copy_from_slice(&next.to_le_bytes());
    Ok(())
}

// ── Interior page read/write ──────────────────────────────

fn read_interior_page(page: &[u8]) -> (Vec<u64>, Vec<u32>) {
    let off = PAGE_HEADER_SIZE;
    let key_count = u16::from_le_bytes([page[off], page[off + 1]]) as usize;
    let mut cursor = off + 2;

    // Read interleaved: child_0, key_0, child_1, key_1, ..., child_n
    let mut children = Vec::with_capacity(key_count + 1);
    let mut keys = Vec::with_capacity(key_count);

    // First child
    children.push(u32::from_le_bytes([
        page[cursor],
        page[cursor + 1],
        page[cursor + 2],
        page[cursor + 3],
    ]));
    cursor += 4;

    for _ in 0..key_count {
        keys.push(u64::from_le_bytes([
            page[cursor],
            page[cursor + 1],
            page[cursor + 2],
            page[cursor + 3],
            page[cursor + 4],
            page[cursor + 5],
            page[cursor + 6],
            page[cursor + 7],
        ]));
        cursor += 8;
        children.push(u32::from_le_bytes([
            page[cursor],
            page[cursor + 1],
            page[cursor + 2],
            page[cursor + 3],
        ]));
        cursor += 4;
    }

    (keys, children)
}

fn write_interior_page(
    alloc: &mut PageAllocator,
    page_num: u32,
    keys: &[u64],
    children: &[u32],
) -> Result<()> {
    let page = alloc.write_page(page_num)?;
    let off = PAGE_HEADER_SIZE;

    // Clear data area
    let page_size = page.len();
    page[off..page_size].fill(0);

    // key_count
    page[off..off + 2].copy_from_slice(&(keys.len() as u16).to_le_bytes());
    let mut cursor = off + 2;

    // First child
    page[cursor..cursor + 4].copy_from_slice(&children[0].to_le_bytes());
    cursor += 4;

    for i in 0..keys.len() {
        page[cursor..cursor + 8].copy_from_slice(&keys[i].to_le_bytes());
        cursor += 8;
        page[cursor..cursor + 4].copy_from_slice(&children[i + 1].to_le_bytes());
        cursor += 4;
    }

    // Update header item_count
    page[2..4].copy_from_slice(&(keys.len() as u16).to_le_bytes());

    Ok(())
}

fn read_interior_child(page: &[u8], index: usize) -> u32 {
    let off = PAGE_HEADER_SIZE + 2; // after key_count
                                    // Children are at: off + 0, off + 12, off + 24, ...
                                    // Interleaved: child(4) key(8) child(4) key(8) ...
                                    // child_i is at: off + i * 12 for i=0, off + 4 + 8 + (i-1)*12 for i>0
                                    // Actually: child_0 at off, then key_0(8)+child_1(4), key_1(8)+child_2(4), ...
                                    // child_i at: off + i * 12
    let pos = off + index * 12;
    u32::from_le_bytes([page[pos], page[pos + 1], page[pos + 2], page[pos + 3]])
}

fn find_child_for_key(page: &[u8], key: u64) -> u32 {
    let (keys, children) = read_interior_page(page);
    // Binary search: find the rightmost key < target
    let idx = keys.partition_point(|&k| k <= key);
    children[idx]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::embedded::page::{PageAllocator, PageType, DEFAULT_PAGE_SIZE};

    fn make_value(n: u64) -> Vec<u8> {
        format!("value-{n}").into_bytes()
    }

    #[test]
    fn test_insert_and_get_single() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        let mut tree = BPlusTree::new(&mut alloc, PageType::EntityLeaf, PageType::EntityInterior);

        tree.insert(&mut alloc, 42, b"hello").unwrap();
        let val = tree.get(&alloc, 42).unwrap();
        assert_eq!(val, Some(b"hello".to_vec()));
    }

    #[test]
    fn test_get_missing_key() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        let tree = BPlusTree::new(&mut alloc, PageType::EntityLeaf, PageType::EntityInterior);
        assert_eq!(tree.get(&alloc, 999).unwrap(), None);
    }

    #[test]
    fn test_insert_replace_existing() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        let mut tree = BPlusTree::new(&mut alloc, PageType::EntityLeaf, PageType::EntityInterior);

        tree.insert(&mut alloc, 1, b"first").unwrap();
        tree.insert(&mut alloc, 1, b"second").unwrap();

        let val = tree.get(&alloc, 1).unwrap();
        assert_eq!(val, Some(b"second".to_vec()));
    }

    #[test]
    fn test_insert_1000_get_all() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        let mut tree = BPlusTree::new(&mut alloc, PageType::EntityLeaf, PageType::EntityInterior);

        for i in 1..=1000u64 {
            tree.insert(&mut alloc, i, &make_value(i)).unwrap();
        }

        for i in 1..=1000u64 {
            let val = tree.get(&alloc, i).unwrap();
            assert_eq!(val, Some(make_value(i)), "key {i} not found");
        }
    }

    #[test]
    fn test_insert_reverse_order() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        let mut tree = BPlusTree::new(&mut alloc, PageType::EntityLeaf, PageType::EntityInterior);

        for i in (1..=500u64).rev() {
            tree.insert(&mut alloc, i, &make_value(i)).unwrap();
        }

        for i in 1..=500u64 {
            assert_eq!(tree.get(&alloc, i).unwrap(), Some(make_value(i)));
        }
    }

    #[test]
    fn test_delete_key() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        let mut tree = BPlusTree::new(&mut alloc, PageType::EntityLeaf, PageType::EntityInterior);

        tree.insert(&mut alloc, 1, b"a").unwrap();
        tree.insert(&mut alloc, 2, b"b").unwrap();
        tree.insert(&mut alloc, 3, b"c").unwrap();

        assert!(tree.delete(&mut alloc, 2).unwrap());
        assert_eq!(tree.get(&alloc, 2).unwrap(), None);
        assert_eq!(tree.get(&alloc, 1).unwrap(), Some(b"a".to_vec()));
        assert_eq!(tree.get(&alloc, 3).unwrap(), Some(b"c".to_vec()));
    }

    #[test]
    fn test_delete_nonexistent() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        let mut tree = BPlusTree::new(&mut alloc, PageType::EntityLeaf, PageType::EntityInterior);
        assert!(!tree.delete(&mut alloc, 999).unwrap());
    }

    #[test]
    fn test_delete_and_reinsert() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        let mut tree = BPlusTree::new(&mut alloc, PageType::EntityLeaf, PageType::EntityInterior);

        tree.insert(&mut alloc, 5, b"old").unwrap();
        tree.delete(&mut alloc, 5).unwrap();
        assert_eq!(tree.get(&alloc, 5).unwrap(), None);

        tree.insert(&mut alloc, 5, b"new").unwrap();
        assert_eq!(tree.get(&alloc, 5).unwrap(), Some(b"new".to_vec()));
    }

    #[test]
    fn test_scan_empty() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        let tree = BPlusTree::new(&mut alloc, PageType::EntityLeaf, PageType::EntityInterior);
        let results = tree.scan(&alloc).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_scan_returns_sorted_order() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        let mut tree = BPlusTree::new(&mut alloc, PageType::EntityLeaf, PageType::EntityInterior);

        // Insert out of order
        for &k in &[50u64, 10, 90, 30, 70, 20, 80, 40, 60, 100] {
            tree.insert(&mut alloc, k, &make_value(k)).unwrap();
        }

        let results = tree.scan(&alloc).unwrap();
        let keys: Vec<u64> = results.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100]);
    }

    #[test]
    fn test_scan_1000_sorted() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        let mut tree = BPlusTree::new(&mut alloc, PageType::EntityLeaf, PageType::EntityInterior);

        for i in 1..=1000u64 {
            tree.insert(&mut alloc, i, &make_value(i)).unwrap();
        }

        let results = tree.scan(&alloc).unwrap();
        assert_eq!(results.len(), 1000);
        for (idx, (key, val)) in results.iter().enumerate() {
            assert_eq!(*key, idx as u64 + 1);
            assert_eq!(*val, make_value(idx as u64 + 1));
        }
    }

    #[test]
    fn test_scan_skips_deleted() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        let mut tree = BPlusTree::new(&mut alloc, PageType::EntityLeaf, PageType::EntityInterior);

        for i in 1..=10u64 {
            tree.insert(&mut alloc, i, &make_value(i)).unwrap();
        }

        tree.delete(&mut alloc, 3).unwrap();
        tree.delete(&mut alloc, 7).unwrap();

        let results = tree.scan(&alloc).unwrap();
        let keys: Vec<u64> = results.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, vec![1, 2, 4, 5, 6, 8, 9, 10]);
    }

    #[test]
    fn test_split_creates_multiple_pages() {
        let mut alloc = PageAllocator::new(DEFAULT_PAGE_SIZE);
        let mut tree = BPlusTree::new(&mut alloc, PageType::EntityLeaf, PageType::EntityInterior);

        // Insert enough data to force splits (4KB pages, ~8+2+1+8 = 19 bytes per entry)
        // Usable = 4096 - 8 - 12 = 4076 bytes. 4076 / 19 ≈ 214 entries per leaf.
        // With 500 entries, we need at least 3 leaf pages.
        for i in 1..=500u64 {
            tree.insert(&mut alloc, i, &make_value(i)).unwrap();
        }

        // Tree should have grown beyond the single root leaf
        assert!(
            alloc.page_count() > 2,
            "expected multiple pages, got {}",
            alloc.page_count()
        );

        // All entries still retrievable
        for i in 1..=500u64 {
            assert_eq!(tree.get(&alloc, i).unwrap(), Some(make_value(i)), "key {i}");
        }
    }

    #[test]
    fn test_max_interior_keys() {
        let max = max_interior_keys(DEFAULT_PAGE_SIZE);
        // (4088 - 6) / 12 = 4082 / 12 = 340
        assert_eq!(max, 340);
    }
}
