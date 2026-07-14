# hora-graph-core — Guide Developpeur

Moteur de knowledge graph embarque en Rust pur, bio-inspire, sans dependance runtime.
Facts bi-temporels, recherche plein texte BM25, activation ACT-R, reconsolidation,
dark nodes, planification FSRS et cycle de reve pour la consolidation memoire.

---

## 1. Installation

Ajoutez a votre `Cargo.toml` :

```toml
[dependencies]
hora-graph-core = { path = "../hora-graph-core" }
```

Ou depuis un depot git :

```toml
[dependencies]
hora-graph-core = { git = "https://github.com/Vivien83/hora-graph-core.git", tag = "v1.0.0" }
```

## 2. Demarrage rapide

```rust
use hora_graph_core::{HoraCore, HoraConfig, props};

fn main() -> hora_graph_core::Result<()> {
    // Creer un graphe en memoire
    let mut hora = HoraCore::new(HoraConfig::default())?;

    // Ajouter des entites
    let alice = hora.add_entity("person", "Alice", Some(props! { "role" => "engineer" }), None)?;
    let rust  = hora.add_entity("language", "Rust", None, None)?;

    // Ajouter un fait (arete orientee)
    let _fact = hora.add_fact(alice, rust, "uses", "Alice utilise Rust quotidiennement", None)?;

    // Recherche
    let hits = hora.text_search("Rust", 5)?;
    println!("Trouve {} resultats", hits.len());

    // Persister sur disque
    let mut hora = HoraCore::open("mon_graphe.hora", HoraConfig::default())?;
    let _id = hora.add_entity("demo", "hello", None, None)?;
    hora.flush()?;

    Ok(())
}
```

## 3. Concepts

### Entites

Une entite est un noeud dans le graphe :

| Champ | Type | Description |
|-------|------|-------------|
| `id` | `EntityId(u64)` | ID unique auto-genere |
| `entity_type` | `String` | Label semantique (ex: `"person"`, `"concept"`) |
| `name` | `String` | Nom lisible |
| `properties` | `HashMap<String, PropertyValue>` | Metadonnees cle-valeur |
| `embedding` | `Option<Vec<f32>>` | Vecteur optionnel pour la recherche de similarite |
| `created_at` | `i64` | Timestamp Unix en ms (temps systeme) |

### Faits (Aretes)

Un fait est une relation orientee et bi-temporelle entre deux entites :

| Champ | Type | Description |
|-------|------|-------------|
| `id` | `EdgeId(u64)` | ID unique auto-genere |
| `source` | `EntityId` | Entite d'origine |
| `target` | `EntityId` | Entite de destination |
| `relation_type` | `String` | Label (ex: `"knows"`, `"works_at"`) |
| `description` | `String` | Description lisible |
| `confidence` | `f32` | Score de confiance `[0.0, 1.0]` |
| `valid_at` | `i64` | Quand ce fait est devenu vrai (temps monde) |
| `invalid_at` | `i64` | Quand il a cesse d'etre vrai ; `0` = toujours valide |
| `created_at` | `i64` | Quand il a ete enregistre (temps systeme) |

### Modele bi-temporel

Chaque fait a deux axes temporels :
- **Temps monde** (`valid_at` / `invalid_at`) : quand le fait est vrai dans la realite
- **Temps systeme** (`created_at`) : quand le fait a ete enregistre

Utilisez `invalidate_fact()` pour un soft-delete (fixe `invalid_at` a maintenant). Utilisez `delete_fact()` pour un hard-delete.

### Episodes

Un episode est un instantane d'une interaction — un groupe d'IDs d'entites et de faits avec une source et une session :

| Champ | Type | Description |
|-------|------|-------------|
| `id` | `u64` | ID unique auto-genere |
| `source` | `EpisodeSource` | `Conversation`, `Document` ou `Api` |
| `session_id` | `String` | Regroupe les episodes d'une meme session |
| `entity_ids` | `Vec<EntityId>` | Entites referencees |
| `fact_ids` | `Vec<EdgeId>` | Faits references |
| `consolidation_count` | `u32` | Nombre de consolidations de cet episode |

