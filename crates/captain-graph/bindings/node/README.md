# hora-graph-core for Node.js

Native Node.js binding for `hora-graph-core`, built with
[napi-rs](https://napi.rs).

## Install

```bash
npm install hora-graph-core
```

Node.js 16 or newer is required. npm selects the matching optional native
package when a prebuilt binary exists for the current platform. Build the
binding from source for targets not included in a published package.

## Quick Start

```js
const { HoraCore } = require('hora-graph-core');

const graph = HoraCore.newMemory();

const alice = graph.addEntity('person', 'Alice', { team: 'platform' });
const bob = graph.addEntity('person', 'Bob');
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

`HoraCore` uses factory methods rather than a public constructor:

- `HoraCore.newMemory(config?)` creates an in-memory graph.
- `HoraCore.open(path, config?)` opens or creates a file-backed graph.

For file-backed graphs, call `flush()` when the current state must be durable
immediately. `snapshot(destination)` writes a copy without changing the active
path.

## Exported API

- Entity CRUD: `addEntity`, `getEntity`, `updateEntity`, `deleteEntity`.
- Fact CRUD: `addFact`, `getFact`, `updateFact`, `invalidateFact`,
  `deleteFact`, `getEntityFacts`.
- Graph queries: `traverse`, `neighbors`, `timeline`, `factsAt`.
- Episodes and persistence: `addEpisode`, `flush`, `snapshot`, `stats`.

The generated [`index.d.ts`](index.d.ts) file is the TypeScript contract for
the packaged binding. The Rust exports in [`src/lib.rs`](src/lib.rs) remain the
implementation source of truth.

## Build from Source

```bash
cd crates/captain-graph/bindings/node
npm install
npm run build
npm test
```

## License

MIT OR Apache-2.0
