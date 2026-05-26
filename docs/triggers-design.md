# Advancement Trigger System Design

Covers everything needed to go from manual `award()` calls to full vanilla-parity automatic trigger firing.

---

## Vanilla Architecture (brief)

Vanilla uses `SimpleCriterionTrigger<T>` as the base for 57 of the 58 triggers. The pattern:

1. A trigger singleton lives in `CriteriaTriggers` (static fields).
2. On player login, `PlayerAdvancements.registerListeners()` registers a `Listener<T>` for every incomplete criterion.
   A `Listener` stores: trigger instance (the condition data parsed from advancement JSON) + advancement holder + criterion name.
   It is added to the trigger's own `Map<PlayerAdvancements, Set<Listener<T>>>`.
3. When a game event fires, the trigger calls `trigger(ServerPlayer, Predicate<T> matcher)`:
   - Fetch all listeners for this player from the map.
   - For each listener, run `matcher.test(listener.triggerInstance)` (game-state check: "does the changed item match?").
   - If that passes, also check `listener.triggerInstance.player()` (optional `ContextAwarePredicate` = loot conditions on the player entity).
   - Matching listeners call `listener.run(advancements)` → `advancements.award(advancement, criterion)`.
4. On `award()`, completed listeners for that advancement are unregistered.

`ImpossibleTrigger` is the exception: it never fires and has no-op register/unregister methods.

**Key insight**: trigger objects are stateless and reused across all players; state lives in the per-player listener set stored on the trigger, keyed by `PlayerAdvancements` identity.

---

## Steel Architecture

### Deviation from vanilla: listener storage moves to the player

Vanilla stores `Map<PlayerAdvancements, Set<Listener>>` on the trigger. This requires a global lock when any player fires that trigger in parallel.

Steel instead stores `FxHashMap<&'static str, Vec<CriterionListener>>` on the player (trigger_id → listeners). When a trigger fires for a player, it receives `&mut PlayerAdvancements` (already locked for that tick) and simply iterates `player.trigger_listeners[trigger_id]`. No cross-player synchronisation needed.

The trigger object on the server is then purely a condition-evaluation helper with no mutable state — `Arc<dyn CriterionTrigger>` or a simple function pointer.

### Data layer changes

`StaticCriterionDef` gains a `conditions` field:

```rust
pub struct StaticCriterionDef {
    pub name: &'static str,
    pub trigger: &'static str,
    pub conditions: &'static [u8],  // raw JSON bytes of the "conditions" object
}
```

The build script already reads `criteria[name].conditions` from each advancement JSON. We just need to re-serialize those bytes into the generated file (using `serde_json::to_vec` or by writing the raw JSON string bytes).

`conditions` is `{}` for many criteria. For criteria that use predicates it contains the full JSON object.

### TriggerCondition — typed conditions parsed at startup

At server startup (`TriggerRegistry::new()`), every `StaticCriterionDef.conditions` is parsed into a `TriggerCondition` enum and stored in a flat table alongside the advancement data.  
This is a one-time cost.

```rust
pub enum TriggerCondition {
    Impossible,
    AlwaysFire { player: Option<PlayerPredicate> },      // tick, location (no-cond), etc.
    InventoryChanged {
        player: Option<PlayerPredicate>,
        slots: SlotsPredicate,
        items: Vec<ItemPredicate>,
    },
    RecipeUnlocked {
        player: Option<PlayerPredicate>,
        recipe: Option<&'static str>,
    },
    PlayerKilledEntity {
        player: Option<PlayerPredicate>,
        entity: Option<EntityPredicate>,
        killing_blow: Option<DamageSourcePredicate>,
    },
    EntityKilledPlayer {
        player: Option<PlayerPredicate>,
        entity: Option<EntityPredicate>,
        killing_blow: Option<DamageSourcePredicate>,
    },
    ConsumeItem {
        player: Option<PlayerPredicate>,
        item: Option<ItemPredicate>,
    },
    PlacedBlock {
        player: Option<PlayerPredicate>,
        block: Option<BlockPredicate>,
        item: Option<ItemPredicate>,
        location: Option<LocationPredicate>,
    },
    EnterBlock {
        player: Option<PlayerPredicate>,
        block: Option<BlockId>,
        state: Vec<(String, String)>,  // block state property → value
    },
    ChangedDimension {
        player: Option<PlayerPredicate>,
        from: Option<&'static str>,  // dimension resource key
        to: Option<&'static str>,
    },
    // ... (one variant per trigger, see trigger list below)
    Unknown,  // unimplemented trigger — criterion never fires, advancement stays incomplete
}
```