### Valeurs de proprietes

```rust
use hora_graph_core::PropertyValue;

// Types supportes
PropertyValue::String("hello".into())
PropertyValue::Int(42)
PropertyValue::Float(3.14)
PropertyValue::Bool(true)

// Construction avec la macro props!
use hora_graph_core::props;
let p = props! {
    "name" => "Alice",
    "age" => 30,
    "active" => true,
};
```

## 4. Operations CRUD

### Entites

```rust
use hora_graph_core::{HoraCore, HoraConfig, EntityUpdate, props};

let mut hora = HoraCore::new(HoraConfig::default())?;

// Creer
let id = hora.add_entity("person", "Alice",
    Some(props! { "email" => "alice@example.com" }),
    None, // pas d'embedding
)?;

// Lire
let entity = hora.get_entity(id)?.expect("existe");
assert_eq!(entity.name, "Alice");

// Mettre a jour (partiel — seuls les champs Some sont modifies)
hora.update_entity(id, EntityUpdate {
    name: Some("Alice Smith".into()),
    properties: Some(props! { "email" => "alice@smith.com", "phone" => "555-0100" }),
    ..Default::default()
})?;

// Supprimer (en cascade : supprime toutes les aretes connectees)
hora.delete_entity(id)?;
```

### Faits

```rust
let alice = hora.add_entity("person", "Alice", None, None)?;
let bob   = hora.add_entity("person", "Bob", None, None)?;

// Creer un fait
let fact_id = hora.add_fact(alice, bob, "knows", "Rencontre a RustConf 2025", Some(0.9))?;

// Lire
let fact = hora.get_fact(fact_id)?.expect("existe");
assert_eq!(fact.relation_type, "knows");

// Obtenir tous les faits d'une entite (dans les deux sens)
let facts = hora.get_entity_facts(alice)?;

// Mettre a jour un fait (partiel)
use hora_graph_core::FactUpdate;
hora.update_fact(fact_id, FactUpdate {
    confidence: Some(0.95),
    description: Some("Rencontre a RustConf 2025 — devenus proches".into()),
})?;

// Soft-delete (bi-temporel : fixe invalid_at = maintenant)
hora.invalidate_fact(fact_id)?;

// Hard-delete
// hora.delete_fact(fact_id)?;
```

### Episodes

```rust
use hora_graph_core::EpisodeSource;

let ep_id = hora.add_episode(
    EpisodeSource::Conversation,
    "session-001",
    &[alice, bob],      // IDs d'entites
    &[fact_id],         // IDs de faits
)?;

// Lire
let episode = hora.get_episode(ep_id)?.expect("existe");

// Filtrer
let episodes = hora.get_episodes(
    Some("session-001"), // filtre session_id
    None,                // filtre source
    None,                // depuis (epoch ms)
    None,                // jusqu'a (epoch ms)
)?;

// Incrementer le compteur de consolidation (utilise par le replay du dream cycle)
hora.increment_consolidation(ep_id)?;
```

## 5. Recherche

### Recherche plein texte BM25

Recherche dans les noms d'entites et toutes les proprietes textuelles. Scoring BM25+.

```rust
let _id = hora.add_entity("doc", "Programmation Rust",
    Some(props! { "body" => "Rust est un langage de programmation systeme" }),
    None,
)?;

let hits = hora.text_search("programmation", 10)?;
for hit in &hits {
    println!("entite:{} score={:.3}", hit.entity_id.0, hit.score);
}
```

### Recherche vectorielle

Similarite cosinus brute-force sur les embeddings. Necessite `embedding_dims > 0`.

