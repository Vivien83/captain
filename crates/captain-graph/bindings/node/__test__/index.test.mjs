import { describe, it, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { HoraCore } from '../index.js';

function tmpDir() {
  return mkdtempSync(join(tmpdir(), 'hora-test-'));
}

describe('HoraCore', () => {
  // ── Entity CRUD ────────────────────────────────

  it('creates and retrieves an entity', () => {
    const hora = HoraCore.newMemory();
    const id = hora.addEntity('project', 'hora', { language: 'Rust' });
    const entity = hora.getEntity(id);

    assert.equal(entity.name, 'hora');
    assert.equal(entity.entityType, 'project');
    assert.equal(entity.properties.language, 'Rust');
    assert.ok(entity.createdAt > 0);
  });

  it('returns null for missing entity', () => {
    const hora = HoraCore.newMemory();
    assert.equal(hora.getEntity(999), null);
  });

  it('updates an entity', () => {
    const hora = HoraCore.newMemory();
    const id = hora.addEntity('project', 'hora');
    hora.updateEntity(id, { name: 'hora-graph-core' });

    const entity = hora.getEntity(id);
    assert.equal(entity.name, 'hora-graph-core');
    assert.equal(entity.entityType, 'project'); // unchanged
  });

  it('deletes an entity with cascade', () => {
    const hora = HoraCore.newMemory();
    const a = hora.addEntity('a', 'x');
    const b = hora.addEntity('b', 'y');
    const factId = hora.addFact(a, b, 'rel', 'desc');

    hora.deleteEntity(a);

    assert.equal(hora.getEntity(a), null);
    assert.equal(hora.getFact(factId), null);
    assert.notEqual(hora.getEntity(b), null);
  });

  // ── Fact CRUD ──────────────────────────────────

  it('creates and retrieves a fact', () => {
    const hora = HoraCore.newMemory();
    const a = hora.addEntity('project', 'hora');
    const b = hora.addEntity('language', 'Rust');
    const factId = hora.addFact(a, b, 'built_with', 'hora uses Rust', 0.95);

    const fact = hora.getFact(factId);
    assert.equal(fact.relationType, 'built_with');
    assert.equal(fact.description, 'hora uses Rust');
    assert.ok(Math.abs(fact.confidence - 0.95) < 0.01);
    assert.ok(fact.validAt > 0);
    assert.equal(fact.invalidAt, 0);
  });

  it('invalidates a fact (bi-temporal)', () => {
    const hora = HoraCore.newMemory();
    const a = hora.addEntity('a', 'x');
    const b = hora.addEntity('b', 'y');
    const factId = hora.addFact(a, b, 'rel', 'desc');

    hora.invalidateFact(factId);

    const fact = hora.getFact(factId);
    assert.ok(fact.invalidAt > 0);
  });

  it('deletes a fact', () => {
    const hora = HoraCore.newMemory();
    const a = hora.addEntity('a', 'x');
    const b = hora.addEntity('b', 'y');
    const factId = hora.addFact(a, b, 'rel', 'desc');

    hora.deleteFact(factId);
    assert.equal(hora.getFact(factId), null);
  });

  it('gets entity facts bidirectionally', () => {
    const hora = HoraCore.newMemory();
    const a = hora.addEntity('a', 'x');
    const b = hora.addEntity('b', 'y');
    hora.addFact(a, b, 'rel', 'desc');

    assert.equal(hora.getEntityFacts(a).length, 1);
    assert.equal(hora.getEntityFacts(b).length, 1);
  });

  // ── Traversal ──────────────────────────────────

  it('BFS traversal with depth limit', () => {
    const hora = HoraCore.newMemory();
    const a = hora.addEntity('node', 'A');
    const b = hora.addEntity('node', 'B');
    const c = hora.addEntity('node', 'C');
    const d = hora.addEntity('node', 'D');

    hora.addFact(a, b, 'link', 'A->B');
    hora.addFact(b, c, 'link', 'B->C');
    hora.addFact(c, d, 'link', 'C->D');

    const result = hora.traverse(a, { depth: 2 });
    assert.equal(result.entityIds.length, 3); // A, B, C
    assert.ok(result.entityIds.includes(a));
    assert.ok(result.entityIds.includes(b));
    assert.ok(result.entityIds.includes(c));
    assert.ok(!result.entityIds.includes(d));
  });

  it('neighbors returns direct connections', () => {
    const hora = HoraCore.newMemory();
    const a = hora.addEntity('node', 'A');
    const b = hora.addEntity('node', 'B');
    const c = hora.addEntity('node', 'C');
    hora.addFact(a, b, 'link', 'A->B');
    hora.addFact(a, c, 'link', 'A->C');

    const neighbors = hora.neighbors(a);
    assert.equal(neighbors.length, 2);
    assert.ok(neighbors.includes(b));
    assert.ok(neighbors.includes(c));
  });

  it('timeline returns facts sorted by valid_at', () => {
    const hora = HoraCore.newMemory();
    const a = hora.addEntity('person', 'Alice');
    const b = hora.addEntity('company', 'Acme');
    const c = hora.addEntity('company', 'BigCorp');

    hora.addFact(a, b, 'works_at', 'Alice at Acme');
    hora.addFact(a, c, 'works_at', 'Alice at BigCorp');

    const tl = hora.timeline(a);
    assert.equal(tl.length, 2);
    assert.ok(tl[0].validAt <= tl[1].validAt);
  });

  // ── Episodes ───────────────────────────────────

  it('adds an episode', () => {
    const hora = HoraCore.newMemory();
    const a = hora.addEntity('project', 'hora');
    const epId = hora.addEpisode('conversation', 'sess-1', [a], []);
    assert.equal(epId, 1);

    const stats = hora.stats();
    assert.equal(stats.episodes, 1);
  });

  // ── Persistence ────────────────────────────────

  it('roundtrip: flush and reopen', () => {
    const dir = tmpDir();
    const path = join(dir, 'test.hora');

    try {
      const hora = HoraCore.open(path);
      const id = hora.addEntity('project', 'hora', { language: 'Rust' });
      hora.addFact(id, id, 'self', 'self-ref');
      hora.flush();

      const hora2 = HoraCore.open(path);
      const stats = hora2.stats();
      assert.equal(stats.entities, 1);
      assert.equal(stats.edges, 1);

      const entity = hora2.getEntity(id);
      assert.equal(entity.name, 'hora');
      assert.equal(entity.properties.language, 'Rust');
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  it('snapshot creates independent copy', () => {
    const dir = tmpDir();
    const snapPath = join(dir, 'snapshot.hora');

    try {
      const hora = HoraCore.newMemory();
      hora.addEntity('project', 'hora');
      hora.snapshot(snapPath);

      const hora2 = HoraCore.open(snapPath);
      assert.equal(hora2.stats().entities, 1);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  // ── Stats ──────────────────────────────────────

  it('returns correct stats', () => {
    const hora = HoraCore.newMemory();
    const a = hora.addEntity('a', 'x');
    const b = hora.addEntity('b', 'y');
    hora.addFact(a, b, 'rel', 'desc');

    const stats = hora.stats();
    assert.equal(stats.entities, 2);
    assert.equal(stats.edges, 1);
    assert.equal(stats.episodes, 0);
  });
});
