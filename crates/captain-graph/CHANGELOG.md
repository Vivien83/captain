# Changelog

All notable changes to hora-graph-core are documented here.

## [1.0.0] - 2026-03-03

### Release

First stable release. API and binary format (.hora v2) are frozen.

### v1.0e - Security Review
- Bounds checks on format deserialization (MAX_ALLOC caps)
- UAF fix in mmap `remap()` (null pointer after munmap)
- WAL replay DoS cap (MAX_REPLAY_PAGE)
- Release-safe `assert_eq!` on SIMD vector lengths
- SAFETY documentation on all 10 unsafe blocks

### v1.0d - Performance Validation
- `get_entity`: 240 ns -> 42 ns (Vec-indexed storage)
- BM25 search: 3.7 ms -> 668 us (inline doc_len, dense Vec scores)
- Dream cycle: 110 s -> 26 ms (O(n*k) linking cap)
- Open 1M: 559 ms -> 94 ms (lazy BM25, bulk embedding read)
- Lazy activation recording (flush before queries)

### v1.0c - Documentation
- Complete rustdoc on all public types and methods
- `cargo doc` with zero warnings

### v1.0b - Format Freeze
- Binary format v2 with CRC32 header checksum
- `verify_file()` integrity checker
- Backward-compatible with format v1

### v1.0a - API Freeze
- `#[non_exhaustive]` on all public enums
- `#[must_use]` on Result-returning methods

## [0.7.0] - 2026-03-02

### Bindings & CI
- **v0.7a**: Python binding (PyO3 + maturin, 21 tests)
- **v0.7b**: WASM binding (wasm-bindgen, 22 tests)
- **v0.7c**: C FFI binding (extern "C", opaque pointer, `hora_last_error()`)
- **v0.7d**: GitHub Actions CI/CD (multi-platform matrix)

## [0.6.0] - 2026-03-02

### Backend Abstraction
- **v0.6a**: SQLite backend with FTS5
- **v0.6b**: PostgreSQL backend with tsvector
- **v0.6c**: Conformance test suite across backends

## [0.5.0] - 2026-03-02

### Robustness (Embedded Storage)
- **v0.5a**: Page allocator with freelist and CRC32 integrity
- **v0.5b**: B+ tree index (insert, get, delete, range scan)
- **v0.5c**: WAL (Write-Ahead Log)
- **v0.5d**: mmap reader + ReadReader fallback
- **v0.5e**: Crash recovery with Database struct
- **v0.5f**: Compaction (incremental + full vacuum)
- **v0.5g**: Multi-statement transactions (begin/commit/rollback)

## [0.4.0] - 2026-03-01

### Memory Consolidation
- **v0.4a**: Episode management with filtered queries
- **v0.4b**: SHY homeostatic downscaling
- **v0.4c**: Interleaved replay (mixed recent/old episodes)
- **v0.4d**: CLS transfer (episodic to semantic facts)
- **v0.4e**: Memory linking (temporal co-creation edges)
- **v0.4f**: Dream cycle (6-step consolidation pipeline)

## [0.3.0] - 2026-03-01

### Bio-Inspired Memory
- **v0.3a**: ACT-R base-level learning (Petrov decay approximation)
- **v0.3b**: Spreading activation with fan effect
- **v0.3c**: Reconsolidation window (Nader 2000 destabilization model)
- **v0.3d**: Dark nodes (Rac1-inspired active forgetting)
- **v0.3e**: FSRS scheduling (retrievability + stability tracking)

## [0.2.0] - 2026-02-28

### Perception (Search)
- **v0.2a**: SIMD vector search (NEON/AVX2 cosine, brute-force top-k)
- **v0.2b**: BM25+ inverted index (full-text search)
- **v0.2c**: Hybrid search RRF (Reciprocal Rank Fusion)
- **v0.2d**: Triple deduplication (name, Jaccard, cosine)

## [0.1.0] - 2026-02-28

### Foundation
- **v0.1a**: Core types and structures
- **v0.1b**: CRUD operations (add, get, update, delete entities and facts)
- **v0.1c**: Graph traversal (BFS, timeline, `facts_at`)
- **v0.1d**: Embedded storage (binary format, flush, open, snapshot)
- **v0.1e**: Node.js binding (napi-rs)
- **v0.1f**: Criterion benchmarks