```rust
use hora_graph_core::HoraConfig;

let config = HoraConfig { embedding_dims: 3, ..Default::default() };
let mut hora = HoraCore::new(config)?;

let _id = hora.add_entity("vec", "test", None, Some(&[1.0, 0.0, 0.0]))?;
let _id = hora.add_entity("vec", "similaire", None, Some(&[0.9, 0.1, 0.0]))?;

let hits = hora.vector_search(&[1.0, 0.0, 0.0], 5)?;
// hits[0] sera "test" (correspondance exacte, score ~ 1.0)
```

### Recherche hybride (BM25 + Vecteur via RRF)

Combine les deux jambes via Reciprocal Rank Fusion. Fournissez le texte, l'embedding, ou les deux.

```rust
use hora_graph_core::SearchOpts;

let hits = hora.search(
    Some("programmation"),      // requete BM25
    Some(&[1.0, 0.0, 0.0]),   // requete vectorielle
    SearchOpts { top_k: 10, include_dark: false },
)?;
```

Si une seule jambe est fournie, l'autre est ignoree. Retourne vide si aucune n'est fournie.

## 6. Parcours de graphe

### Parcours BFS

```rust
use hora_graph_core::TraverseOpts;

// Parcourir jusqu'a la profondeur 3 depuis Alice
let result = hora.traverse(alice, TraverseOpts { depth: 3 })?;
println!("Decouvert {} entites, {} aretes",
    result.entity_ids.len(), result.edge_ids.len());
```

### Voisins

```rust
// Voisins directs uniquement (profondeur 1)
let neighbor_ids = hora.neighbors(alice)?;
```

### Timeline

```rust
// Tous les faits impliquant Alice, tries par valid_at
let timeline = hora.timeline(alice)?;
for fact in &timeline {
    println!("{}: {} -> {}", fact.relation_type, fact.source.0, fact.target.0);
}
```

### Requete temporelle

```rust
// Tous les faits valides a un instant donne
let timestamp = 1700000000000_i64; // epoch ms
let valid_facts = hora.facts_at(timestamp)?;
```

## 7. Memoire bio-inspiree

### Activation ACT-R

Chaque entite a un score d'activation base sur l'equation Base-Level Learning d'ACT-R.
Il decroit avec le temps et augmente a chaque acces.

```rust
// get_entity() enregistre automatiquement un acces
let _ = hora.get_entity(alice)?;

// Obtenir le score d'activation actuel
let activation = hora.get_activation(alice);
// Some(f64) — plus haut = plus actif, NEG_INFINITY = jamais accede

// Enregistrer manuellement un acces (appele auto par get_entity/search)
hora.record_access(alice);
```

La formule d'activation : `B_i = ln(Sigma t_j^(-d))` ou `t_j` est le temps depuis chaque acces
et `d = 0.5` (taux de decroissance).

### Reconsolidation

Quand une memoire est reactivee, elle entre dans une phase labile (destabilisee) avant de se restabiliser.
Cela modelise la fenetre de reconsolidation des neurosciences.

```rust
use hora_graph_core::MemoryPhase;

let phase = hora.get_memory_phase(alice);
// Some(&MemoryPhase::Stable) — etat normal
// Some(&MemoryPhase::Labile { .. }) — destabilise, peut etre mis a jour
// Some(&MemoryPhase::Restabilizing { .. }) — en cours de restabilisation
// Some(&MemoryPhase::Dark { .. }) — silence (en dessous du seuil)

let multiplier = hora.get_stability_multiplier(alice);
// Augmente de 1.2x a chaque cycle de reconsolidation complet
```

### Dark Nodes

Les entites avec une activation tres basse qui n'ont pas ete accedees recemment deviennent "dark" — cachees des resultats de recherche mais pas supprimees.

