# hora-graph-core C FFI

C-compatible access to `hora-graph-core`, generated with
[cbindgen](https://github.com/mozilla/cbindgen).

The checked-in [`hora_graph_core.h`](hora_graph_core.h) header is the public ABI
source of truth. Functions return `NULL`, `0`, or `-1` on failure as documented
in that header; call `hora_last_error()` for the current thread's error text.

## Build

From the Captain repository:

```bash
cd crates/captain-graph/bindings/c
cargo build --release
```

The crate produces static and dynamic libraries named `hora_graph_ffi` using
the platform's normal extension (`.a`/`.lib`, `.so`/`.dylib`/`.dll`). Its build
script regenerates `hora_graph_core.h` from the Rust exports.

## Usage

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
    if (alice == 0 || bob == 0) {
        hora_free(graph);
        return 1;
    }

    uint64_t fact = hora_add_fact(
        graph, alice, bob, "knows", "Met at RustConf", 0.9f
    );
    if (fact == 0) {
        hora_free(graph);
        return 1;
    }

    hora_free(graph);
    return 0;
}
```

Heap-allocated entities, facts, search results, traversal results, ID arrays,
and strings must be released with their matching `hora_free_*` function. A
`HoraCore *` must be released with `hora_free()`.

## Supported Targets

The FFI crate builds wherever Rust can produce a C-compatible static or dynamic
library. The repository exercises Linux, macOS, and Windows targets; consumers
must compile or package the library for each target they ship.

## License

MIT OR Apache-2.0
