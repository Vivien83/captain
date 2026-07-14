<div align="center">

# hora-graph-core

**Bio-inspired embedded knowledge graph engine in Rust.**

*Your memory never sleeps.*

[![CI](https://github.com/Vivien83/hora-graph-core/actions/workflows/ci.yml/badge.svg)](https://github.com/Vivien83/hora-graph-core/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/hora-graph-core.svg)](https://crates.io/crates/hora-graph-core)
[![npm](https://img.shields.io/npm/v/hora-graph-core.svg)](https://www.npmjs.com/package/hora-graph-core)
[![PyPI](https://img.shields.io/pypi/v/hora-graph-core.svg)](https://pypi.org/project/hora-graph-core/)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE-MIT)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org)

[Developer Guide](docs/GUIDE.md) &#183; [Guide FR](docs/GUIDE-FR.md) &#183; [Performance](docs/PERFORMANCE.md) &#183; [Releases](https://github.com/Vivien83/hora-graph-core/releases)

</div>

`hora-graph-core` is the embedded graph and memory engine used by Captain. It
combines bi-temporal facts, graph traversal, text and vector search, activation
decay, spaced-repetition state, and persistence behind one Rust API. The
default crate build has no third-party runtime dependency; SQLite and
PostgreSQL support are optional Cargo features.

Bindings are maintained for Node.js, Python, WebAssembly, and C. Their public
APIs intentionally follow each language's naming conventions, so use the
binding-specific README rather than translating Rust method names mechanically.

## Quick Start

### Rust

```bash
cargo add hora-graph-core
```

```rust
use hora_graph_core::{HoraConfig, HoraCore, TraverseOpts};

fn main() -> hora_graph_core::Result<()> {
    let mut graph = HoraCore::new(HoraConfig::default())?;

    let alice = graph.add_entity("person", "Alice", None, None)?;
    let bob = graph.add_entity("person", "Bob", None, None)?;
    graph.add_fact(alice, bob, "knows", "Met at RustConf", Some(0.9))?;

    let result = graph.traverse(alice, TraverseOpts { depth: 3 })?;
    assert!(result.entity_ids.contains(&bob));

    let hits = graph.text_search("Alice", 5)?;
    assert!(!hits.is_empty());
    Ok(())
}
```

Open a file-backed graph with `HoraCore::open(path, config)` and call `flush()`
when the current state must be durable immediately.

### Node.js

```bash
npm install hora-graph-core
```

```js
const { HoraCore } = require('hora-graph-core');

const graph = HoraCore.newMemory();
const alice = graph.addEntity('person', 'Alice');
const bob = graph.addEntity('person', 'Bob');
graph.addFact(alice, bob, 'knows', 'Met at RustConf', 0.9);

console.log(graph.neighbors(alice));
```

See [the Node.js binding README](bindings/node/README.md) for persistence and
the supported prebuilt targets.

### Python

```bash
pip install hora-graph-core
```

```python
from hora_graph_core import HoraCore

graph = HoraCore.new_memory()
alice = graph.add_entity("person", "Alice")
bob = graph.add_entity("person", "Bob")
graph.add_fact(alice, bob, "knows", "Met at RustConf", 0.9)

print(graph.neighbors(alice))
```

See [the Python binding README](bindings/python/README.md) for persistence,
search, and supported Python versions.

### WebAssembly

```bash
npm install hora-graph-wasm
```

```js
import init, { HoraCore } from 'hora-graph-wasm';

await init();
const graph = HoraCore.newMemory();
const alice = graph.addEntity('person', 'Alice', {}, undefined);
const bob = graph.addEntity('person', 'Bob', {}, undefined);
graph.addFact(alice, bob, 'knows', 'Met at RustConf', 0.9);

console.log(graph.neighbors(alice));
```

The WASM binding is in-memory only. See its
[binding README](bindings/wasm/README.md) for the exported API.

### C

```c
#include "hora_graph_core.h"

int main(void) {
    HoraCore *graph = hora_new(0);
    if (graph == NULL) {
        return 1;
    }

    uint64_t alice = hora_add_entity(
        graph, "person", "Alice", NULL, NULL, 0
    );
    uint64_t bob = hora_add_entity(
        graph, "person", "Bob", NULL, NULL, 0
    );
    hora_add_fact(graph, alice, bob, "knows", "Met at RustConf", 0.9f);

    hora_free(graph);
    return 0;
}
```

The generated header is the C ABI source of truth. See the
[C binding README](bindings/c/README.md) for build and error-handling details.

## Capabilities

### Graph and Search

- Bi-temporal directed facts with validity and creation timestamps.
- Breadth-first traversal, neighbors, timelines, and point-in-time facts.
- Exact-name, token-overlap, and embedding-aware deduplication.
- BM25+ text search, cosine vector search, and reciprocal-rank fusion.

### Memory Model

- ACT-R-inspired activation and spreading activation.
- Reconsolidation state and active forgetting for low-activation entities.
- FSRS stability, retrievability, and review scheduling.
- A six-step consolidation cycle covering downscaling, replay, transfer,
  linking, pruning, and statistics.

### Persistence

- `HoraCore::new` uses the in-process memory backend.
- `HoraCore::open` loads the portable graph format into memory; `flush()` writes
  it through a temporary file and atomic rename, and `snapshot()` copies it.
- The default `storage::embedded` module provides the lower-level page
  allocator, B+ tree, WAL, mmap, recovery, transaction, and compaction APIs.
- `storage::sqlite` and `storage::pg` are available through the optional
  `sqlite` and `postgres` Cargo features.

## Architecture

```text
hora-graph-core/
├── src/
│   ├── lib.rs                 Unified public API
│   ├── core/                  Entities, facts, episodes, deduplication
│   ├── memory/                Activation, reconsolidation, FSRS, consolidation
│   ├── search/                Vector, BM25+, and hybrid search
│   └── storage/
│       ├── memory.rs          In-memory backend
│       ├── format.rs          Embedded serialization format
│       ├── sqlite.rs          Optional SQLite backend
│       ├── pg.rs              Optional PostgreSQL backend
│       └── embedded/          Pages, B+ tree, WAL, mmap, and recovery
├── bindings/
│   ├── node/                  napi-rs
│   ├── python/                PyO3 and maturin
│   ├── wasm/                  wasm-bindgen
│   └── c/                     cbindgen
├── benches/                   Criterion benchmarks
└── tests/                     Backend conformance tests
```

## Performance

Criterion benchmarks live in `benches/`, with methodology and the recorded
benchmark environment in [docs/PERFORMANCE.md](docs/PERFORMANCE.md). Run them
on the target hardware before making latency or throughput assumptions:

```bash
cargo bench -p hora-graph-core
```

## Building from Source

From the Captain workspace:

```bash
cargo test -p hora-graph-core
cargo test -p hora-graph-core --all-features
cargo bench -p hora-graph-core
```

From the standalone project:

```bash
git clone https://github.com/Vivien83/hora-graph-core.git
cd hora-graph-core
cargo test --all-features
```

**Minimum Rust version:** 1.70.

## Neuroscience References

The memory subsystem is informed by published work on:

| Model | Reference | Module |
|:---|:---|:---|
| ACT-R base-level learning | Anderson & Lebiere (1998) | `memory/activation.rs` |
| Petrov decay approximation | Petrov (2006) | `memory/activation.rs` |
| Spreading activation | Anderson (1983) | `memory/spreading.rs` |
| Memory reconsolidation | Nader, Schafe & Le Doux (2000) | `memory/reconsolidation.rs` |
| Rac1 active forgetting | Shuai et al. (2010) | `memory/dark_nodes.rs` |
| FSRS spaced repetition | Ye (2023) | `memory/fsrs.rs` |
| Synaptic homeostasis | Tononi & Cirelli (2003) | `memory/consolidation.rs` |
| Complementary learning systems | McClelland et al. (1995) | `memory/consolidation.rs` |

## License

Licensed under either [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at
your option.