```rust
// Lancer une passe dark node (marque les entites a faible activation comme Dark)
let count = hora.dark_node_pass();
println!("{} entites assombries", count);

// Lister toutes les entites dark
let dark_ids = hora.dark_nodes();

// Recuperer une entite dark (transite vers Labile pour re-encodage)
let recovered = hora.attempt_recovery(alice);

// Lister les entites eligibles au garbage collection
let gc_candidates = hora.gc_candidates();
```

### Planification FSRS

Free Spaced Repetition Scheduler — optimise les intervalles de revision.

```rust
// Recuperabilite actuelle (0.0 a 1.0, decroit avec le temps)
let r = hora.get_retrievability(alice);

// Intervalle optimal de prochaine revision en jours
let days = hora.get_next_review_days(alice);

// Stabilite actuelle en jours
let stability = hora.get_fsrs_stability(alice);
```

### Activation par propagation

Propagation d'activation ACT-R a travers le graphe avec effet d'eventail (fan effect).

```rust
use hora_graph_core::SpreadingParams;

let sources = vec![(alice, 1.0)]; // entites sources avec activation initiale
let result = hora.spread_activation(&sources, &SpreadingParams::default())?;

for (entity_id, activation) in &result {
    println!("entite:{} activation_propagee={:.3}", entity_id.0, activation);
}
```

## 8. Cycle de reve (Dream Cycle)

Le cycle de reve est un pipeline de consolidation en 6 etapes inspire des neurosciences du sommeil :

| Etape | Action | Inspiration |
|-------|--------|-------------|
| **SHY** | Reduire tous les scores d'activation (facteur par defaut 0.78) | Tononi & Cirelli 2003 |
| **Replay** | Reactiver des entites depuis un melange d'episodes recents/anciens | McClelland 1995, Ji & Wilson 2007 |
| **CLS** | Extraire les patterns episodiques recurrents en faits semantiques | Kumaran et al. 2016 |
| **Linking** | Creer des liens temporels entre entites co-creees | Zeithamova & Preston 2010 |
| **Dark check** | Silencer les entites a faible activation | Consolidation inhibitrice |
| **GC** | Supprimer les entites dark eligibles (opt-in) | Decroissance memorielle |

```rust
use hora_graph_core::DreamCycleConfig;

// Lancer avec toutes les etapes (GC desactive par defaut)
let stats = hora.dream_cycle(&DreamCycleConfig::default())?;
println!("Reduits: {}", stats.entities_downscaled);
println!("Rejoues: {} episodes", stats.replay.episodes_replayed);
println!("CLS: {} faits crees", stats.cls.facts_created);
println!("Liens: {} crees", stats.linking.links_created);
println!("Assombris: {}", stats.dark_nodes_marked);

// Lancer avec GC active
let stats = hora.dream_cycle(&DreamCycleConfig {
    gc: true,
    ..Default::default()
})?;
println!("GC supprimes: {}", stats.gc_deleted);

// Lancer uniquement certaines etapes
let stats = hora.dream_cycle(&DreamCycleConfig {
    shy: true,
    replay: false,
    cls: false,
    linking: false,
    dark_check: true,
    gc: false,
})?;
```

### Etapes individuelles

Chaque etape peut aussi etre appelee independamment :

```rust
// Reduction SHY (facteur: 0.0-1.0)
let count = hora.shy_downscaling(0.78);

// Replay entrelace
let replay_stats = hora.interleaved_replay()?;

// Transfert CLS
let cls_stats = hora.cls_transfer()?;

// Liaison memorielle
let link_stats = hora.memory_linking()?;
```

## 9. Persistence

### Instance fichier

```rust
use hora_graph_core::{HoraCore, HoraConfig};

// Ouvrir (cree le fichier s'il n'existe pas, charge s'il existe)
let mut hora = HoraCore::open("data.hora", HoraConfig::default())?;

// Operations d'ecriture...
let _id = hora.add_entity("demo", "test", None, None)?;

// Sauvegarder sur disque (atomique : ecrit dans .tmp puis renomme)
hora.flush()?;
```

