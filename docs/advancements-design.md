# Advancements System Design

> Companion to `stats-advancements-design.md`. Stats are done. This document covers advancements only.

---

## 1. Vanilla reference map

Every section below references the Java source in `minecraft-src/minecraft/src/net/minecraft/`.

| What | Vanilla file |
|---|---|
| Advancement data record | `advancements/Advancement.java` |
| Advancement + ID pair | `advancements/AdvancementHolder.java` |
| Display metadata (icon, title, frame…) | `advancements/DisplayInfo.java` |
| AND/OR requirement logic | `advancements/AdvancementRequirements.java` |
| Per-player progress (all criteria) | `advancements/AdvancementProgress.java` |
| Single criterion completion timestamp | `advancements/CriterionProgress.java` |
| Trigger type interface | `advancements/CriterionTrigger.java` |
| All 50 trigger registrations | `advancements/CriteriaTriggers.java` |
| All trigger implementations | `advancements/criterion/` (~60 files) |
| Per-player state machine | `server/PlayerAdvancements.java` |
| Server-side advancement loader | `server/ServerAdvancementManager.java` |
| Visibility propagation algorithm | `server/advancements/AdvancementVisibilityEvaluator.java` |
| Tree structure | `advancements/AdvancementTree.java` / `AdvancementNode.java` |
| Clientbound update packet | `network/protocol/game/ClientboundUpdateAdvancementsPacket.java` |
| Clientbound tab select packet | `network/protocol/game/ClientboundSelectAdvancementsTabPacket.java` |
| Serverbound seen advancements | `network/protocol/game/ServerboundSeenAdvancementsPacket.java` |

---

## 2. System overview

```
Server start
  └─ Load 1617 advancement JSONs from datapack
  └─ Build AdvancementTree (parent→children, roots)

Player joins
  └─ Load {domain}/advancements/{uuid}.json  (progress timestamps)
  └─ Register trigger listeners for all INCOMPLETE advancements
  └─ flushDirty() → CUpdateAdvancements (full snapshot, reset=true)

Game event (e.g. block broken)
  └─ Trigger fires: trigger.trigger(player, predicate_fn)
  └─ Matching listeners call PlayerAdvancements::award(holder, criterion)
  └─ award() grants progress, unregisters done listeners, fires rewards
  └─ Marks visibility dirty

Next tick
  └─ flushDirty() → CUpdateAdvancements (delta: added, removed, progress)

Player opens advancement screen
  └─ Client sends SSeenAdvancements(OPENED_TAB, tab_id)
  └─ Server sends CSelectAdvancementsTab(tab_id)

Player closes screen
  └─ Client sends SSeenAdvancements(CLOSED_SCREEN)
```

---

## 3. Wire formats

### 3.1 `CUpdateAdvancements` (clientbound, most complex)

```
bool            reset           // true only on first packet after login
VarInt          added_count
[added_count × AdvancementHolder]
  Identifier    id
  Advancement:
    Optional<Identifier>  parent          // None = root
    Optional<DisplayInfo>:
      Component           title           // trusted stream codec
      Component           description
      ItemStackTemplate   icon
      Enum(u8)            frame           // 0=TASK 1=CHALLENGE 2=GOAL
      i32                 flags           // bit0=has_background, bit1=show_toast, bit2=hidden
      Identifier?         background      // only if flags&1
      f32                 x
      f32                 y
    AdvancementRequirements:
      VarInt              group_count
      [group_count × VarInt+[String...]]  // outer=AND, inner=OR
    bool                  sends_telemetry
VarInt          removed_count
[removed_count × Identifier]
VarInt          progress_count
[progress_count × (Identifier, AdvancementProgress)]
  Identifier    advancement_id
  VarInt        criterion_count
  [criterion_count × (String, CriterionProgress)]
    String      criterion_name
    bool?       obtained_instant  // writeNullable(Instant) = bool + i64 epoch_second + i32 nanos
bool            show_advancements
```

Key insight: the **added** list sends advancement definitions (display, requirements), but **not** the criteria names or trigger types — the client only needs to render the tree and requirements for the progress bar. The server keeps all trigger logic server-side.

### 3.2 `CSelectAdvancementsTab` (clientbound)