`Unknown` means the criterion silently never completes. This is safe: the advancement screen still renders, progress just stays 0 until we implement that trigger.

### CriterionListener — stored per player

```rust
pub struct CriterionListener {
    pub advancement_id: &'static str,
    pub criterion: &'static str,
    pub condition: &'static TriggerCondition,  // pointer into TriggerRegistry table
}
```

### PlayerAdvancements changes

```rust
pub struct PlayerAdvancements {
    // existing fields ...
    
    /// trigger_id → pending listeners for that trigger.
    /// Only criteria that are not yet complete are registered here.
    pub(crate) trigger_listeners: FxHashMap<&'static str, Vec<CriterionListener>>,
}
```

New methods:

```rust
impl PlayerAdvancements {
    /// Called once on login, after loading saved progress.
    /// Registers listeners for all incomplete criteria.
    pub fn register_all_listeners(
        &mut self,
        manager: &AdvancementManager,
        registry: &TriggerRegistry,
    )

    /// Called by each trigger when it fires. Returns true if anything was awarded.
    pub fn fire_trigger(
        &mut self,
        trigger_id: &str,
        ctx: &TriggerContext,        // game-state snapshot, see below
        player: &Player,
        manager: &AdvancementManager,
    ) -> bool

    /// Called internally after award() if the advancement is now fully done.
    fn unregister_listeners_for(&mut self, advancement_id: &'static str)
}
```

### TriggerContext — game-state snapshot passed to fire_trigger

Instead of a closure, the game state is bundled into a type-safe enum:

```rust
pub enum TriggerContext<'a> {
    Tick,
    InventoryChanged {
        inventory: &'a PlayerInventory,
        changed_item: &'a ItemStack,
        slots_full: usize,
        slots_empty: usize,
        slots_occupied: usize,
    },
    RecipeUnlocked {
        recipe_id: &'a str,
    },
    PlayerKilledEntity {
        entity: EntitySnapshot,
        killing_blow: &'a DamageSource,
    },
    EntityKilledPlayer {
        entity: EntitySnapshot,
        killing_blow: &'a DamageSource,
    },
    ConsumeItem {
        item: &'a ItemStack,
    },
    PlacedBlock {
        block: BlockId,
        state: BlockStateSnapshot,
        item: &'a ItemStack,
        location: BlockPos,
    },
    EnterBlock {
        state: BlockStateSnapshot,
    },
    ChangedDimension {
        from: DimensionKey,
        to: DimensionKey,
    },
    // ... etc
}
```

### Firing mechanism

Each trigger provides a `matches(condition: &TriggerCondition, ctx: &TriggerContext) -> bool` function. `PlayerAdvancements::fire_trigger` does:

```rust
pub fn fire_trigger(&mut self, trigger_id: &str, ctx: &TriggerContext, ...) -> bool {
    let Some(listeners) = self.trigger_listeners.get(trigger_id) else {
        return false;
    };

    let mut newly_awarded: Vec<(&'static str, &'static str)> = Vec::new();
    for listener in listeners {
        if trigger_matches(listener.condition, ctx) {
            // TODO: check optional player ContextAwarePredicate (phase 2)
            newly_awarded.push((listener.advancement_id, listener.criterion));
        }
    }

    let mut any = false;
    for (adv_id, criterion) in newly_awarded {
        if self.award(adv_id, criterion, manager) {
            any = true;
        }
    }
    any
}
```

`trigger_matches` is a free function that dispatches on the condition variant and context variant — a simple match statement. No vtables needed.

### TriggerRegistry on Server

```rust
pub struct TriggerRegistry {
    /// Pre-parsed TriggerCondition for every criterion in every advancement.
    /// Indexed the same way as AdvancementManager: advancement_id → criterion_name.
    conditions: FxHashMap<&'static str, FxHashMap<&'static str, TriggerCondition>>,
}

impl TriggerRegistry {
    pub fn new(manager: &AdvancementManager) -> Self {
        // Iterate VANILLA_ADVANCEMENTS, for each criterion parse conditions JSON
        // into a TriggerCondition enum.
    }

    pub fn get_condition(
        &self,
        advancement_id: &str,
        criterion: &str,
    ) -> Option<&TriggerCondition>
}
```

`Server` gains `pub trigger_registry: TriggerRegistry` alongside `advancement_manager`.

### Registration lifecycle

On player login (after loading saved progress):

```rust
player.advancements.lock().register_all_listeners(
    &server.advancement_manager,
    &server.trigger_registry,
);
```

