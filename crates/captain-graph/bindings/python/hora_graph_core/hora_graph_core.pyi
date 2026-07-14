from typing import Optional

class HoraCore:
    @staticmethod
    def new_memory(embedding_dims: int = 0) -> "HoraCore": ...
    @staticmethod
    def open(path: str, embedding_dims: int = 0) -> "HoraCore": ...

    # Entities
    def add_entity(
        self,
        entity_type: str,
        name: str,
        properties: Optional[dict[str, str | int | float | bool]] = None,
        embedding: Optional[list[float]] = None,
    ) -> int: ...
    def get_entity(self, id: int) -> Optional[dict]: ...
    def update_entity(
        self,
        id: int,
        name: Optional[str] = None,
        entity_type: Optional[str] = None,
        properties: Optional[dict[str, str | int | float | bool]] = None,
        embedding: Optional[list[float]] = None,
    ) -> None: ...
    def delete_entity(self, id: int) -> None: ...

    # Facts
    def add_fact(
        self,
        source: int,
        target: int,
        relation: str,
        description: str = "",
        confidence: Optional[float] = None,
    ) -> int: ...
    def get_fact(self, id: int) -> Optional[dict]: ...
    def update_fact(
        self,
        id: int,
        confidence: Optional[float] = None,
        description: Optional[str] = None,
    ) -> None: ...
    def invalidate_fact(self, id: int) -> None: ...
    def delete_fact(self, id: int) -> None: ...
    def get_entity_facts(self, entity_id: int) -> list[dict]: ...

    # Search
    def search(
        self,
        query: Optional[str] = None,
        embedding: Optional[list[float]] = None,
        top_k: int = 10,
    ) -> list[dict]: ...

    # Traversal
    def traverse(self, start_id: int, depth: int = 3) -> dict: ...
    def neighbors(self, entity_id: int) -> list[int]: ...
    def timeline(self, entity_id: int) -> list[dict]: ...
    def facts_at(self, timestamp: int) -> list[dict]: ...

    # Spreading Activation
    def spread_activation(
        self,
        sources: list[tuple[int, float]],
        s_max: float = 1.6,
        w_total: float = 1.0,
        max_depth: int = 3,
        cutoff: float = 0.01,
    ) -> list[dict]: ...

    # Memory
    def get_memory_phase(self, entity_id: int) -> Optional[str]: ...
    def get_retrievability(self, entity_id: int) -> Optional[float]: ...
    def get_next_review_days(self, entity_id: int) -> Optional[float]: ...
    def dark_node_pass(self) -> int: ...
    def dark_nodes(self) -> list[int]: ...

    # Episodes
    def add_episode(
        self,
        source: str,
        session_id: str,
        entity_ids: list[int],
        fact_ids: list[int],
    ) -> int: ...

    # Persistence
    def flush(self) -> None: ...
    def snapshot(self, dest: str) -> None: ...

    # Stats
    def stats(self) -> dict: ...