```
Optional<Identifier>  tab_id   // None = close tab
```

### 3.3 `SSeenAdvancements` (serverbound)

```
Enum(VarInt)    action         // 0=OPENED_TAB, 1=CLOSED_SCREEN
Identifier?     tab_id         // only present when action=OPENED_TAB
```

---

## 4. Advancement data model

### 4.1 Advancement definition (static, from datapack JSON)

```rust
pub struct Advancement {
    pub parent: Option<Identifier>,
    pub display: Option<DisplayInfo>,
    pub rewards: AdvancementRewards,
    pub criteria: HashMap<String, Criterion>,    // name → trigger+conditions
    pub requirements: AdvancementRequirements,   // AND(OR(criteria...))
    pub sends_telemetry: bool,
}

pub struct DisplayInfo {
    pub title: TextComponent,
    pub description: TextComponent,
    pub icon: ItemStackTemplate,
    pub background: Option<Identifier>,          // only on root advancements
    pub frame: AdvancementType,                  // Task / Challenge / Goal
    pub show_toast: bool,
    pub announce_to_chat: bool,
    pub hidden: bool,
    pub x: f32,                                  // tree position, set by TreeNodePosition
    pub y: f32,
}

pub enum AdvancementType { Task, Challenge, Goal }

/// Outer vec = AND groups, inner vec = OR alternatives within group.
/// All groups must be satisfied. Within a group, any one criterion suffices.
pub struct AdvancementRequirements(pub Vec<Vec<String>>);

pub struct AdvancementRewards {
    pub experience: i32,
    pub loot_tables: Vec<Identifier>,
    pub recipes: Vec<Identifier>,
    pub function: Option<Identifier>,
}
```

### 4.2 Per-player progress (mutable, per player)

```rust
pub struct AdvancementProgress {
    pub criteria: HashMap<String, CriterionProgress>,
}

pub struct CriterionProgress {
    pub obtained: Option<Instant>,   // Some = done, with timestamp for JSON/network
}

impl AdvancementProgress {
    pub fn is_done(&self, requirements: &AdvancementRequirements) -> bool {
        // Every AND group must have at least one done criterion
        requirements.0.iter().all(|group| {
            group.iter().any(|c| self.criteria.get(c).map_or(false, |p| p.obtained.is_some()))
        })
    }

    pub fn has_progress(&self) -> bool {
        self.criteria.values().any(|p| p.obtained.is_some())
    }
}
```

---

## 5. Trigger system

### 5.1 Trait design

```rust
/// Marker trait for trigger condition data (deserialized from advancement JSON).
pub trait CriterionTriggerInstance: Send + Sync + 'static {
    fn trigger_type_id(&self) -> &'static Identifier;
}

/// A registered trigger type. Each variant is a singleton (like vanilla's CriteriaTriggers).
pub trait CriterionTrigger: Send + Sync + 'static {
    type Instance: CriterionTriggerInstance;

    fn id(&self) -> &'static Identifier;

    /// Register a listener: "when this trigger fires for `player_advancements`,
    /// check `instance` conditions and award `advancement/criterion` if matched."
    fn add_listener(&self, advancements: &Arc<PlayerAdvancements>, listener: TriggerListener<Self::Instance>);
    fn remove_listener(&self, advancements: &Arc<PlayerAdvancements>, listener: &TriggerListener<Self::Instance>);
    fn remove_all_listeners(&self, advancements: &Arc<PlayerAdvancements>);
}

pub struct TriggerListener<T> {
    pub instance: T,
    pub advancement_id: Identifier,
    pub criterion: String,
}
```

### 5.2 The 50 trigger types

Vanilla has 50+ types. For initial implementation, prioritize the ones used by the most advancements. Full list is in `CriteriaTriggers.java`.

**Tier 1 — needed immediately** (used by most progression advancements):

