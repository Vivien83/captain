# hora-graph-wasm

WebAssembly binding for `hora-graph-core`, built with
[wasm-bindgen](https://rustwasm.github.io/wasm-bindgen/) for browsers and edge
runtimes.

## Install

```bash
npm install hora-graph-wasm
```

## Quick Start

```js
import init, { HoraCore } from 'hora-graph-wasm';

await init();
const graph = HoraCore.newMemory();

const alice = graph.addEntity('person', 'Alice', { team: 'platform' }, undefined);
const bob = graph.addEntity('person', 'Bob', {}, undefined);
const fact = graph.addFact(
  alice,
  bob,
  'knows',
  'Met at RustConf',
  0.9,
);

console.log(graph.getFact(fact));
console.log(graph.neighbors(alice));
```

`HoraCore.newMemory(embeddingDims?)` is the supported constructor. The WASM
binding is deliberately in-memory only and does not expose the Rust file,
SQLite, or PostgreSQL backends.

## Exported API

- Entity CRUD: `addEntity`, `getEntity`, `updateEntity`, `deleteEntity`.
- Fact CRUD: `addFact`, `getFact`, `updateFact`, `invalidateFact`,
  `deleteFact`, `getEntityFacts`.
- Search and traversal: `search`, `traverse`, `neighbors`.
- Episodes and inspection: `addEpisode`, `stats`.

The wasm-bindgen exports in [`src/lib.rs`](src/lib.rs) are the API source of
truth. Regenerate the JavaScript and TypeScript glue whenever those exports
change.

## Build from Source

```bash
cd crates/captain-graph/bindings/wasm
wasm-pack build --target web
wasm-pack test --headless --chrome
```

## License

MIT OR Apache-2.0