### Snapshots

```rust
// Copier l'etat actuel dans un fichier separe
hora.snapshot("backup.hora")?;
```

### Verification de fichier

```rust
use hora_graph_core::verify_file;

let report = verify_file("data.hora")?;
println!("Entites: {}", report.entity_count);
println!("Aretes: {}", report.edge_count);
println!("Episodes: {}", report.episode_count);
```

### En memoire uniquement

```rust
let mut hora = HoraCore::new(HoraConfig::default())?;
// flush() retournera une erreur — pas de chemin configure
// Utilisez snapshot() pour ecrire dans un fichier specifique
```

## 10. Reference de configuration

### HoraConfig

```rust
use hora_graph_core::{HoraConfig, DedupConfig};

let config = HoraConfig {
    // Dimensions des vecteurs d'embedding. 0 = mode texte uniquement.
    embedding_dims: 384,

    // Parametres de deduplication
    dedup: DedupConfig {
        enabled: true,             // Activer la dedup sur add_entity
        name_exact: true,          // Detecter les correspondances de nom normalisees
        jaccard_threshold: 0.85,   // Seuil de chevauchement de tokens (0.0 = desactive)
        cosine_threshold: 0.92,    // Seuil de similarite d'embedding (0.0 = desactive)
    },
};

// Desactiver la deduplication
let config = HoraConfig {
    dedup: DedupConfig::disabled(),
    ..Default::default()
};
```

### ConsolidationParams (Cycle de reve)

Parametres internes via `ConsolidationParams::default()` :

| Parametre | Defaut | Description |
|-----------|--------|-------------|
| `shy_factor` | `0.78` | Facteur multiplicatif pour la reduction SHY |
| `recent_ratio` | `0.7` | Fraction d'episodes recents dans le replay (70%) |
| `max_replay_items` | `100` | Max episodes par cycle de replay |
| `cls_threshold` | `3` | Min consolidation_count pour eligibilite CLS |
| `linking_window_ms` | `21_600_000` | Fenetre temporelle pour le linking (6 heures) |
| `linking_max_neighbors` | `20` | Max voisins temporels par entite |

### DreamCycleConfig

| Champ | Defaut | Description |
|-------|--------|-------------|
| `shy` | `true` | Activer la reduction SHY |
| `replay` | `true` | Activer le replay entrelace |
| `cls` | `true` | Activer le transfert semantique CLS |
| `linking` | `true` | Activer la liaison memorielle temporelle |
| `dark_check` | `true` | Activer la detection des dark nodes |
| `gc` | `false` | Activer le GC des entites dark (destructif) |

### DarkNodeParams

| Parametre | Defaut | Description |
|-----------|--------|-------------|
| `silencing_threshold` | `-2.0` | Activation en dessous de laquelle les entites deviennent dark |
| `silencing_delay_secs` | `604_800` | Secondes minimum depuis le dernier acces (7 jours) |
| `gc_eligible_after_secs` | `2_592_000` | Secondes en dark avant eligibilite GC (30 jours) |

### FsrsParams

| Parametre | Defaut | Description |
|-----------|--------|-------------|
| `desired_retention` | `0.9` | Recuperabilite cible pour la planification |
| `initial_stability_days` | `1.0` | Stabilite initiale pour les nouvelles entites |
| `decay` | `0.2` | Exposant de decroissance en loi de puissance |

### SpreadingParams

| Parametre | Defaut | Description |
|-----------|--------|-------------|
| `max_depth` | `3` | Profondeur maximale de propagation |
| `s_max` | `1.6` | Force d'association maximale |
| `decay_per_hop` | `0.5` | Decroissance multiplicative par saut |

### ReconsolidationParams