| Trigger | Vanilla class | Fires when |
|---|---|---|
| `minecraft:tick` | `PlayerTrigger` | Every player tick (for location/time-based checks) |
| `minecraft:inventory_changed` | `InventoryChangeTrigger` | Item enters/leaves inventory |
| `minecraft:player_killed_entity` | `KilledTrigger` | Player kills entity |
| `minecraft:entity_killed_player` | `KilledTrigger` | Entity kills player |
| `minecraft:recipe_crafted` | `RecipeCraftedTrigger` | Player crafts a recipe |
| `minecraft:consume_item` | `ConsumeItemTrigger` | Player eats/drinks |
| `minecraft:placed_block` | `ItemUsedOnLocationTrigger` | Player places a block |
| `minecraft:enter_block` | `EnterBlockTrigger` | Player enters a block |
| `minecraft:changed_dimension` | `ChangeDimensionTrigger` | Player travels between dimensions |
| `minecraft:impossible` | `ImpossibleTrigger` | Never fires (manually awarded only) |

**Tier 2 — common** (needed for most story/nether/end tabs):

`enchanted_item`, `filled_bucket`, `brewed_potion`, `construct_beacon`, `bred_animals`, `tame_animal`, `slept_in_bed`, `cured_zombie_villager`, `villager_trade`, `summoned_entity`, `levitation`, `used_ender_eye`, `effects_changed`, `used_totem`, `nether_travel`, `fall_from_height`, `location`

**Tier 3 — advanced** (challenge advancements, less common):

Everything else in `CriteriaTriggers.java`.

### 5.3 Condition predicates

Each trigger instance carries **conditions** from the JSON — e.g. `InventoryChangeTrigger` conditions specify which items must be in the inventory. Vanilla uses a complex `Codec`-based predicate system (`ItemPredicate`, `EntityPredicate`, `LocationPredicate`, etc.).

**Design decision**: For initial implementation, implement predicates as opaque `serde_json::Value` conditions that are checked lazily. Only parse and evaluate the predicates actually needed for Tier 1 triggers. This avoids implementing the entire 60-file predicate system upfront.

---

## 6. Advancement loading (build-time vs. runtime)

**CLAUDE.md rule**: No runtime JSON parsing for game data. Advancement definitions must be compiled at build time.

**The challenge**: There are 1617 vanilla advancement JSON files in the datapack. These need to be compiled into typed Rust data by a build script.