In `PlayerAdvancements::award()`, after a criterion is granted and the advancement is now fully complete:

```rust
self.unregister_listeners_for(advancement_id);
```

On player logout: nothing to do — listeners live on the player struct, which is dropped.

---

## Predicate Types

Implement these bottom-up (simpler first). The `Option<T>` wrapping means an absent predicate always passes.

### Phase 1 — needed for most triggers

```rust
/// Range check [min, max], both optional.
pub struct MinMaxInt { pub min: Option<i32>, pub max: Option<i32> }
pub struct MinMaxFloat { pub min: Option<f64>, pub max: Option<f64> }

impl MinMaxInt {
    pub fn matches(&self, value: i32) -> bool {
        self.min.map_or(true, |m| value >= m) &&
        self.max.map_or(true, |m| value <= m)
    }
}
```

```rust
/// Item predicate — matches an ItemStack.
pub struct ItemPredicate {
    pub items: Option<Vec<ItemId>>,    // any of these item IDs
    pub count: MinMaxInt,
    // data components omitted until component system exists
}

impl ItemPredicate {
    pub fn matches(&self, stack: &ItemStack) -> bool {
        self.items.as_ref().map_or(true, |ids| ids.contains(&stack.item_id)) &&
        self.count.matches(stack.count as i32)
    }
}
```

```rust
/// Slot occupancy predicate for inventory_changed.
pub struct SlotsPredicate {
    pub occupied: MinMaxInt,
    pub full: MinMaxInt,
    pub empty: MinMaxInt,
}
```

### Phase 2 — for location/entity triggers

```rust
/// Simplified entity predicate (no sub-predicates yet).
pub struct EntityPredicate {
    pub entity_type: Option<EntityTypeId>,
    pub nbt: Option<Vec<u8>>,        // raw NBT bytes for matching
    // team, effects, equipment etc. added as needed
}

pub struct PlayerPredicate {
    // wraps EntityPredicate; also used as ContextAwarePredicate in vanilla
    pub entity: EntityPredicate,
}

pub struct LocationPredicate {
    pub dimension: Option<DimensionKey>,
    pub x: MinMaxFloat,
    pub y: MinMaxFloat,
    pub z: MinMaxFloat,
    pub biome: Option<&'static str>,
    // structure, light etc. added later
}
```

### Phase 3 — for damage/kill triggers

```rust
pub struct DamageSourcePredicate {
    pub tags: Vec<(DamageTypeTagId, bool)>,  // tag_id, expected
    pub direct_entity: Option<EntityPredicate>,
    pub source_entity: Option<EntityPredicate>,
}
```

### Phase 4 — block predicates

```rust
pub struct BlockPredicate {
    pub block: Option<BlockId>,
    pub state: Vec<(String, String)>,  // property → value
    pub nbt: Option<Vec<u8>>,
}
```

---

## The 58 Triggers

Grouped by callsite and complexity. Each entry shows: trigger ID | vanilla callsite | condition fields Steel needs.

### Group A — Always or near-always fire (no predicate matching needed beyond player check)

| Trigger | Fire callsite | Conditions |
|---------|--------------|------------|
| `impossible` | Never | (no-op) |
| `tick` | Player::tick() every tick | `player` only |
| `location` | Player::tick() if moved to new chunk | `player`, `location` |
| `slept_in_bed` | Player sleeps | `player`, `location` |
| `changed_dimension` | Player teleports to new dimension | `player`, `from`, `to` |
| `started_riding` | Player mounts entity | `player` |
| `avoid_vibration` | Allays pick up item | `player` |

### Group B — Inventory / items

| Trigger | Fire callsite | Conditions |
|---------|--------------|------------|
| `inventory_changed` | Inventory slot changes | `player`, `slots`, `items[]` |
| `recipe_unlocked` | Recipe added to recipe book | `player`, `recipe` |
| `consume_item` | Player uses consumable item | `player`, `item` |
| `item_durability_changed` | Item takes damage | `player`, `item`, `delta`, `durability` |
| `using_item` | Player holds and uses item (bow, spyglass…) | `player`, `item` |
| `filled_bucket` | Player fills bucket | `player`, `item` |
| `fishing_rod_hooked` | Rod catches entity/item | `player`, `rod`, `entity`, `item` |
| `recipe_crafted` | Player crafts from recipe | `player`, `recipe_id`, `ingredients[]` |
| `crafter_recipe_crafted` | Crafter block crafts | `player`, `recipe_id`, `ingredients[]` |
| `thrown_item_picked_up_by_entity` | Thrown item retrieved by entity | `player`, `item`, `entity` |
| `thrown_item_picked_up_by_player` | Entity-thrown item picked up by player | `player`, `item`, `entity` |

