"""Tests for hora-graph-core Python binding."""
import os
import tempfile

import pytest
from hora_graph_core import HoraCore


# ── Entity CRUD ───────────────────────────────────────────────


def test_add_and_get_entity():
    h = HoraCore.new_memory()
    eid = h.add_entity("language", "Rust")
    entity = h.get_entity(eid)
    assert entity is not None
    assert entity["name"] == "Rust"
    assert entity["entity_type"] == "language"


def test_get_entity_not_found():
    h = HoraCore.new_memory()
    assert h.get_entity(999) is None


def test_update_entity():
    h = HoraCore.new_memory()
    eid = h.add_entity("language", "Rust")
    h.update_entity(eid, name="Rust-lang")
    entity = h.get_entity(eid)
    assert entity["name"] == "Rust-lang"


def test_delete_entity():
    h = HoraCore.new_memory()
    eid = h.add_entity("test", "to-delete")
    h.delete_entity(eid)
    assert h.get_entity(eid) is None


# ── Properties (typed, not string-only) ───────────────────────


def test_properties_round_trip():
    h = HoraCore.new_memory()
    props = {"language": "Rust", "stars": 42, "score": 9.5, "active": True}
    eid = h.add_entity("project", "hora", properties=props)
    entity = h.get_entity(eid)
    p = entity["properties"]
    assert p["language"] == "Rust"
    assert p["stars"] == 42
    assert p["score"] == 9.5
    assert p["active"] is True


# ── Embeddings ────────────────────────────────────────────────


def test_embedding_round_trip():
    h = HoraCore.new_memory(embedding_dims=4)
    emb = [1.0, -0.5, 0.0, 3.14]
    eid = h.add_entity("vec", "test", embedding=emb)
    entity = h.get_entity(eid)
    assert entity["embedding"] is not None
    assert len(entity["embedding"]) == 4
    assert abs(entity["embedding"][0] - 1.0) < 1e-6
    assert abs(entity["embedding"][3] - 3.14) < 1e-5


def test_no_embedding():
    h = HoraCore.new_memory()
    eid = h.add_entity("test", "no-emb")
    entity = h.get_entity(eid)
    assert entity["embedding"] is None


# ── Facts (edges) ─────────────────────────────────────────────


def test_add_and_get_fact():
    h = HoraCore.new_memory()
    a = h.add_entity("node", "A")
    b = h.add_entity("node", "B")
    fid = h.add_fact(a, b, "knows", "they met")
    fact = h.get_fact(fid)
    assert fact is not None
    assert fact["source"] == a
    assert fact["target"] == b
    assert fact["relation_type"] == "knows"


def test_update_fact():
    h = HoraCore.new_memory()
    a = h.add_entity("node", "A")
    b = h.add_entity("node", "B")
    fid = h.add_fact(a, b, "knows", "old")
    h.update_fact(fid, description="new description")
    fact = h.get_fact(fid)
    assert fact["description"] == "new description"


def test_invalidate_fact():
    h = HoraCore.new_memory()
    a = h.add_entity("node", "A")
    b = h.add_entity("node", "B")
    fid = h.add_fact(a, b, "knows", "")
    h.invalidate_fact(fid)
    fact = h.get_fact(fid)
    assert fact["invalid_at"] != 0


def test_delete_fact():
    h = HoraCore.new_memory()
    a = h.add_entity("node", "A")
    b = h.add_entity("node", "B")
    fid = h.add_fact(a, b, "knows", "")
    h.delete_fact(fid)
    assert h.get_fact(fid) is None


def test_get_entity_facts():
    h = HoraCore.new_memory()
    a = h.add_entity("node", "A")
    b = h.add_entity("node", "B")
    c = h.add_entity("node", "C")
    h.add_fact(a, b, "knows", "")
    h.add_fact(c, a, "follows", "")
    facts = h.get_entity_facts(a)
    assert len(facts) == 2


# ── Search ────────────────────────────────────────────────────


def test_text_search():
    h = HoraCore.new_memory()
    h.add_entity("service", "authentication")
    h.add_entity("service", "database")
    results = h.search(query="authentication", top_k=5)
    assert len(results) >= 1
    assert results[0]["entity_id"] is not None


def test_vector_search():
    h = HoraCore.new_memory(embedding_dims=4)
    h.add_entity("vec", "close", embedding=[1.0, 0.0, 0.0, 0.0])
    h.add_entity("vec", "far", embedding=[0.0, 0.0, 0.0, 1.0])
    results = h.search(embedding=[1.0, 0.0, 0.0, 0.0], top_k=2)
    assert len(results) == 2
    # "close" should rank first (higher cosine similarity)
    assert results[0]["score"] >= results[1]["score"]


# ── Traversal ─────────────────────────────────────────────────


def test_traverse():
    h = HoraCore.new_memory()
    a = h.add_entity("node", "A")
    b = h.add_entity("node", "B")
    c = h.add_entity("node", "C")
    h.add_fact(a, b, "knows", "")
    h.add_fact(b, c, "knows", "")
    result = h.traverse(a, depth=2)
    assert a in result["entity_ids"]
    assert b in result["entity_ids"]
    assert c in result["entity_ids"]


def test_neighbors():
    h = HoraCore.new_memory()
    a = h.add_entity("node", "A")
    b = h.add_entity("node", "B")
    h.add_fact(a, b, "knows", "")
    nbrs = h.neighbors(a)
    assert b in nbrs


# ── Episodes ──────────────────────────────────────────────────


def test_add_episode():
    h = HoraCore.new_memory()
    a = h.add_entity("node", "A")
    b = h.add_entity("node", "B")
    fid = h.add_fact(a, b, "knows", "")
    ep_id = h.add_episode("conversation", "sess-1", [a, b], [fid])
    assert ep_id >= 1


def test_invalid_episode_source():
    h = HoraCore.new_memory()
    with pytest.raises(ValueError, match="unknown source"):
        h.add_episode("invalid", "sess", [], [])


# ── Persistence ───────────────────────────────────────────────


def test_flush_and_reopen():
    with tempfile.TemporaryDirectory() as tmpdir:
        path = os.path.join(tmpdir, "test.hora")
        h = HoraCore.open(path)
        eid = h.add_entity("project", "hora", properties={"lang": "Rust"})
        h.flush()

        h2 = HoraCore.open(path)
        entity = h2.get_entity(eid)
        assert entity is not None
        assert entity["name"] == "hora"
        assert entity["properties"]["lang"] == "Rust"


# ── Stats ─────────────────────────────────────────────────────


def test_stats():
    h = HoraCore.new_memory()
    s = h.stats()
    assert s["entities"] == 0
    assert s["edges"] == 0
    assert s["episodes"] == 0

    h.add_entity("node", "A")
    h.add_entity("node", "B")
    s = h.stats()
    assert s["entities"] == 2


# ── Error handling ────────────────────────────────────────────


def test_entity_not_found_error():
    h = HoraCore.new_memory()
    with pytest.raises(ValueError):
        h.delete_entity(999)