### Option A: Build script generates Rust code (matches CLAUDE.md)
- `steel-registry/build/advancements.rs` reads all 1617 JSONs from extracted datapack
- Generates a `static VANILLA_ADVANCEMENTS: &[AdvancementDef]` array
- Each entry: id, parent, display info, requirements, criteria trigger IDs + condition bytes
- **Pro**: Fast startup, type-checked, no serde_json at runtime
- **Con**: Large generated file; conditions still need runtime JSON interpretation (they're per-instance)

### Option B: Build script compiles to a binary blob
- Serialize all 1617 advancements to a compact binary format at build time
- `include_bytes!` it at compile time, deserialize once at server start
- **Pro**: Minimal generated code size
- **Con**: Slightly more complex build step

**Recommended**: Option A for structure (advancement tree, display, requirements), Option B for condition payloads. The trigger type and requirement structure can be fully typed; the condition data (`ItemPredicate`, `EntityPredicate`, etc.) can remain as raw `Vec<u8>` that gets deserialized on-demand when a trigger fires.

The extractor (`SteelExtractor`) may already export advancement JSON files. Ask the user to confirm what's available before writing the build script.

---

## 7. Per-player state (`PlayerAdvancements`)

```rust
pub struct PlayerAdvancements {
    /// Progress for every advancement the player has interacted with.
    progress: FxHashMap<Identifier, AdvancementProgress>,
    /// Which advancements are currently visible to this player.
    visible: FxHashSet<Identifier>,
    /// Advancements whose progress changed since last flush.
    progress_changed: FxHashSet<Identifier>,
    /// Root nodes whose subtree needs visibility recalculation.
    roots_to_update: FxHashSet<Identifier>,
    /// Currently selected tab (root advancement ID).
    last_selected_tab: Option<Identifier>,
    /// True for the very first flush after login (triggers reset=true packet).
    is_first_packet: bool,
}

impl PlayerAdvancements {
    /// Grant a criterion. Returns true if the criterion was newly granted.
    /// If all requirements are now satisfied, fires rewards and announces.
    pub fn award(&mut self, advancement_id: &Identifier, criterion: &str, tree: &AdvancementTree, player: &Player) -> bool;

    /// Called every tick from Player::tick() when dirty.
    pub fn flush_dirty(&mut self, player: &Player, tree: &AdvancementTree, show_advancements: bool);

    /// Called when player opens the advancements screen (SSeenAdvancements).
    pub fn set_selected_tab(&mut self, tab_id: Option<&Identifier>, player: &Player);

    /// Called on login: load from disk, register listeners, mark first packet.
    pub fn load(&mut self, path: &Path, tree: &AdvancementTree);

    /// Save progress to disk.
    pub fn save(&self, path: &Path) -> io::Result<()>;

    /// Register trigger listeners for all incomplete advancements.
    fn register_listeners(&mut self, tree: &AdvancementTree, triggers: &TriggerRegistry);

    /// Recalculate visibility for a root subtree.
    fn update_tree_visibility(&mut self, root_id: &Identifier, tree: &AdvancementTree,
                              added: &mut Vec<Identifier>, removed: &mut Vec<Identifier>);
}
```

**Where it lives**: `steel-core/src/player/advancements.rs`

**Threading**: `PlayerAdvancements` is owned by `Player` behind a `SyncMutex<PlayerAdvancements>`, same pattern as `PlayerStats`. It is only accessed from the player's tick context.

---

## 8. Trigger registry

A global singleton that maps `Identifier → Box<dyn CriterionTrigger>` and manages per-player listener subscriptions.

```rust
// steel-core/src/player/advancements/trigger_registry.rs
pub struct TriggerRegistry {
    triggers: FxHashMap<Identifier, Box<dyn ErasedCriterionTrigger>>,
    /// Per-player listeners: trigger_id → player_uuid → [listener_data]
    listeners: SyncMutex<FxHashMap<Identifier, FxHashMap<Uuid, Vec<RawListener>>>>,
}

pub struct RawListener {
    pub advancement_id: Identifier,
    pub criterion: String,
    pub conditions: serde_json::Value,   // raw trigger conditions, checked on fire
}

impl TriggerRegistry {
    /// Fire a trigger for a specific player. Checks all registered listeners
    /// and awards matching criteria.
    pub fn trigger(&self, trigger_id: &Identifier, player: &Player, context: TriggerContext);
}
```

**Where it lives**: `steel-core/src/player/advancements/trigger_registry.rs`

---

## 9. Advancement tree

```rust
// steel-core/src/player/advancements/tree.rs
pub struct AdvancementTree {
    /// All advancements by ID.
    advancements: FxHashMap<Identifier, Arc<AdvancementDef>>,
    /// Parent → children relationships.
    children: FxHashMap<Identifier, Vec<Identifier>>,
    /// Root advancement IDs (no parent).
    roots: Vec<Identifier>,
}

pub struct AdvancementDef {
    pub id: Identifier,
    pub advancement: Advancement,
    pub criteria: FxHashMap<String, CriterionDef>,
}

pub struct CriterionDef {
    pub trigger_id: Identifier,
    pub conditions: serde_json::Value,
}
```

Built once at server start from the compiled advancement data.

---

## 10. Visibility algorithm

Direct port of `AdvancementVisibilityEvaluator.java`. Rules:
- If advancement has no `display` → always hidden
- If advancement is **done** → always visible
- If advancement has `display.hidden = true` → hidden unless done
- Otherwise: visible if within depth 2 of a visible/done ancestor

The algorithm does a DFS from the root. An advancement is visible if:
- It is done, **or**
- Any descendant is done, **or**
- Any ancestor within 2 hops has `SHOW` visibility

```rust
fn evaluate_visibility(
    node: &Identifier,
    tree: &AdvancementTree,
    progress: &FxHashMap<Identifier, AdvancementProgress>,
    ascendants: &mut Vec<VisibilityRule>,
    output: &mut impl FnMut(&Identifier, bool),
) -> bool { ... }
```

---

## 11. Persistence format

Vanilla uses JSON per player at `{world}/advancements/{uuid}.json`.

**Steel decision**: Same choice as stats — use Steel binary format (`STLA` magic + wincode + zstd), stored at `{domain}/advancements/{uuid}.dat`.

The saved data: only advancements with **any** progress (`has_progress() == true`). Each entry: `advancement_id → [(criterion_name, epoch_seconds)]`.

---

## 12. New files to create

```
steel-core/src/player/advancements/
  mod.rs               PlayerAdvancements struct + load/save/flush/award
  tree.rs              AdvancementTree + AdvancementDef + build from static data
  trigger_registry.rs  TriggerRegistry + listener management
  triggers/
    mod.rs             TriggerContext enum, ErasedCriterionTrigger trait
    impossible.rs      ImpossibleTrigger (never fires)
    inventory.rs       InventoryChangeTrigger (fires on inventory change)
    tick.rs            PlayerTrigger/TICK (fires every tick)
    killed.rs          KilledTrigger (entity kills)
    crafted.rs         RecipeCraftedTrigger
    consume.rs         ConsumeItemTrigger
    placed_block.rs    ItemUsedOnLocationTrigger (placed_block variant)
    enter_block.rs     EnterBlockTrigger
    dimension.rs       ChangeDimensionTrigger
    ... (one file per trigger type, add as needed)

steel-protocol/src/packets/game/
  c_update_advancements.rs   CUpdateAdvancements + all sub-types
  c_select_advancements_tab.rs
  s_seen_advancements.rs

steel-registry/build/
  advancements.rs      Build script: reads 1617 JSONs → generates AdvancementDef array
```

---

## 13. Files to modify

| File | Change |
|---|---|
| `steel-core/src/player/mod.rs` | Add `pub advancements: SyncMutex<PlayerAdvancements>` field; call `flush_dirty()` in `tick()` |
| `steel-core/src/player/player_data_storage.rs` | Add `load_advancements(domain, uuid)` / `save_advancements(domain, uuid, bytes)` |
| `steel-core/src/server/mod.rs` | Load advancements on join / domain switch; save on leave |
| `steel-core/src/world/world_entities.rs` | Save advancements on `remove_player()` |
| `steel-protocol/src/packets/game/mod.rs` | Export new packets |
| `steel-core/src/player/networking.rs` | Handle `SSeenAdvancements` |
| Various behavior/inventory files | Fire trigger callsites (block placed, item consumed, entity killed, etc.) |

---

## 14. Recommended implementation order

1. **Packets first** — `CUpdateAdvancements`, `CSelectAdvancementsTab`, `SSeenAdvancements`. No logic, just wire encoding. Verify client can receive an empty update without crashing.

2. **Data types** — `AdvancementProgress`, `CriterionProgress`, `AdvancementDef`, `AdvancementRequirements`, `AdvancementTree`. These are just structs.

3. **Build script** — Read 1617 JSONs, generate static Rust data. Confirm extractor output path with user first.

4. **`PlayerAdvancements`** — Stub with `new()`, `load()`, `save()`, `flush_dirty()`. For the first pass, send the full tree on join with empty progress so the client renders the advancement screen correctly. No triggers yet.

5. **Visibility evaluator** — Port the DFS algorithm. Required before the client will show advancements correctly (hidden advancements must be filtered out).

6. **Trigger registry** — Wire up `TriggerRegistry`, register Tier 1 triggers.

7. **Trigger callsites** — Add fire points in behavior code (block break, entity kill, etc.).

8. **Rewards** — XP grant (already works via `Experience`), recipe unlock (TODO: recipe system), loot tables (TODO), chat announcement.

---

## 15. Known blockers

| Blocker | Impact |
|---|---|
| **Build script for advancement JSONs** — extractor output path unknown | Blocks step 3. Must confirm with user before writing build script. |
| **`ItemStackTemplate` wire encoding** — used in `DisplayInfo.icon` | Need to check if Steel already encodes this, or implement for the packet. |
| **`TextComponent` trusted stream codec** — title/description use a different codec than chat | Check if `steel-protocol` already has a trusted component encoder. |
| **Recipe unlock reward** — `AdvancementRewards.recipes` | Blocked on recipe system being wired to advancements. |
| **Loot table reward** — `AdvancementRewards.loot_tables` | Loot table execution is not yet wired. |
| **Complex predicates** — `EntityPredicate`, `LocationPredicate`, `ItemPredicate` | Must decide: fully implement now, or start with conditions always-true and refine later. |
| **`TICK` trigger** — fires every tick for every player with location/time conditions | Performance concern: check if firing this per-tick per-player is acceptable, or needs coarser granularity. |