### Group C — Block interaction

| Trigger | Fire callsite | Conditions |
|---------|--------------|------------|
| `enter_block` | Player::tick() on block state change | `player`, `block`, `state` |
| `placed_block` | BlockItem.place() | `player`, `block`, `item`, `location`, `state` |
| `item_used_on_block` | Player right-clicks block with item | `player`, `location`, `item` |
| `default_block_use` | Player right-clicks block (no item) | `player`, `location` |
| `any_block_use` | Either of the above | `player`, `location` |
| `slide_down_block` | Player slides down block (honey) | `player`, `block` |
| `bee_nest_destroyed` | Bee nest destroyed | `player`, `block`, `item`, `num_bees` |
| `target_hit` | Target block hit | `player`, `signal_strength`, `projectile`, `shooter` |
| `allay_drop_item_on_block` | Allay drops item on note block | `player`, `location`, `item` |

### Group D — Combat

| Trigger | Fire callsite | Conditions |
|---------|--------------|------------|
| `player_killed_entity` | Entity::die() when killed by player | `player`, `entity`, `killing_blow` |
| `entity_killed_player` | Player::die() killed by entity | `player`, `entity`, `killing_blow` |
| `player_hurt_entity` | Player attacks entity | `player`, `entity`, `damage` |
| `entity_hurt_player` | Player receives damage | `player`, `damage` |
| `killed_by_arrow` | Player killed by arrow | `player`, `unique_entity_types` |
| `spear_mobs` | Trident hits entity | `player`, `projectile`, `target`, `thrown` |
| `shot_crossbow` | Player fires crossbow | `player`, `item` |
| `channeled_lightning` | Channeling trident hits | `player`, `victims[]` |
| `kill_mob_near_sculk_catalyst` | Mob killed near sculk catalyst | `player`, `entity`, `killing_blow` |
| `fall_after_explosion` | Player falls after being launched by explosion | `player`, `start_position`, `distance` |

### Group E — Mob interaction

| Trigger | Fire callsite | Conditions |
|---------|--------------|------------|
| `tame_animal` | AnimalTameEvent | `player`, `entity` |
| `bred_animals` | Breeding succeeds | `player`, `parent`, `partner`, `child` |
| `villager_trade` | Trade completes | `player`, `villager`, `item` |
| `cured_zombie_villager` | Zombie villager converted | `player`, `villager`, `zombie` |
| `summoned_entity` | Player summons entity (wither, golem, etc.) | `player`, `entity` |
| `player_interacted_with_entity` | Player right-clicks entity | `player`, `item`, `entity` |
| `player_sheared_equipment` | Player shears entity | `player`, `item`, `entity` |
| `hero_of_the_village` | Player gains Hero of the Village | `player`, `location` |
| `voluntary_exile` | Player gains Bad Omen | `player`, `location` |

### Group F — Effects / status

| Trigger | Fire callsite | Conditions |
|---------|--------------|------------|
| `effects_changed` | Mob effect added/removed/changed | `player`, `effects`, `source` |
| `levitation` | Levitation effect active | `player`, `distance`, `duration` |
| `enchanted_item` | Item enchanted at enchanting table | `player`, `item`, `levels` |
| `brewed_potion` | Brewing stand completes | `player`, `potion` |

### Group G — Exploration / distance

| Trigger | Fire callsite | Conditions |
|---------|--------------|------------|
| `nether_travel` | Player moves in nether (calc overworld distance) | `player`, `start_position`, `distance` |
| `fall_from_height` | Player lands after falling | `player`, `start_position`, `distance` |
| `ride_entity_in_lava` | Player rides entity through lava | `player`, `start_position`, `distance` |
| `used_ender_eye` | Ender eye thrown | `player`, `distance` |
| `lightning_strike` | Lightning strikes near player | `player`, `lightning`, `bystander` |
| `construct_beacon` | Beacon activates | `player`, `level` |

### Group H — Container / loot

| Trigger | Fire callsite | Conditions |
|---------|--------------|------------|
| `player_generates_container_loot` | Player opens loot container | `player`, `loot_table` |
| `used_totem` | Totem of undying activates | `player`, `item` |

---

## Build Script Changes (`steel-registry/build/advancements.rs`)

Add `conditions` byte slice to `StaticCriterionDef` output:

