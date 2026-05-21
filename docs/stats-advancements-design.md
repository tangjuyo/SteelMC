# Statistics & Advancements Implementation Design

SteelMC — Minecraft 26.1 (protocol 775)

---

## Table of Contents

1. [System Overview](#1-system-overview)
2. [Vanilla Reference](#2-vanilla-reference)
   - [Statistics](#21-statistics)
   - [Advancements](#22-advancements)
   - [Protocol Packets](#23-protocol-packets)
3. [SteelMC Integration Points](#3-steelmc-integration-points)
   - [Existing Hooks & TODOs](#31-existing-hooks--todos)
   - [Player Data Lifecycle](#32-player-data-lifecycle)
   - [Packet Infrastructure](#33-packet-infrastructure)
4. [Statistics: Data Model & Design](#4-statistics-data-model--design)
   - [Stat Types](#41-stat-types)
   - [Custom Stats Enum](#42-custom-stats-enum)
   - [PlayerStats Struct](#43-playerstats-struct)
   - [Dirty Tracking & Flushing](#44-dirty-tracking--flushing)
   - [Persistence](#45-persistence)
5. [Advancements: Data Model & Design](#5-advancements-data-model--design)
   - [Advancement Data Structures](#51-advancement-data-structures)
   - [Criterion Triggers](#52-criterion-triggers)
   - [PlayerAdvancements Struct](#53-playeradvancements-struct)
   - [Visibility Logic](#54-visibility-logic)
   - [Build-Time Compilation](#55-build-time-compilation)
6. [New Files to Create](#6-new-files-to-create)
7. [Files to Modify](#7-files-to-modify)
8. [Trigger Callsite Map](#8-trigger-callsite-map)
9. [Design Decisions & Constraints](#9-design-decisions--constraints)

---

## 1. System Overview

Minecraft's statistics and advancements are two closely related but distinct systems:

- **Statistics** are per-player integer counters (blocks mined, distance walked, mob kills, etc.) stored as JSON on disk and periodically sent to the client via `C_AWARD_STATS`. They are purely informational and do not gate any gameplay.
- **Advancements** (formerly "achievements") are a tree of goals, each defined by a set of `Criterion` that the player must satisfy. They are stored on disk, sent to the client via `C_UPDATE_ADVANCEMENTS`, and can grant rewards (XP, loot, recipes, function commands). The client renders the advancement UI.

Both systems are **entirely missing** from SteelMC. The packet IDs exist in `vanilla_packets.rs` but no structs, storage, or trigger logic have been implemented yet.

---

## 2. Vanilla Reference

### 2.1 Statistics

**Source files in `minecraft-src/`:**

| File | Role |
|------|------|
| `src/net/minecraft/stats/Stats.java` | All stat definitions: 4 registry-based `StatType`s + ~50 custom `Identifier` stats |
| `src/net/minecraft/stats/StatType.java` | Generic `StatType<T>` — wraps a `Registry<T>` and maps entries to `Stat<T>` instances |
| `src/net/minecraft/stats/Stat.java` | A `Stat<T>` = a `(StatType<T>, T)` pair with a `StatFormatter`; `STREAM_CODEC` encodes as `(stat_type_id, entry_id)` |
| `src/net/minecraft/stats/StatsCounter.java` | In-memory counter: `Object2IntMap<Stat<?>>` with `getValue`, `setValue`, `increment` |
| `src/net/minecraft/stats/ServerStatsCounter.java` | Extends `StatsCounter`; loads from / saves to JSON; tracks a dirty set; `sendStats()` sends `ClientboundAwardStatsPacket` |
| `src/net/minecraft/stats/StatFormatter.java` | Display formatters: `DEFAULT`, `TIME`, `DISTANCE`, `DIVIDE_BY_TEN` (client-side display only, not sent over wire) |

**Stat categories:**

| StatType key | Registry | Example |
|---|---|---|
| `minecraft:mined` | `BuiltInRegistries.BLOCK` | blocks broken |
| `minecraft:crafted` | `BuiltInRegistries.ITEM` | items crafted |
| `minecraft:used` | `BuiltInRegistries.ITEM` | items used |
| `minecraft:broken` | `BuiltInRegistries.ITEM` | tools broken |
| `minecraft:picked_up` | `BuiltInRegistries.ITEM` | items picked up |
| `minecraft:dropped` | `BuiltInRegistries.ITEM` | items dropped |
| `minecraft:killed` | `BuiltInRegistries.ENTITY_TYPE` | entities killed |
| `minecraft:killed_by` | `BuiltInRegistries.ENTITY_TYPE` | killed by entity type |
| `minecraft:custom` | `BuiltInRegistries.CUSTOM_STAT` | walk distance, deaths, etc. |

**Custom stats (subset):** `play_time`, `walk_one_cm`, `sprint_one_cm`, `jump`, `deaths`, `mob_kills`, `player_kills`, `damage_dealt`, `damage_taken`, `interact_with_crafting_table`, `open_barrel`, `open_chest`, `eat_cake_slice`, `sleep_in_bed`, `enchant_item`, and ~35 more. Full list in `Stats.java` lines 21–97.

**Persistence format (vanilla):** JSON file at `{world_folder}/stats/{player_uuid}.json`:
```json
{
  "DataVersion": 4189,
  "stats": {
    "minecraft:mined": { "minecraft:stone": 12 },
    "minecraft:custom": { "minecraft:walk_one_cm": 15000 }
  }
}
```

**Wire encoding of a stat** (`Stat.STREAM_CODEC`): `VarInt(stat_type_registry_id)` + `VarInt(entry_registry_id)`. So `C_AWARD_STATS` = `VarInt(map_size)` followed by N × `(stat, VarInt(value))` pairs.

### 2.2 Advancements

**Source files in `minecraft-src/`:**

| File | Role |
|------|------|
| `src/net/minecraft/advancements/Advancement.java` | Core data record: `parent?`, `display?`, `rewards`, `criteria: Map<String, Criterion>`, `requirements: AdvancementRequirements`, `sendsTelemetryEvent` |
| `src/net/minecraft/advancements/AdvancementHolder.java` | Wraps `(Identifier id, Advancement value)`; has a stream codec for the wire |
| `src/net/minecraft/advancements/AdvancementProgress.java` | Per-player: `Map<String, CriterionProgress>`; `isDone()`, `grantProgress(name)`, `revokeProgress(name)` |
| `src/net/minecraft/advancements/CriterionProgress.java` | Single criterion: either `obtained: Option<Instant>` (done) or null (not done) |
| `src/net/minecraft/advancements/AdvancementRequirements.java` | CNF formula over criterion names: `[[a, b], [c]]` means "(a OR b) AND c" |
| `src/net/minecraft/advancements/AdvancementTree.java` | Graph of `AdvancementNode`s; root nodes are those with no parent |
| `src/net/minecraft/advancements/CriterionTrigger.java` | Interface with `addPlayerListener`, `removePlayerListener`, `removePlayerListeners` |
| `src/net/minecraft/advancements/CriteriaTriggers.java` | Registry of all ~50 built-in trigger types |
| `src/net/minecraft/server/PlayerAdvancements.java` | Per-player state machine: loads JSON, tracks `progress`, `visible`, `progressChanged`; `award()`, `revoke()`, `flushDirty()` |
| `src/net/minecraft/server/ServerAdvancementManager.java` | Server-level: holds `AdvancementTree`; loads all advancement JSONs from datapacks |

**Data definition format** (JSON from `minecraft-src/minecraft/resources/data/minecraft/advancement/**/*.json`):
```json
{
  "parent": "minecraft:story/root",
  "criteria": {
    "has_crafting_table": {
      "trigger": "minecraft:inventory_changed",
      "conditions": { "items": [{ "items": "minecraft:crafting_table" }] }
    }
  },
  "requirements": [["has_crafting_table"]],
  "display": {
    "icon": { "id": "minecraft:crafting_table" },
    "title": { "translate": "advancements.story.obtain_pickaxe.title" },
    "description": { "translate": "advancements.story.obtain_pickaxe.description" },
    "frame": "task",
    "show_toast": true,
    "announce_to_chat": true,
    "hidden": false
  },
  "rewards": { "experience": 10 },
  "sends_telemetry_event": true
}
```

There are **1617 advancement JSON files** in minecraft-src across 6 categories: `story`, `nether`, `end`, `adventure`, `husbandry`, and `recipes`.

**Player advancement persistence** (vanilla) — JSON at `{world_folder}/advancements/{player_uuid}.json`:
```json
{
  "DataVersion": 4189,
  "minecraft:story/root": {
    "criteria": { "crafting_table": "2024-01-01 12:00:00 +0000" },
    "done": true
  }
}
```

**Criterion trigger lifecycle:**
1. Server loads advancement JSONs → builds `AdvancementTree`
2. On player join: `PlayerAdvancements.load()` → restores progress from JSON → `registerListeners()` subscribes the player to every incomplete criterion's trigger
3. When a game event occurs (block broken, item picked up, etc.) → `CriterionTrigger.trigger()` is called → iterates listener list → checks predicate → calls `PlayerAdvancements.award(holder, criterion)` if matched
4. `award()` marks progress dirty, grants rewards on completion, announces in chat
5. On each tick (or explicit flush): `flushDirty()` → sends `C_UPDATE_ADVANCEMENTS`

**Advancement rewards** (`AdvancementRewards.java`): can grant `experience`, `loot` (list of loot table IDs), `recipes` (list of recipe IDs), `function` (a function command). Only XP and recipes are relevant for initial implementation.

**Visibility rules** (`AdvancementVisibilityEvaluator.java`): an advancement is visible if it is done, OR if its parent is visible, OR if it is a root. Hidden advancements are only visible once done.

### 2.3 Protocol Packets

All packet IDs are already generated in `steel-registry/src/generated/vanilla_packets.rs`:

| Packet ID constant | Direction | ID | Description |
|---|---|---|---|
| `C_AWARD_STATS` | Clientbound | 3 | Sends dirty stats to client (map of stat → value) |
| `C_UPDATE_ADVANCEMENTS` | Clientbound | 130 | Sends full or delta advancement tree + progress |
| `C_SELECT_ADVANCEMENTS_TAB` | Clientbound | 85 | Selects which tab is open in the advancement screen |
| `S_SEEN_ADVANCEMENTS` | Serverbound | 50 | Client notifies server which advancement tab it opened |

**`C_AWARD_STATS` wire format** (from `ClientboundAwardStatsPacket.java`):
```
VarInt count
for each:
  VarInt stat_type_registry_id  // index in STAT_TYPE registry
  VarInt entry_registry_id      // index in the stat type's registry
  VarInt value                  // absolute value (not delta)
```

**`C_UPDATE_ADVANCEMENTS` wire format** (from `ClientboundUpdateAdvancementsPacket.java`):
```
Boolean reset              // true on first send (login), clears client state
VarInt added_count
for each added:
  Identifier id
  Advancement (stream_codec): parent?, display?, requirements, sendsTelemetryEvent
VarInt removed_count
for each removed:
  Identifier id
VarInt progress_count
for each progress:
  Identifier id
  AdvancementProgress: VarInt criteria_count, for each: String name, Instant? obtained_time
Boolean showAdvancements   // from game rule ANNOUNCE_ADVANCEMENTS
```

**`C_SELECT_ADVANCEMENTS_TAB` wire format:**
```
Optional<Identifier> tab   // null = deselect
```

**`S_SEEN_ADVANCEMENTS` wire format:**
```
Identifier tab_id
```

---

## 3. SteelMC Integration Points

### 3.1 Existing Hooks & TODOs

Scattered TODOs already mark where stat/advancement calls must be inserted:

| File | Line | TODO |
|---|---|---|
| `steel-core/src/player/mod.rs` | ~935 | `// TODO: implement stats` (inside item pickup handling) |
| `steel-core/src/player/mod.rs` | ~425 | `// TODO: Updating advancements` (inside `tick()`) |
| `steel-core/src/behavior/blocks/container/crafting_table_block.rs` | ~50 | `// TODO: Award stat INTERACT_WITH_CRAFTING_TABLE` |
| `steel-core/src/behavior/blocks/container/barrel_block.rs` | ~80 | `// TODO: Award stat OPEN_BARREL` |
| `steel-core/src/behavior/items/honeycomb.rs` | ~24 | `// TODO: trigger CriteriaTriggers.ITEM_USED_ON_BLOCK advancement` |
| `steel-core/src/inventory/slot.rs` | ~650 | `// TODO: Add statistics/achievement tracking here` |

### 3.2 Player Data Lifecycle

**Current player load/save path:**

```
Login (steel-login/src/handlers/config.rs)
  → finish_configuration()
  → Arc<Player> created with default state
  → PlayerDataStorage::load() → PersistentPlayerData::apply_to_player()

Player disconnect / world save:
  → PersistentPlayerData::from_player()
  → PlayerDataStorage::save()
```

`PersistentPlayerData` (`steel-core/src/player/player_data.rs`) is a flat struct serialized via `wincode` into a binary `STLP`-magic file in `players/` directory. It currently contains: position, rotation, motion, health, game_mode, abilities, inventory, food data, and experience.

**Extension needed:** stats and advancement progress must be loaded and saved alongside player data. Stats and advancements should each live in their own separate files (matching vanilla layout) rather than being appended to the STLP binary, since:
1. The vanilla JSON format for both is stable and well-defined
2. Stats/advancements are large variable-length data
3. Vanilla clients and tools expect these files to exist at known paths
4. Per CLAUDE.md: "We should use the extraction data from the minecraft data pack instead of generating a custom format if it exists in there"

### 3.3 Packet Infrastructure

New packets follow the existing derive pattern in `steel-protocol/src/packets/game/`.

**Serverbound packet** (`S_SEEN_ADVANCEMENTS`) — client tells server which tab was opened:
```rust
#[derive(ReadFrom, ServerPacket, Clone, Debug)]
pub struct SSeenAdvancements {
    pub tab: Option<Identifier>,
}
```

**Clientbound packets** (3 new):
- `CAwardStats` — sends dirty stat counters
- `CUpdateAdvancements` — sends advancement tree + progress
- `CSelectAdvancementsTab` — tells client which tab to show

After creating, export in `steel-protocol/src/packets/game/mod.rs` and add the handler for `S_SEEN_ADVANCEMENTS` in `steel-core/src/player/networking.rs`.

---

## 4. Statistics: Data Model & Design

### 4.1 Stat Types

Vanilla has 4 + 1 stat types. In SteelMC, stat types map directly to SteelMC's typed registry references:

| StatType | Key type in SteelMC | Registry |
|---|---|---|
| `BLOCK_MINED` | `BlockRef` | blocks |
| `ITEM_CRAFTED` | `ItemRef` | items |
| `ITEM_USED` | `ItemRef` | items |
| `ITEM_BROKEN` | `ItemRef` | items |
| `ITEM_PICKED_UP` | `ItemRef` | items |
| `ITEM_DROPPED` | `ItemRef` | items |
| `ENTITY_KILLED` | `EntityTypeRef` | entity types |
| `ENTITY_KILLED_BY` | `EntityTypeRef` | entity types |
| `CUSTOM` | `CustomStatId` | custom stats |

A `Stat` on the wire is encoded as two VarInts: the stat-type's registry index, then the entry's registry index. SteelMC must match vanilla's registry ordering exactly so the protocol encoding is correct.

### 4.2 Custom Stats Enum

The ~50 custom stat identifiers (`walk_one_cm`, `deaths`, etc.) should be a hand-written enum with a `to_identifier()` method. Do **not** generate this — it is small and the logic is trivial. Matches `Stats.java` lines 21–97 verbatim.

```rust
// steel-core/src/player/stats/custom_stat.rs
pub enum CustomStat {
    LeaveGame,
    PlayTime,
    TotalWorldTime,
    TimeSinceDeath,
    // ... (all 50+ variants)
    WalkOneCm,
    SprintOneCm,
    // ...
}
```

Each variant maps to a `minecraft:<name>` identifier. The identifier order in the `CUSTOM_STAT` registry determines the VarInt sent on the wire.

### 4.3 PlayerStats Struct

Location: `steel-core/src/player/stats/mod.rs`

```rust
pub struct PlayerStats {
    // Registry-typed stats
    blocks_mined:   FxHashMap<BlockRef, i32>,
    items_crafted:  FxHashMap<ItemRef, i32>,
    items_used:     FxHashMap<ItemRef, i32>,
    items_broken:   FxHashMap<ItemRef, i32>,
    items_picked_up: FxHashMap<ItemRef, i32>,
    items_dropped:  FxHashMap<ItemRef, i32>,
    entities_killed:    FxHashMap<EntityTypeRef, i32>,
    entities_killed_by: FxHashMap<EntityTypeRef, i32>,
    // Custom stats — array indexed by CustomStat discriminant
    custom: [i32; CUSTOM_STAT_COUNT],
    // Dirty tracking
    dirty: HashSet<StatKey>,
}
```

Where `StatKey` is an enum of `(StatTypeIdx, EntryIdx)` or a similar compact representation that maps to the two VarInts in the wire format.

`PlayerStats` lives behind a `SyncMutex` on `Player`, parallel to `food_data`, `experience`, etc.

### 4.4 Dirty Tracking & Flushing

Mirroring vanilla `ServerStatsCounter`:
- Every `increment()` / `set_value()` call adds the stat to a dirty set
- `flush_dirty(player)` drains the dirty set, builds a `Vec<(StatWire, i32)>` and sends `CAwardStats`
- Called once per tick from `Player::tick()` (or on demand after bulk changes)
- On login: `mark_all_dirty()` so the client receives the full snapshot on first join

**No client → server stat sync** — stats are server-authoritative only. The `S_SEEN_ADVANCEMENTS` packet is the only relevant serverbound packet.

### 4.5 Persistence

Stats are stored as JSON at `{save_root}/{domain}/stats/{uuid}.json`, matching vanilla's layout so vanilla tools can read them.

```
players/
├─ {uuid}.stlg          (global data — Steel binary)
overworld/
├─ players/
│  └─ {uuid}.stlp       (domain player data — Steel binary)
├─ stats/
│  └─ {uuid}.json       (statistics — vanilla JSON)
└─ advancements/
   └─ {uuid}.json       (advancement progress — vanilla JSON)
```

The JSON format matches vanilla `ServerStatsCounter.toJson()` / `parse()`: a top-level `stats` object keyed by stat-type ID, then by entry ID, with integer values plus a `DataVersion` field.

Serialization uses `serde_json` (already a dependency via simdnbt's ecosystem or can be added). Do **not** use `wincode` for stats/advancements — the vanilla JSON format is required for compatibility.

---

## 5. Advancements: Data Model & Design

### 5.1 Advancement Data Structures

```rust
// steel-core/src/advancement/mod.rs

pub struct Advancement {
    pub parent:               Option<Identifier>,
    pub display:              Option<DisplayInfo>,
    pub rewards:              AdvancementRewards,
    /// CNF: Vec<Vec<String>> where inner vec = OR group, outer = AND
    pub requirements:         Vec<Vec<String>>,
    pub sends_telemetry_event: bool,
}

pub struct DisplayInfo {
    pub title:           TextComponent,
    pub description:     TextComponent,
    pub icon:            ItemStack,       // or ItemRef for vanilla items
    pub background:      Option<Identifier>,
    pub frame:           AdvancementType, // Task, Goal, Challenge
    pub show_toast:      bool,
    pub announce_to_chat: bool,
    pub hidden:          bool,
}

pub enum AdvancementType { Task, Goal, Challenge }

pub struct AdvancementRewards {
    pub experience: i32,
    pub loot:       Vec<Identifier>,   // loot table IDs
    pub recipes:    Vec<Identifier>,   // recipe IDs
    pub function:   Option<Identifier>,
}

/// An advancement with its registry ID
pub struct AdvancementHolder {
    pub id:    Identifier,
    pub value: Arc<Advancement>,
}
```

For the **tree structure**:
```rust
pub struct AdvancementNode {
    pub holder:   AdvancementHolder,
    pub parent:   Option<Weak<AdvancementNode>>,
    pub children: Vec<Arc<AdvancementNode>>,
}
```

### 5.2 Criterion Triggers

The criterion trigger system is the most complex part. Vanilla uses a listener registry per trigger type where each player subscribes listeners for their in-progress advancements.

SteelMC should implement this as:

```rust
// steel-core/src/advancement/criteria/mod.rs

pub trait CriterionTrigger: Send + Sync {
    fn id(&self) -> &Identifier;
    fn add_listener(&self, player_adv: &Arc<PlayerAdvancements>, listener: CriterionListener);
    fn remove_listener(&self, player_adv: &Arc<PlayerAdvancements>, listener: &CriterionListener);
    fn remove_player_listeners(&self, player_adv: &Arc<PlayerAdvancements>);
}

pub struct CriterionListener {
    pub advancement_id: Identifier,
    pub criterion_key:  String,
    pub instance:       Arc<dyn CriterionTriggerInstance>,
}

pub trait CriterionTriggerInstance: Send + Sync {
    fn matches(&self, context: &TriggerContext) -> bool;
}
```

Each trigger type (e.g., `InventoryChangedTrigger`) holds a `SyncMutex<HashMap<Uuid, Vec<CriterionListener>>>` — mapping player UUID to their active listeners.

**Built-in trigger types** to implement (from `CriteriaTriggers.java`) — prioritized for core gameplay:

| Priority | Trigger ID | Fires when |
|---|---|---|
| 1 | `minecraft:inventory_changed` | Player inventory changes |
| 2 | `minecraft:player_killed_entity` | Player kills a mob |
| 3 | `minecraft:entity_killed_player` | Player is killed by entity |
| 4 | `minecraft:placed_block` | Player places a block |
| 5 | `minecraft:item_used_on_block` | Player uses item on block |
| 6 | `minecraft:consume_item` | Player consumes food/potion |
| 7 | `minecraft:recipe_unlocked` | Recipe added to recipe book |
| 8 | `minecraft:tick` | Every tick (for location-based) |
| 9 | `minecraft:location` | Player at specific location |
| 10 | `minecraft:changed_dimension` | Player changes dimension |
| ... | others | Implement as gameplay is added |

The full list is 50 trigger types. Start with the ~10 above to cover the `story` tab advancements.

### 5.3 PlayerAdvancements Struct

Location: `steel-core/src/player/advancements.rs`

```rust
pub struct PlayerAdvancements {
    /// All advancements with their per-player progress
    progress:         HashMap<Identifier, AdvancementProgress>,
    /// Currently visible advancements (sent to client)
    visible:          HashSet<Identifier>,
    /// Advancements whose progress changed since last flush
    progress_changed: HashSet<Identifier>,
    /// Root nodes that need visibility re-evaluation
    roots_to_update:  HashSet<Identifier>,
    /// Whether the next packet should have reset=true (first send)
    is_first_packet:  bool,
    /// Currently selected tab
    last_selected_tab: Option<Identifier>,
}

pub struct AdvancementProgress {
    criteria: HashMap<String, Option<Instant>>,  // None = not done
}
```

Key methods:
- `award(id, criterion_key)` — grants a criterion, triggers reward if now complete
- `revoke(id, criterion_key)` — un-grants a criterion
- `flush_dirty(player)` — sends `CUpdateAdvancements` and clears dirty sets
- `load(path)` — reads the vanilla JSON file
- `save(path)` — writes the vanilla JSON file

`PlayerAdvancements` lives behind a `SyncMutex` on `Player`.

### 5.4 Visibility Logic

Mirrors vanilla `AdvancementVisibilityEvaluator`:
- A non-hidden advancement is visible if any ancestor is done (breadth-first from root)
- A hidden advancement is visible only when it is done
- On `award()`: re-evaluate visibility for all nodes in the same root tree

### 5.5 Build-Time Compilation

The 1617 JSON files in `minecraft-src/minecraft/resources/data/minecraft/advancement/**/*.json` must be compiled at build time into Rust data.

Per CLAUDE.md ("Vanilla extracted registry/worldgen data should be compiled by build scripts into typed Rust data, not parsed from JSON at runtime") and ("Do not design for runtime datapack JSON loading"):

**Location:** `steel-registry/build/advancements.rs` (new build module) or `steel-core/build/advancements.rs`

The build script reads all JSON files, parses them, and emits:
```rust
// steel-registry/src/generated/vanilla_advancements.rs  (generated)
pub static VANILLA_ADVANCEMENTS: &[AdvancementDef] = &[
    AdvancementDef {
        id: "minecraft:story/root",
        parent: None,
        criteria: &[...],
        requirements: &[&["crafting_table"]],
        display: Some(DisplayDef { ... }),
        rewards: RewardsDef { experience: 0, ... },
        sends_telemetry_event: true,
    },
    // ...
];
```

Where `AdvancementDef`, `DisplayDef`, etc. are `const`-compatible structs with `&'static str` fields. This avoids runtime JSON parsing and keeps startup fast.

For modding: vanilla advancement definitions are a generated baseline. Mods register their own `AdvancementDef`s through a registry (similar to how block/item registries work). The generated file is never hand-edited.

**Note on recipes:** The 1617 files include ~1450+ recipe advancements (auto-granted on recipe unlock). These are structurally identical but require recipe book integration to fire `RECIPE_UNLOCKED` triggers.

---

## 6. New Files to Create

| Path | Description |
|---|---|
| `steel-core/src/player/stats/mod.rs` | `PlayerStats` struct, dirty tracking, flush |
| `steel-core/src/player/stats/custom_stat.rs` | `CustomStat` enum (all ~50 custom stat IDs) |
| `steel-core/src/player/advancements.rs` | `PlayerAdvancements` struct, award/revoke, flush |
| `steel-core/src/advancement/mod.rs` | Core advancement data types (`Advancement`, `DisplayInfo`, etc.) |
| `steel-core/src/advancement/criteria/mod.rs` | `CriterionTrigger` trait + listener infrastructure |
| `steel-core/src/advancement/criteria/inventory_changed.rs` | `InventoryChangedTrigger` |
| `steel-core/src/advancement/criteria/killed.rs` | `KilledTrigger` (player_killed_entity, entity_killed_player) |
| `steel-core/src/advancement/criteria/placed_block.rs` | `PlacedBlockTrigger` |
| `steel-core/src/advancement/criteria/consume_item.rs` | `ConsumeItemTrigger` |
| `steel-core/src/advancement/criteria/tick.rs` | `PlayerTrigger` (tick, location, slept_in_bed, etc.) |
| `steel-core/src/advancement/criteria/recipe_unlocked.rs` | `RecipeUnlockedTrigger` |
| `steel-core/src/advancement/criteria/changed_dimension.rs` | `ChangedDimensionTrigger` |
| `steel-core/src/advancement/tree.rs` | `AdvancementTree` + `AdvancementNode` + visibility evaluator |
| `steel-core/src/advancement/registry.rs` | Server-level advancement registry (holds the compiled tree) |
| `steel-core/src/advancement/rewards.rs` | `AdvancementRewards` + `grant()` impl |
| `steel-protocol/src/packets/game/stats.rs` | `CAwardStats` packet |
| `steel-protocol/src/packets/game/advancements.rs` | `CUpdateAdvancements`, `CSelectAdvancementsTab`, `SSeenAdvancements` |
| `steel-registry/build/advancements.rs` | Build script module to compile advancement JSONs |
| `steel-registry/src/generated/vanilla_advancements.rs` | Generated advancement definitions (**do not hand-edit**) |

---

## 7. Files to Modify

| Path | What changes |
|---|---|
| `steel-core/src/player/mod.rs` | Add `stats: SyncMutex<PlayerStats>` and `advancements: SyncMutex<PlayerAdvancements>` fields to `Player`; call `flush_dirty` in `tick()`; fill the two existing TODO comments |
| `steel-core/src/player/player_data.rs` | No change to binary format (stats/advancements are separate JSON files) |
| `steel-core/src/player/player_data_storage.rs` | Add `load_stats()`, `save_stats()`, `load_advancements()`, `save_advancements()` — reads/writes vanilla-format JSON side-by-side with existing STLP files |
| `steel-login/src/handlers/config.rs` | After player creation, load stats and advancements from disk; send initial `CUpdateAdvancements` (reset=true) and `CAwardStats` |
| `steel-core/src/player/networking.rs` | Add handler for `S_SEEN_ADVANCEMENTS` → `player.advancements.lock().set_selected_tab()` |
| `steel-core/src/behavior/blocks/container/crafting_table_block.rs` | Replace TODO with `player.stats.lock().increment_custom(CustomStat::InteractWithCraftingTable)` |
| `steel-core/src/behavior/blocks/container/barrel_block.rs` | Replace TODO with `player.stats.lock().increment_custom(CustomStat::OpenBarrel)` |
| `steel-core/src/behavior/items/honeycomb.rs` | Replace TODO with `CriteriaTriggers::ITEM_USED_ON_BLOCK.trigger(...)` |
| `steel-core/src/inventory/slot.rs` | Replace TODO with stat increments for picked_up/dropped |
| `steel-core/src/lib.rs` | Expose `advancement` module |
| `steel-protocol/src/packets/game/mod.rs` | Export new packet structs |
| `steel-registry/build/build.rs` | Include the `advancements` build module |

---

## 8. Trigger Callsite Map

Where each criterion trigger must be fired in the SteelMC codebase:

| Trigger | Fire location in SteelMC |
|---|---|
| `inventory_changed` | `steel-core/src/inventory/slot.rs` — after any slot change; `player.advancements.lock().trigger_inventory_changed(player)` |
| `player_killed_entity` | `steel-core/src/player/mod.rs` — in `kill()` or damage handler that causes entity death |
| `entity_killed_player` | `steel-core/src/player/mod.rs` — in `hurtServer()` when `health <= 0` |
| `placed_block` | `steel-core/src/player/networking.rs` — after `handle_use_item_on()` places a block |
| `item_used_on_block` | `steel-core/src/player/networking.rs` — in `handle_use_item_on()` |
| `consume_item` | `steel-core/src/player/mod.rs` — after food/potion consumption |
| `recipe_unlocked` | Recipe book system (not yet implemented) |
| `tick` | `steel-core/src/player/mod.rs` — in `tick()` |
| `changed_dimension` | Not yet implemented (dimension travel) |
| `slept_in_bed` | Not yet implemented (sleep) |
| `any_block_use` / `default_block_use` | `steel-core/src/player/networking.rs` — in `handle_use_item_on()` after block behavior returns |

---

## 9. Design Decisions & Constraints

**Why separate JSON files for stats/advancements (not the STLP binary)?**
Vanilla defines a stable JSON format for both. Using it means vanilla tools, Minecraft-compatible backup scripts, and the vanilla client can all read the data. The STLP binary is for internal state that has no external contract.

**Why hand-write the `CustomStat` enum?**
Only ~50 entries, no complex generation logic, and the enum needs to map to both a string identifier and a registry index. Generation would add more complexity than it saves. Matches CLAUDE.md: "hand write registries with complex logic unless they have a lot of entries (30+)" — the custom stat list does have 50+ entries, but each entry is just a constant; the serialization is trivial enough to stay hand-written given it maps 1:1 to vanilla's `Stats.java`.

**Why compile advancements at build time?**
1617 JSON files total. Parsing them at runtime on every server start would be slow and violates CLAUDE.md's rule against runtime JSON loading for vanilla data. Build-time compilation gives compile errors for malformed data and zero runtime parsing cost.

**Advancement progress storage: vanilla JSON vs Steel binary?**
Vanilla JSON is used for both stats and advancements to maintain save compatibility. If a user runs a vanilla server and SteelMC alternately, their progress is preserved. This also avoids implementing a custom migration path.

**Criteria trigger architecture: pull vs push?**
Vanilla uses a push model: each trigger type holds a per-player listener list. When a game event fires, the trigger iterates its listeners and checks predicates. SteelMC follows the same pattern. The alternative (polling all advancements every tick) would be O(n_advancements) per player per tick — unacceptable.

**Recipe advancements:**
The 1617 JSON files include ~1450 recipe advancements. These should be compiled by the build script along with gameplay advancements. However, firing the `recipe_unlocked` trigger depends on the recipe book system which is not yet implemented. Add them to the tree but leave the trigger call as a TODO until recipe books are done.

**Modding / ABI compatibility:**
- `CriterionTrigger` is a trait — mods can add new trigger types by implementing it and registering with the server's trigger registry
- `Advancement` data is loaded from a generated static slice for vanilla; mods supply their own slice at registration time
- `PlayerStats` uses typed registry refs (`BlockRef`, `ItemRef`, etc.) so mod-added blocks/items automatically get stat tracking if the caller passes their ref
- `PlayerAdvancements` does not assume a closed set of advancement IDs — `HashMap<Identifier, AdvancementProgress>` accommodates mod advancements without API changes

**What is explicitly out of scope for the initial implementation:**
- Advancement rewards: `function` command execution (requires command system)
- Advancement rewards: loot table grants (requires loot system)
- `changed_dimension` trigger (requires dimension travel)
- `slept_in_bed` trigger (requires sleep system)
- `nether_travel`, `levitation`, `ride_entity_in_lava` (requires vehicle/effect systems)
- Recipe book auto-unlock (requires recipe book)
- `/advancement` and `/stats` commands
- Server-side telemetry (`sends_telemetry_event` field — always ignored)