| Parametre | Defaut | Description |
|-----------|--------|-------------|
| `labile_duration_secs` | `3600.0` | Duree de la phase labile (1 heure) |
| `restabilization_duration_secs` | `7200.0` | Duree de la restabilisation (2 heures) |
| `reactivation_threshold` | `-1.0` | Niveau d'activation declenchant la reconsolidation |
| `restabilization_boost` | `1.2` | Multiplicateur de stabilite gagne par cycle |

## Statistiques

```rust
let stats = hora.stats()?;
println!("Entites: {}, Aretes: {}, Episodes: {}",
    stats.entities, stats.edges, stats.episodes);
```

## Gestion des erreurs

Toutes les operations faillibles retournent `hora_graph_core::Result<T>`, qui encapsule `HoraError` :

```rust
use hora_graph_core::HoraError;

match hora.get_entity(EntityId(999)) {
    Ok(Some(e)) => println!("Trouve: {}", e.name),
    Ok(None) => println!("Non trouve"),
    Err(HoraError::Io(e)) => eprintln!("Erreur I/O: {}", e),
    Err(e) => eprintln!("Erreur: {}", e),
}
```

Variantes d'erreur : `Io`, `CorruptedFile`, `InvalidFile`, `VersionMismatch`, `EntityNotFound`,
`EdgeNotFound`, `DimensionMismatch`, `AlreadyInvalidated`, `StringTooLong`, `StorageFull`.

---

## 11. Bindings multi-langages

hora-graph-core expose la meme API en Rust, Node.js, Python, WASM et C.

### Node.js (napi-rs)

```js
const { HoraCore } = require('hora-graph-core');

// En memoire
const g = HoraCore.newMemory();

// Avec fichier
const g2 = HoraCore.open('data.hora');

// Les IDs sont u32 en JS (u64 en interne)
const alice = g.addEntity('person', 'Alice', { role: 'engineer' });
const bob   = g.addEntity('person', 'Bob');

const factId = g.addFact(alice, bob, 'knows', 'Rencontre a RustConf', 0.9);

// Parcours
const result = g.traverse(alice, { depth: 3 });
// result.entityIds: number[], result.edgeIds: number[]

// Recherche
const hits = g.textSearch('Alice', 5);
// hits: [{ entityId, name, entityType, score }]

// Activation
const activation = g.getActivation(alice);

// Phase memoire
const phase = g.getMemoryPhase(alice); // "stable" | "labile" | "restabilizing" | "dark" | null

// Persistence
g.flush();
g.snapshot('backup.hora');
```

### Python (PyO3)

```python
from hora_graph_core import HoraGraph

# En memoire
g = HoraGraph()

# Avec fichier
g = HoraGraph(path="data.hora")

# Les IDs sont des int (u64)
alice = g.add_entity("person", "Alice", properties={"role": "engineer"})
bob   = g.add_entity("person", "Bob")

fact_id = g.add_fact(alice, bob, "knows", "Rencontre a RustConf", 0.9)

# Parcours
result = g.traverse(alice, depth=3)

# Recherche
hits = g.text_search("Alice", k=5)

# Memoire
activation = g.get_activation(alice)
phase = g.get_memory_phase(alice)

# Cycle de reve
stats = g.dream_cycle()

# Persistence
g.flush()
g.snapshot("backup.hora")
```

### WASM (wasm-bindgen)

```js
import init, { HoraCore } from 'hora-graph-wasm';

await init();
const g = HoraCore.newMemory();

// Meme API que Node.js, mais en memoire uniquement (pas de persistence fichier)
const alice = g.addEntity('person', 'Alice', { role: 'engineer' });
const bob   = g.addEntity('person', 'Bob');
g.addFact(alice, bob, 'knows', 'Rencontre a RustConf', 0.9);

const result = g.traverse(alice, 3);
const stats = g.stats();
```

### C FFI (cbindgen)

```c
#include "hora_graph_core.h"

HoraCore* g = hora_new_memory(0);
uint64_t alice = hora_add_entity(g, "person", "Alice", NULL);
uint64_t bob   = hora_add_entity(g, "person", "Bob", NULL);
hora_add_fact(g, alice, bob, "knows", "Rencontre a RustConf", 0.9f);

hora_flush(g, "data.hora");
hora_free(g);
```

