# hora-graph-core for Python

Python binding for `hora-graph-core`, built with
[PyO3](https://pyo3.rs) and [maturin](https://maturin.rs).

## Install

```bash
pip install hora-graph-core
```

The current binding supports CPython 3.9 through 3.13, matching its PyO3 0.22
dependency.

## Quick Start

```python
from hora_graph_core import HoraCore

graph = HoraCore.new_memory()

alice = graph.add_entity("person", "Alice", {"team": "platform"})
bob = graph.add_entity("person", "Bob")
fact = graph.add_fact(
    alice,
    bob,
    "knows",
    "Met at RustConf",
    0.9,
)

print(graph.get_fact(fact))
print(graph.neighbors(alice))
```

`HoraCore` uses factory methods rather than a public constructor:

- `HoraCore.new_memory(embedding_dims=0)` creates an in-memory graph.
- `HoraCore.open(path, embedding_dims=0)` opens or creates a file-backed graph.

For file-backed graphs, call `flush()` when the current state must be durable
immediately. `snapshot(destination)` writes a copy without changing the active
path.

## Exported API

- Entity and fact CRUD, including bi-temporal invalidation.
- Hybrid `search(query=None, embedding=None, top_k=10)`.
- `traverse`, `neighbors`, `timeline`, and `facts_at` graph queries.
- Spreading activation, memory phase, retrievability, review scheduling, and
  dark-node inspection.
- Episodes, snapshots, persistence, and summary statistics.

The checked-in
[`hora_graph_core.pyi`](hora_graph_core/hora_graph_core.pyi) file documents the
typed Python surface. The PyO3 methods in [`src/lib.rs`](src/lib.rs) are the
implementation source of truth.

## Build from Source

```bash
cd crates/captain-graph/bindings/python
maturin develop
pytest
```

## Supported Targets

Published wheel availability depends on the release. A source build requires a
Rust toolchain and a CPython version supported by PyO3 on Linux, macOS, or
Windows.

## License

MIT OR Apache-2.0