```rust
// In the generated StaticCriterionDef struct
pub struct StaticCriterionDef {
    pub name: &'static str,
    pub trigger: &'static str,
    pub conditions: &'static [u8],  // raw JSON bytes of {"conditions": {...}}
}
```

In the build script, when emitting each criterion:
```rust
let conditions_bytes = serde_json::to_vec(&criterion.conditions).unwrap_or_default();
// emit as: conditions: b"..." (byte literal)
```

`criterion.conditions` is `serde_json::Value` already parsed from the advancement JSON.

---

## Files to Create / Modify

### New files

| File | Contents |
|------|----------|
| `steel-core/src/server/trigger_registry.rs` | `TriggerRegistry`, `TriggerCondition` enum, `TriggerContext` enum, `trigger_matches()` dispatcher |
| `steel-core/src/server/predicates.rs` | `ItemPredicate`, `SlotsPredicate`, `EntityPredicate`, `PlayerPredicate`, `LocationPredicate`, `DamageSourcePredicate`, `BlockPredicate`, `MinMaxInt`, `MinMaxFloat` — with JSON deserialization via `serde` |

### Modified files

| File | Change |
|------|--------|
| `steel-registry/build/advancements.rs` | Add `conditions: &'static [u8]` to `StaticCriterionDef`; emit conditions bytes |
| `steel-registry/src/lib.rs` | (regenerated automatically) |
| `steel-core/src/server/mod.rs` | Add `mod trigger_registry; mod predicates;`; add `trigger_registry: TriggerRegistry` field on `Server`; init in `Server::new()`; call `register_all_listeners` at join |
| `steel-core/src/player/advancements.rs` | Add `trigger_listeners` field; add `register_all_listeners()`, `fire_trigger()`, `unregister_listeners_for()`; call `unregister_listeners_for` from `award()` when fully done |
| `steel-core/src/player/mod.rs` | Nothing extra — advancements.rs already covered |
| `steel-core/src/player/networking.rs` | Nothing (existing `S_SEEN_ADVANCEMENTS` handler already present) |
| Various game event callsites | Add `player.advancements.lock().fire_trigger(TRIGGER_ID, &ctx, ...)` at each point listed in the trigger table above |

---

## Implementation Order

1. **Build script** — add `conditions` bytes to `StaticCriterionDef`. Verify generated file contains byte slices.

2. **`predicates.rs`** — implement `MinMaxInt`, `MinMaxFloat`, `ItemPredicate`, `SlotsPredicate` with serde deserialization. Test with a few advancement condition JSONs.

3. **`trigger_registry.rs`** — implement `TriggerCondition` (start with `Impossible`, `AlwaysFire`, `InventoryChanged`, `RecipeUnlocked`, `PlayerKilledEntity`). Implement `TriggerRegistry::new()` which parses all conditions. Implement `TriggerContext`. Implement `trigger_matches()`.

4. **`PlayerAdvancements`** — add `trigger_listeners` field; implement `register_all_listeners`, `fire_trigger`, `unregister_listeners_for`. Wire into `award()`.

5. **Server init** — add `TriggerRegistry` to `Server`, call `register_all_listeners` at join.

6. **Callsites** — wire one trigger group at a time, starting with Group A (tick, location, changed_dimension) since they have no predicate data to parse, then Group B (inventory_changed, recipe_unlocked) as they cover ~40% of vanilla advancements, then Group D (combat) and so on.

---

## Vanilla source cross-reference

| Design element | Vanilla file |
|----------------|-------------|
| Trigger list | `advancements/critereon/CriteriaTriggers.java` |
| `CriterionTrigger` interface | `advancements/critereon/CriterionTrigger.java` |
| `SimpleCriterionTrigger` base | `advancements/critereon/SimpleCriterionTrigger.java` |
| Listener lifecycle | `server/PlayerAdvancements.java` lines 146–274 |
| `ContextAwarePredicate` | `advancements/critereon/ContextAwarePredicate.java` |
| `EntityPredicate` | `advancements/critereon/EntityPredicate.java` |
| `ItemPredicate` | `advancements/critereon/ItemPredicate.java` |
| `DamageSourcePredicate` | `advancements/critereon/DamageSourcePredicate.java` |
| `MinMaxBounds` | `advancements/critereon/MinMaxBounds.java` |
| `ImpossibleTrigger` | `advancements/critereon/ImpossibleTrigger.java` |
| `InventoryChangeTrigger` | `advancements/critereon/InventoryChangeTrigger.java` |
| `KilledTrigger` | `advancements/critereon/KilledTrigger.java` |