## 12. Reference API complete

### Methodes HoraCore

| Categorie | Methode | Description |
|-----------|---------|-------------|
| **Creation** | `new(config)` | Nouvelle instance en memoire |
| | `open(path, config)` | Ouvrir instance avec fichier |
| **Entites** | `add_entity(type, name, props, embedding)` | Creer une entite, retourne `EntityId` |
| | `get_entity(id)` | Lire (enregistre un acces pour l'activation) |
| | `update_entity(id, update)` | Mise a jour partielle (`EntityUpdate`) |
| | `delete_entity(id)` | Supprimer + cascade des aretes |
| **Faits** | `add_fact(source, target, relation, desc, confidence)` | Creer une arete orientee |
| | `get_fact(id)` | Lire un fait |
| | `update_fact(id, update)` | Mise a jour partielle (`FactUpdate`: confidence, description) |
| | `invalidate_fact(id)` | Soft-delete bi-temporel (fixe `invalid_at`) |
| | `delete_fact(id)` | Hard-delete physique |
| | `get_entity_facts(entity_id)` | Tous les faits d'une entite |
| **Parcours** | `traverse(start, opts)` | BFS jusqu'a profondeur donnee |
| | `neighbors(entity_id)` | IDs des voisins directs |
| | `timeline(entity_id)` | Faits tries par `valid_at` |
| | `facts_at(timestamp)` | Faits valides au temps `t` |
| **Recherche** | `text_search(query, k)` | Recherche plein texte BM25+ |
| | `vector_search(query, k)` | Similarite cosinus SIMD |
| | `search(text, embedding, opts)` | Hybride RRF (BM25 + vecteur) |
| **Memoire** | `get_activation(id)` | Activation base-level ACT-R |
| | `record_access(id)` | Enregistrer un acces manuellement |
| | `get_memory_phase(id)` | Stable/Labile/Restabilizing/Dark |
| | `get_stability_multiplier(id)` | Gain de stabilite par reconsolidation |
| | `get_retrievability(id)` | Recuperabilite FSRS (0.0-1.0) |
| | `get_next_review_days(id)` | Intervalle de revision optimal |
| | `get_fsrs_stability(id)` | Stabilite FSRS en jours |
| | `spread_activation(sources, params)` | Propagation ACT-R avec fan effect |
| **Dark Nodes** | `dark_node_pass()` | Silencer les entites a faible activation |
| | `dark_nodes()` | Lister les IDs dark |
| | `attempt_recovery(id)` | Recuperer une entite dark |
| | `gc_candidates()` | Entites dark eligibles a la suppression |
| **Consolidation** | `shy_downscaling(factor)` | Reduire toutes les activations |
| | `interleaved_replay()` | Rejouer episodes recents+anciens |
| | `cls_transfer()` | Extraire faits semantiques des episodes |
| | `memory_linking()` | Creer liens de co-occurrence temporelle |
| | `dream_cycle(config)` | Lancer la consolidation complete en 6 etapes |
| **Episodes** | `add_episode(source, session, entities, facts)` | Creer un episode |
| | `get_episode(id)` | Lire un episode |
| | `get_episodes(session, source, since, until)` | Filtrer les episodes |
| | `increment_consolidation(id)` | Incrementer le compteur de consolidation |
| **Persistence** | `flush()` | Sauvegarder dans le fichier configure |
| | `snapshot(dest)` | Copier dans un autre fichier |
| | `verify_file(path)` | Valider l'integrite du fichier |
| **Stats** | `stats()` | Compteurs entites/aretes/episodes |

---

**hora-graph-core v1.0.0** — [github.com/Vivien83/hora-graph-core](https://github.com/Vivien83/hora-graph-core)
