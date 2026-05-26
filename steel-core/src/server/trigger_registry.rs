//! Advancement trigger condition registry.
//!
//! ## Design differences from vanilla
//!
//! Vanilla stores per-player listener sets on each trigger singleton
//! (`SimpleCriterionTrigger.players: Map<PlayerAdvancements, Set<Listener>>`),
//! requiring a cross-player lock on every trigger fire.
//!
//! Here the listener storage lives on `PlayerAdvancements`, keyed by trigger ID.
//! `TriggerRegistry` is purely a lookup table of parsed `TriggerCondition`s —
//! it has no mutable state and needs no synchronisation.
//!
//! ## Condition data
//!
//! `StaticCriterionDef::conditions` holds the raw JSON bytes for each criterion's
//! `conditions` object (may be `b"null"` for absent conditions).
//! `TriggerRegistry::new()` parses those bytes once at server startup into typed
//! `TriggerCondition` variants.  Unimplemented triggers become `Unknown`, which
//! means those criteria never fire — the advancement stays incomplete but renders
//! correctly in the screen.

use rustc_hash::FxHashMap;
use serde::Deserialize;
use steel_registry::vanilla_advancements::VANILLA_ADVANCEMENTS;

use crate::server::predicates::{
    DamageSourcePredicate, EntityPredicate, ItemPredicate, LocationPredicate, MinMaxFloat,
    MinMaxInt, SlotsPredicate,
};

// ─────────────────────────────────────────────────────────────────────────────
// TriggerCondition — one variant per vanilla trigger type
// ─────────────────────────────────────────────────────────────────────────────

/// Parsed conditions for one advancement criterion.
///
/// Mirrors vanilla's per-trigger `TriggerInstance` records but unified into a
/// single enum so condition matching can be a plain `match` with no vtables.
///
/// The `player` optional predicate present in all `SimpleCriterionTrigger`
/// instances is omitted for now — it requires the full loot-context system.
/// All triggers effectively treat the player predicate as always-passing.
#[expect(missing_docs, reason = "variant field names directly mirror vanilla TriggerInstance condition keys")]
#[derive(Debug, Clone)]
pub enum TriggerCondition {
    // ── Group A: always / near-always fire ───────────────────────────────────
    /// `minecraft:impossible` — never fires.
    Impossible,
    /// `minecraft:tick` — fires every player tick (no extra conditions).
    Tick,
    /// `minecraft:location` — fires when the player moves to a new chunk position.
    Location { location: Option<LocationPredicate> },
    /// `minecraft:slept_in_bed` — fires when the player successfully sleeps.
    SleptInBed { location: Option<LocationPredicate> },
    /// `minecraft:changed_dimension` — fires on dimension transition.
    ChangedDimension {
        from: Option<String>,
        to: Option<String>,
    },
    /// `minecraft:started_riding` — fires when the player mounts a vehicle.
    StartedRiding,
    /// `minecraft:avoid_vibration` — fires when player avoids sculk vibration.
    AvoidVibration,

    // ── Group B: inventory / items ────────────────────────────────────────────
    /// `minecraft:inventory_changed` — fires on any inventory slot change.
    InventoryChanged {
        slots: SlotsPredicate,
        items: Vec<ItemPredicate>,
    },
    /// `minecraft:recipe_unlocked` — fires when a recipe is added to the recipe book.
    RecipeUnlocked { recipe: Option<String> },
    /// `minecraft:consume_item` — fires when the player finishes using a consumable.
    ConsumeItem { item: Option<ItemPredicate> },
    /// `minecraft:item_durability_changed` — fires when an item loses durability.
    ItemDurabilityChanged {
        item: Option<ItemPredicate>,
        delta: MinMaxInt,
        durability: MinMaxInt,
    },
    /// `minecraft:using_item` — fires each tick while the player is using an item.
    UsingItem { item: Option<ItemPredicate> },
    /// `minecraft:filled_bucket` — fires when the player fills a bucket.
    FilledBucket { item: Option<ItemPredicate> },
    /// `minecraft:fishing_rod_hooked` — fires when the fishing rod catches something.
    FishingRodHooked {
        rod: Option<ItemPredicate>,
        entity: Option<EntityPredicate>,
        item: Option<ItemPredicate>,
    },
    /// `minecraft:recipe_crafted` — fires when a recipe is crafted.
    RecipeCrafted {
        recipe_id: Option<String>,
        ingredients: Vec<ItemPredicate>,
    },
    /// `minecraft:crafter_recipe_crafted` — fires when a crafter block completes a recipe.
    CrafterRecipeCrafted {
        recipe_id: Option<String>,
        ingredients: Vec<ItemPredicate>,
    },
    /// `minecraft:thrown_item_picked_up_by_entity` — fired when entity picks up thrown item.
    ThrownItemPickedUpByEntity {
        item: Option<ItemPredicate>,
        entity: Option<EntityPredicate>,
    },
    /// `minecraft:thrown_item_picked_up_by_player` — fired when player picks up thrown item.
    ThrownItemPickedUpByPlayer {
        item: Option<ItemPredicate>,
        entity: Option<EntityPredicate>,
    },

    // ── Group C: block interaction ────────────────────────────────────────────
    /// `minecraft:enter_block` — fires when the player's block position changes.
    EnterBlock {
        block: Option<String>,
        state: Vec<(String, String)>,
    },
    /// `minecraft:placed_block` — fires when the player places a block.
    PlacedBlock {
        block: Option<String>,
        item: Option<ItemPredicate>,
        location: Option<LocationPredicate>,
    },
    /// `minecraft:item_used_on_block` — fires when item is right-clicked on a block.
    ItemUsedOnBlock { location: Option<LocationPredicate>, item: Option<ItemPredicate> },
    /// `minecraft:default_block_use` — fires on right-click block with no item.
    DefaultBlockUse { location: Option<LocationPredicate> },
    /// `minecraft:any_block_use` — fires on any block use (item or bare).
    AnyBlockUse { location: Option<LocationPredicate> },
    /// `minecraft:slide_down_block` — fires when player slides down a block (honey, etc.).
    SlideDownBlock { block: Option<String> },
    /// `minecraft:bee_nest_destroyed` — fires when a bee nest is broken.
    BeeNestDestroyed {
        block: Option<String>,
        item: Option<ItemPredicate>,
        num_bees: MinMaxInt,
    },
    /// `minecraft:target_hit` — fires when a target block is hit.
    TargetHit { signal_strength: MinMaxInt, projectile: Option<EntityPredicate> },
    /// `minecraft:allay_drop_item_on_block` — fires when an allay drops an item on a note block.
    AllayDropItemOnBlock { location: Option<LocationPredicate>, item: Option<ItemPredicate> },

    // ── Group D: combat ───────────────────────────────────────────────────────
    /// `minecraft:player_killed_entity` — fires when the player kills an entity.
    PlayerKilledEntity {
        entity: Option<EntityPredicate>,
        killing_blow: Option<DamageSourcePredicate>,
    },
    /// `minecraft:entity_killed_player` — fires when an entity kills the player.
    EntityKilledPlayer {
        entity: Option<EntityPredicate>,
        killing_blow: Option<DamageSourcePredicate>,
    },
    /// `minecraft:player_hurt_entity` — fires when the player damages an entity.
    PlayerHurtEntity {
        entity: Option<EntityPredicate>,
        damage: Option<DamageSourcePredicate>,
    },
    /// `minecraft:entity_hurt_player` — fires when an entity damages the player.
    EntityHurtPlayer { damage: Option<DamageSourcePredicate> },
    /// `minecraft:killed_by_arrow` — fires when the player is killed by an arrow.
    KilledByArrow { unique_entity_types: Option<MinMaxInt> },
    /// `minecraft:spear_mobs` — fires when a trident hits an entity.
    SpearMobs {
        projectile: Option<EntityPredicate>,
        target: Option<EntityPredicate>,
        thrown: bool,
    },
    /// `minecraft:shot_crossbow` — fires when the player shoots a crossbow.
    ShotCrossbow { item: Option<ItemPredicate> },
    /// `minecraft:channeled_lightning` — fires when the player channels a lightning bolt.
    ChanneledLightning { victims: Vec<EntityPredicate> },
    /// `minecraft:kill_mob_near_sculk_catalyst` — fires when a mob is killed near a sculk catalyst.
    KillMobNearSculkCatalyst {
        entity: Option<EntityPredicate>,
        killing_blow: Option<DamageSourcePredicate>,
    },
    /// `minecraft:fall_after_explosion` — fires when a player lands after being launched by an explosion.
    FallAfterExplosion { distance: Option<MinMaxFloat> },

    // ── Group E: mob interaction ──────────────────────────────────────────────
    /// `minecraft:tame_animal` — fires when the player tames an animal.
    TameAnimal { entity: Option<EntityPredicate> },
    /// `minecraft:bred_animals` — fires when the player breeds animals.
    BredAnimals {
        parent: Option<EntityPredicate>,
        partner: Option<EntityPredicate>,
        child: Option<EntityPredicate>,
    },
    /// `minecraft:villager_trade` — fires when the player trades with a villager.
    VillagerTrade { villager: Option<EntityPredicate>, item: Option<ItemPredicate> },
    /// `minecraft:cured_zombie_villager` — fires when a zombie villager is cured.
    CuredZombieVillager {
        villager: Option<EntityPredicate>,
        zombie: Option<EntityPredicate>,
    },
    /// `minecraft:summoned_entity` — fires when the player summons an entity (wither, golem, etc.).
    SummonedEntity { entity: Option<EntityPredicate> },
    /// `minecraft:player_interacted_with_entity` — fires when the player right-clicks an entity.
    PlayerInteractedWithEntity {
        item: Option<ItemPredicate>,
        entity: Option<EntityPredicate>,
    },
    /// `minecraft:player_sheared_equipment` — fires when the player shears an entity.
    PlayerShearedEquipment {
        item: Option<ItemPredicate>,
        entity: Option<EntityPredicate>,
    },
    /// `minecraft:hero_of_the_village` — fires when the player gains Hero of the Village.
    HeroOfTheVillage { location: Option<LocationPredicate> },
    /// `minecraft:voluntary_exile` — fires when the player gains Bad Omen.
    VoluntaryExile { location: Option<LocationPredicate> },

    // ── Group F: effects / status ─────────────────────────────────────────────
    /// `minecraft:effects_changed` — fires when an effect is added, removed, or changed.
    EffectsChanged,
    /// `minecraft:levitation` — fires while the player has levitation.
    Levitation { distance: Option<MinMaxFloat>, duration: MinMaxInt },
    /// `minecraft:enchanted_item` — fires when the player enchants an item.
    EnchantedItem { item: Option<ItemPredicate>, levels: MinMaxInt },
    /// `minecraft:brewed_potion` — fires when a brewing stand completes.
    BrewedPotion { potion: Option<String> },

    // ── Group G: exploration / distance ──────────────────────────────────────
    /// `minecraft:nether_travel` — fires when the player moves in the nether.
    NetherTravel { distance: Option<MinMaxFloat> },
    /// `minecraft:fall_from_height` — fires when the player lands after falling.
    FallFromHeight { distance: Option<MinMaxFloat> },
    /// `minecraft:ride_entity_in_lava` — fires when riding through lava.
    RideEntityInLava { distance: Option<MinMaxFloat> },
    /// `minecraft:used_ender_eye` — fires when an ender eye is thrown.
    UsedEnderEye { distance: Option<MinMaxFloat> },
    /// `minecraft:lightning_strike` — fires when lightning strikes near the player.
    LightningStrike {
        lightning: Option<EntityPredicate>,
        bystander: Option<EntityPredicate>,
    },
    /// `minecraft:construct_beacon` — fires when a beacon is activated.
    ConstructBeacon { level: MinMaxInt },

    // ── Group H: container / loot ─────────────────────────────────────────────
    /// `minecraft:player_generates_container_loot` — fires when a loot container is opened.
    PlayerGeneratesContainerLoot { loot_table: Option<String> },
    /// `minecraft:used_totem` — fires when a totem of undying activates.
    UsedTotem { item: Option<ItemPredicate> },

    // ── Catch-all ─────────────────────────────────────────────────────────────
    /// Trigger type not yet implemented. Criterion never fires.
    Unknown,
}

// ─────────────────────────────────────────────────────────────────────────────
// TriggerContext — game-state snapshot passed to fire_trigger
// ─────────────────────────────────────────────────────────────────────────────

/// Snapshot of the game state at the moment a trigger fires.
///
/// Each variant corresponds to a group of related trigger types. `fire_trigger`
/// iterates the player's pending listeners for `trigger_id` and calls
/// `condition_matches(condition, ctx)` to decide whether to award each criterion.
///
/// Variant names and field names exactly mirror the trigger IDs and condition fields
/// they represent, so individual field docs would be redundant.
#[expect(missing_docs, reason = "variant and field names are self-documenting game-state snapshots")]
pub enum TriggerContext<'a> {
    Tick,
    Location {
        dimension_key: &'a str,
    },
    SleptInBed {
        dimension_key: &'a str,
    },
    ChangedDimension {
        from: &'a str,
        to: &'a str,
    },
    StartedRiding,
    AvoidVibration,
    InventoryChanged {
        changed_item_key: &'a str,
        changed_item_count: i32,
        slots_occupied: i32,
        slots_full: i32,
        slots_empty: i32,
        all_items: &'a [(String, i32)],
    },
    RecipeUnlocked {
        recipe_id: &'a str,
    },
    ConsumeItem {
        item_key: &'a str,
        item_count: i32,
    },
    ItemDurabilityChanged {
        item_key: &'a str,
        item_count: i32,
        delta: i32,
        durability: i32,
    },
    UsingItem {
        item_key: &'a str,
        item_count: i32,
    },
    FilledBucket {
        item_key: &'a str,
        item_count: i32,
    },
    FishingRodHooked {
        rod_key: &'a str,
        rod_count: i32,
        entity_key: Option<&'a str>,
        item_key: Option<&'a str>,
        item_count: i32,
    },
    RecipeCrafted {
        recipe_id: &'a str,
        ingredient_keys: &'a [(String, i32)],
    },
    CrafterRecipeCrafted {
        recipe_id: &'a str,
        ingredient_keys: &'a [(String, i32)],
    },
    ThrownItemPickedUpByEntity {
        item_key: &'a str,
        item_count: i32,
        entity_key: &'a str,
    },
    ThrownItemPickedUpByPlayer {
        item_key: &'a str,
        item_count: i32,
        entity_key: &'a str,
    },
    EnterBlock {
        block_key: &'a str,
        state_props: &'a [(String, String)],
    },
    PlacedBlock {
        block_key: &'a str,
        item_key: &'a str,
        item_count: i32,
        dimension_key: &'a str,
    },
    ItemUsedOnBlock {
        item_key: &'a str,
        item_count: i32,
        dimension_key: &'a str,
    },
    DefaultBlockUse {
        dimension_key: &'a str,
    },
    AnyBlockUse {
        dimension_key: &'a str,
    },
    SlideDownBlock {
        block_key: &'a str,
    },
    BeeNestDestroyed {
        block_key: &'a str,
        item_key: &'a str,
        item_count: i32,
        num_bees: i32,
    },
    TargetHit {
        signal_strength: i32,
        projectile_key: &'a str,
    },
    AllayDropItemOnBlock {
        item_key: &'a str,
        item_count: i32,
        dimension_key: &'a str,
    },
    PlayerKilledEntity {
        entity_key: &'a str,
    },
    EntityKilledPlayer {
        entity_key: &'a str,
    },
    PlayerHurtEntity {
        entity_key: &'a str,
    },
    EntityHurtPlayer,
    KilledByArrow,
    SpearMobs {
        projectile_key: &'a str,
        target_key: &'a str,
        thrown: bool,
    },
    ShotCrossbow {
        item_key: &'a str,
        item_count: i32,
    },
    ChanneledLightning {
        victim_keys: &'a [String],
    },
    KillMobNearSculkCatalyst {
        entity_key: &'a str,
    },
    FallAfterExplosion {
        distance: f64,
    },
    TameAnimal {
        entity_key: &'a str,
    },
    BredAnimals {
        parent_key: &'a str,
        partner_key: &'a str,
        child_key: Option<&'a str>,
    },
    VillagerTrade {
        villager_key: &'a str,
        item_key: &'a str,
        item_count: i32,
    },
    CuredZombieVillager {
        villager_key: &'a str,
        zombie_key: &'a str,
    },
    SummonedEntity {
        entity_key: &'a str,
    },
    PlayerInteractedWithEntity {
        item_key: &'a str,
        item_count: i32,
        entity_key: &'a str,
    },
    PlayerShearedEquipment {
        item_key: &'a str,
        item_count: i32,
        entity_key: &'a str,
    },
    HeroOfTheVillage {
        dimension_key: &'a str,
    },
    VoluntaryExile {
        dimension_key: &'a str,
    },
    EffectsChanged,
    Levitation {
        distance: f64,
        duration: i32,
    },
    EnchantedItem {
        item_key: &'a str,
        item_count: i32,
        levels: i32,
    },
    BrewedPotion {
        potion_key: &'a str,
    },
    NetherTravel {
        distance: f64,
    },
    FallFromHeight {
        distance: f64,
    },
    RideEntityInLava {
        distance: f64,
    },
    UsedEnderEye {
        distance: f64,
    },
    LightningStrike {
        lightning_key: &'a str,
        bystander_key: Option<&'a str>,
    },
    ConstructBeacon {
        level: i32,
    },
    PlayerGeneratesContainerLoot {
        loot_table: &'a str,
    },
    UsedTotem {
        item_key: &'a str,
        item_count: i32,
    },
}

impl TriggerContext<'_> {
    /// The trigger ID string this context corresponds to.
    pub fn trigger_id(&self) -> &'static str {
        match self {
            Self::Tick => "minecraft:tick",
            Self::Location { .. } => "minecraft:location",
            Self::SleptInBed { .. } => "minecraft:slept_in_bed",
            Self::ChangedDimension { .. } => "minecraft:changed_dimension",
            Self::StartedRiding => "minecraft:started_riding",
            Self::AvoidVibration => "minecraft:avoid_vibration",
            Self::InventoryChanged { .. } => "minecraft:inventory_changed",
            Self::RecipeUnlocked { .. } => "minecraft:recipe_unlocked",
            Self::ConsumeItem { .. } => "minecraft:consume_item",
            Self::ItemDurabilityChanged { .. } => "minecraft:item_durability_changed",
            Self::UsingItem { .. } => "minecraft:using_item",
            Self::FilledBucket { .. } => "minecraft:filled_bucket",
            Self::FishingRodHooked { .. } => "minecraft:fishing_rod_hooked",
            Self::RecipeCrafted { .. } => "minecraft:recipe_crafted",
            Self::CrafterRecipeCrafted { .. } => "minecraft:crafter_recipe_crafted",
            Self::ThrownItemPickedUpByEntity { .. } => "minecraft:thrown_item_picked_up_by_entity",
            Self::ThrownItemPickedUpByPlayer { .. } => "minecraft:thrown_item_picked_up_by_player",
            Self::EnterBlock { .. } => "minecraft:enter_block",
            Self::PlacedBlock { .. } => "minecraft:placed_block",
            Self::ItemUsedOnBlock { .. } => "minecraft:item_used_on_block",
            Self::DefaultBlockUse { .. } => "minecraft:default_block_use",
            Self::AnyBlockUse { .. } => "minecraft:any_block_use",
            Self::SlideDownBlock { .. } => "minecraft:slide_down_block",
            Self::BeeNestDestroyed { .. } => "minecraft:bee_nest_destroyed",
            Self::TargetHit { .. } => "minecraft:target_hit",
            Self::AllayDropItemOnBlock { .. } => "minecraft:allay_drop_item_on_block",
            Self::PlayerKilledEntity { .. } => "minecraft:player_killed_entity",
            Self::EntityKilledPlayer { .. } => "minecraft:entity_killed_player",
            Self::PlayerHurtEntity { .. } => "minecraft:player_hurt_entity",
            Self::EntityHurtPlayer => "minecraft:entity_hurt_player",
            Self::KilledByArrow => "minecraft:killed_by_arrow",
            Self::SpearMobs { .. } => "minecraft:spear_mobs",
            Self::ShotCrossbow { .. } => "minecraft:shot_crossbow",
            Self::ChanneledLightning { .. } => "minecraft:channeled_lightning",
            Self::KillMobNearSculkCatalyst { .. } => "minecraft:kill_mob_near_sculk_catalyst",
            Self::FallAfterExplosion { .. } => "minecraft:fall_after_explosion",
            Self::TameAnimal { .. } => "minecraft:tame_animal",
            Self::BredAnimals { .. } => "minecraft:bred_animals",
            Self::VillagerTrade { .. } => "minecraft:villager_trade",
            Self::CuredZombieVillager { .. } => "minecraft:cured_zombie_villager",
            Self::SummonedEntity { .. } => "minecraft:summoned_entity",
            Self::PlayerInteractedWithEntity { .. } => "minecraft:player_interacted_with_entity",
            Self::PlayerShearedEquipment { .. } => "minecraft:player_sheared_equipment",
            Self::HeroOfTheVillage { .. } => "minecraft:hero_of_the_village",
            Self::VoluntaryExile { .. } => "minecraft:voluntary_exile",
            Self::EffectsChanged => "minecraft:effects_changed",
            Self::Levitation { .. } => "minecraft:levitation",
            Self::EnchantedItem { .. } => "minecraft:enchanted_item",
            Self::BrewedPotion { .. } => "minecraft:brewed_potion",
            Self::NetherTravel { .. } => "minecraft:nether_travel",
            Self::FallFromHeight { .. } => "minecraft:fall_from_height",
            Self::RideEntityInLava { .. } => "minecraft:ride_entity_in_lava",
            Self::UsedEnderEye { .. } => "minecraft:used_ender_eye",
            Self::LightningStrike { .. } => "minecraft:lightning_strike",
            Self::ConstructBeacon { .. } => "minecraft:construct_beacon",
            Self::PlayerGeneratesContainerLoot { .. } => {
                "minecraft:player_generates_container_loot"
            }
            Self::UsedTotem { .. } => "minecraft:used_totem",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Condition matching
// ─────────────────────────────────────────────────────────────────────────────

/// Returns `true` when `condition` passes for the given game-state `ctx`.
///
/// Returns `false` for `TriggerCondition::Impossible` and `TriggerCondition::Unknown`.
pub fn condition_matches(condition: &TriggerCondition, ctx: &TriggerContext<'_>) -> bool {
    match (condition, ctx) {
        (TriggerCondition::Impossible, _) | (TriggerCondition::Unknown, _) => false,

        (TriggerCondition::Tick, TriggerContext::Tick) => true,

        (TriggerCondition::Location { location }, TriggerContext::Location { dimension_key }) => {
            location
                .as_ref()
                .map_or(true, |l| l.matches_dimension(dimension_key))
        }

        (TriggerCondition::SleptInBed { location }, TriggerContext::SleptInBed { dimension_key }) => {
            location
                .as_ref()
                .map_or(true, |l| l.matches_dimension(dimension_key))
        }

        (
            TriggerCondition::ChangedDimension { from, to },
            TriggerContext::ChangedDimension { from: ctx_from, to: ctx_to },
        ) => {
            from.as_ref().map_or(true, |f| f == ctx_from)
                && to.as_ref().map_or(true, |t| t == *ctx_to)
        }

        (TriggerCondition::StartedRiding, TriggerContext::StartedRiding) => true,
        (TriggerCondition::AvoidVibration, TriggerContext::AvoidVibration) => true,

        (
            TriggerCondition::InventoryChanged { slots, items },
            TriggerContext::InventoryChanged {
                changed_item_key,
                changed_item_count,
                slots_occupied,
                slots_full,
                slots_empty,
                all_items,
            },
        ) => {
            if !slots.matches(*slots_occupied, *slots_full, *slots_empty) {
                return false;
            }
            if items.is_empty() {
                return true;
            }
            if items.len() == 1 {
                return items[0].matches_item_key(changed_item_key, *changed_item_count);
            }
            // Multi-item: all predicates must be satisfied by at least one slot
            let mut remaining: Vec<&ItemPredicate> = items.iter().collect();
            for (key, count) in *all_items {
                remaining.retain(|p| !p.matches_item_key(key, *count));
                if remaining.is_empty() {
                    return true;
                }
            }
            remaining.is_empty()
        }

        (
            TriggerCondition::RecipeUnlocked { recipe },
            TriggerContext::RecipeUnlocked { recipe_id },
        ) => recipe.as_ref().map_or(true, |r| r == recipe_id),

        (
            TriggerCondition::ConsumeItem { item },
            TriggerContext::ConsumeItem { item_key, item_count },
        ) => item
            .as_ref()
            .map_or(true, |p| p.matches_item_key(item_key, *item_count)),

        (
            TriggerCondition::ItemDurabilityChanged { item, delta, durability },
            TriggerContext::ItemDurabilityChanged {
                item_key,
                item_count,
                delta: ctx_delta,
                durability: ctx_dur,
            },
        ) => {
            item.as_ref()
                .map_or(true, |p| p.matches_item_key(item_key, *item_count))
                && delta.matches(*ctx_delta)
                && durability.matches(*ctx_dur)
        }

        (
            TriggerCondition::UsingItem { item },
            TriggerContext::UsingItem { item_key, item_count },
        ) => item
            .as_ref()
            .map_or(true, |p| p.matches_item_key(item_key, *item_count)),

        (
            TriggerCondition::FilledBucket { item },
            TriggerContext::FilledBucket { item_key, item_count },
        ) => item
            .as_ref()
            .map_or(true, |p| p.matches_item_key(item_key, *item_count)),

        (
            TriggerCondition::FishingRodHooked { rod, entity, item },
            TriggerContext::FishingRodHooked {
                rod_key,
                rod_count,
                entity_key,
                item_key,
                item_count,
            },
        ) => {
            rod.as_ref()
                .map_or(true, |p| p.matches_item_key(rod_key, *rod_count))
                && entity.as_ref().map_or(true, |p| {
                    entity_key.map_or(false, |k| p.matches_entity_key(k))
                })
                && item.as_ref().map_or(true, |p| {
                    item_key.map_or(false, |k| p.matches_item_key(k, *item_count))
                })
        }

        (
            TriggerCondition::RecipeCrafted { recipe_id, ingredients },
            TriggerContext::RecipeCrafted {
                recipe_id: ctx_id,
                ingredient_keys,
            },
        ) => {
            recipe_id.as_ref().map_or(true, |r| r == ctx_id)
                && ingredients_match(ingredients, ingredient_keys)
        }

        (
            TriggerCondition::CrafterRecipeCrafted { recipe_id, ingredients },
            TriggerContext::CrafterRecipeCrafted {
                recipe_id: ctx_id,
                ingredient_keys,
            },
        ) => {
            recipe_id.as_ref().map_or(true, |r| r == ctx_id)
                && ingredients_match(ingredients, ingredient_keys)
        }

        (
            TriggerCondition::ThrownItemPickedUpByEntity { item, entity },
            TriggerContext::ThrownItemPickedUpByEntity {
                item_key,
                item_count,
                entity_key,
            },
        ) => {
            item.as_ref()
                .map_or(true, |p| p.matches_item_key(item_key, *item_count))
                && entity
                    .as_ref()
                    .map_or(true, |p| p.matches_entity_key(entity_key))
        }

        (
            TriggerCondition::ThrownItemPickedUpByPlayer { item, entity },
            TriggerContext::ThrownItemPickedUpByPlayer {
                item_key,
                item_count,
                entity_key,
            },
        ) => {
            item.as_ref()
                .map_or(true, |p| p.matches_item_key(item_key, *item_count))
                && entity
                    .as_ref()
                    .map_or(true, |p| p.matches_entity_key(entity_key))
        }

        (
            TriggerCondition::EnterBlock { block, state },
            TriggerContext::EnterBlock {
                block_key,
                state_props,
            },
        ) => {
            block.as_ref().map_or(true, |b| b == block_key)
                && state
                    .iter()
                    .all(|(k, v)| state_props.iter().any(|(pk, pv)| pk == k && pv == v))
        }

        (
            TriggerCondition::PlacedBlock { block, item, location },
            TriggerContext::PlacedBlock {
                block_key,
                item_key,
                item_count,
                dimension_key,
            },
        ) => {
            block.as_ref().map_or(true, |b| b == block_key)
                && item
                    .as_ref()
                    .map_or(true, |p| p.matches_item_key(item_key, *item_count))
                && location
                    .as_ref()
                    .map_or(true, |l| l.matches_dimension(dimension_key))
        }

        (
            TriggerCondition::ItemUsedOnBlock { location, item },
            TriggerContext::ItemUsedOnBlock {
                item_key,
                item_count,
                dimension_key,
            },
        ) => {
            item.as_ref()
                .map_or(true, |p| p.matches_item_key(item_key, *item_count))
                && location
                    .as_ref()
                    .map_or(true, |l| l.matches_dimension(dimension_key))
        }

        (
            TriggerCondition::DefaultBlockUse { location },
            TriggerContext::DefaultBlockUse { dimension_key },
        ) => location
            .as_ref()
            .map_or(true, |l| l.matches_dimension(dimension_key)),

        (
            TriggerCondition::AnyBlockUse { location },
            TriggerContext::AnyBlockUse { dimension_key },
        ) => location
            .as_ref()
            .map_or(true, |l| l.matches_dimension(dimension_key)),

        (
            TriggerCondition::SlideDownBlock { block },
            TriggerContext::SlideDownBlock { block_key },
        ) => block.as_ref().map_or(true, |b| b == block_key),

        (
            TriggerCondition::BeeNestDestroyed { block, item, num_bees },
            TriggerContext::BeeNestDestroyed {
                block_key,
                item_key,
                item_count,
                num_bees: ctx_bees,
            },
        ) => {
            block.as_ref().map_or(true, |b| b == block_key)
                && item
                    .as_ref()
                    .map_or(true, |p| p.matches_item_key(item_key, *item_count))
                && num_bees.matches(*ctx_bees)
        }

        (
            TriggerCondition::TargetHit { signal_strength, projectile },
            TriggerContext::TargetHit {
                signal_strength: ctx_sig,
                projectile_key,
            },
        ) => {
            signal_strength.matches(*ctx_sig)
                && projectile
                    .as_ref()
                    .map_or(true, |p| p.matches_entity_key(projectile_key))
        }

        (
            TriggerCondition::AllayDropItemOnBlock { location, item },
            TriggerContext::AllayDropItemOnBlock {
                item_key,
                item_count,
                dimension_key,
            },
        ) => {
            item.as_ref()
                .map_or(true, |p| p.matches_item_key(item_key, *item_count))
                && location
                    .as_ref()
                    .map_or(true, |l| l.matches_dimension(dimension_key))
        }

        (
            TriggerCondition::PlayerKilledEntity { entity, killing_blow },
            TriggerContext::PlayerKilledEntity { entity_key },
        ) => {
            entity
                .as_ref()
                .map_or(true, |p| p.matches_entity_key(entity_key))
                && killing_blow.as_ref().map_or(true, |p| p.matches())
        }

        (
            TriggerCondition::EntityKilledPlayer { entity, killing_blow },
            TriggerContext::EntityKilledPlayer { entity_key },
        ) => {
            entity
                .as_ref()
                .map_or(true, |p| p.matches_entity_key(entity_key))
                && killing_blow.as_ref().map_or(true, |p| p.matches())
        }

        (
            TriggerCondition::PlayerHurtEntity { entity, damage },
            TriggerContext::PlayerHurtEntity { entity_key },
        ) => {
            entity
                .as_ref()
                .map_or(true, |p| p.matches_entity_key(entity_key))
                && damage.as_ref().map_or(true, |p| p.matches())
        }

        (TriggerCondition::EntityHurtPlayer { .. }, TriggerContext::EntityHurtPlayer) => true,

        (TriggerCondition::KilledByArrow { .. }, TriggerContext::KilledByArrow) => true,

        (
            TriggerCondition::SpearMobs { projectile, target, thrown: cond_thrown },
            TriggerContext::SpearMobs { projectile_key, target_key, thrown: ctx_thrown },
        ) => {
            projectile
                .as_ref()
                .map_or(true, |p| p.matches_entity_key(projectile_key))
                && target
                    .as_ref()
                    .map_or(true, |p| p.matches_entity_key(target_key))
                && cond_thrown == ctx_thrown
        }

        (
            TriggerCondition::ShotCrossbow { item },
            TriggerContext::ShotCrossbow { item_key, item_count },
        ) => item
            .as_ref()
            .map_or(true, |p| p.matches_item_key(item_key, *item_count)),

        (
            TriggerCondition::ChanneledLightning { victims },
            TriggerContext::ChanneledLightning { victim_keys },
        ) => victims
            .iter()
            .all(|p| victim_keys.iter().any(|k| p.matches_entity_key(k))),

        (
            TriggerCondition::KillMobNearSculkCatalyst { entity, killing_blow },
            TriggerContext::KillMobNearSculkCatalyst { entity_key },
        ) => {
            entity
                .as_ref()
                .map_or(true, |p| p.matches_entity_key(entity_key))
                && killing_blow.as_ref().map_or(true, |p| p.matches())
        }

        (
            TriggerCondition::FallAfterExplosion { distance },
            TriggerContext::FallAfterExplosion { distance: ctx_d },
        ) => distance.as_ref().map_or(true, |r| r.matches(*ctx_d)),

        (
            TriggerCondition::TameAnimal { entity },
            TriggerContext::TameAnimal { entity_key },
        ) => entity
            .as_ref()
            .map_or(true, |p| p.matches_entity_key(entity_key)),

        (
            TriggerCondition::BredAnimals { parent, partner, child },
            TriggerContext::BredAnimals { parent_key, partner_key, child_key },
        ) => {
            parent
                .as_ref()
                .map_or(true, |p| p.matches_entity_key(parent_key))
                && partner
                    .as_ref()
                    .map_or(true, |p| p.matches_entity_key(partner_key))
                && child.as_ref().map_or(true, |p| {
                    child_key.map_or(false, |k| p.matches_entity_key(k))
                })
        }

        (
            TriggerCondition::VillagerTrade { villager, item },
            TriggerContext::VillagerTrade {
                villager_key,
                item_key,
                item_count,
            },
        ) => {
            villager
                .as_ref()
                .map_or(true, |p| p.matches_entity_key(villager_key))
                && item
                    .as_ref()
                    .map_or(true, |p| p.matches_item_key(item_key, *item_count))
        }

        (
            TriggerCondition::CuredZombieVillager { villager, zombie },
            TriggerContext::CuredZombieVillager { villager_key, zombie_key },
        ) => {
            villager
                .as_ref()
                .map_or(true, |p| p.matches_entity_key(villager_key))
                && zombie
                    .as_ref()
                    .map_or(true, |p| p.matches_entity_key(zombie_key))
        }

        (
            TriggerCondition::SummonedEntity { entity },
            TriggerContext::SummonedEntity { entity_key },
        ) => entity
            .as_ref()
            .map_or(true, |p| p.matches_entity_key(entity_key)),

        (
            TriggerCondition::PlayerInteractedWithEntity { item, entity },
            TriggerContext::PlayerInteractedWithEntity {
                item_key,
                item_count,
                entity_key,
            },
        ) => {
            item.as_ref()
                .map_or(true, |p| p.matches_item_key(item_key, *item_count))
                && entity
                    .as_ref()
                    .map_or(true, |p| p.matches_entity_key(entity_key))
        }

        (
            TriggerCondition::PlayerShearedEquipment { item, entity },
            TriggerContext::PlayerShearedEquipment {
                item_key,
                item_count,
                entity_key,
            },
        ) => {
            item.as_ref()
                .map_or(true, |p| p.matches_item_key(item_key, *item_count))
                && entity
                    .as_ref()
                    .map_or(true, |p| p.matches_entity_key(entity_key))
        }

        (
            TriggerCondition::HeroOfTheVillage { location },
            TriggerContext::HeroOfTheVillage { dimension_key },
        ) => location
            .as_ref()
            .map_or(true, |l| l.matches_dimension(dimension_key)),

        (
            TriggerCondition::VoluntaryExile { location },
            TriggerContext::VoluntaryExile { dimension_key },
        ) => location
            .as_ref()
            .map_or(true, |l| l.matches_dimension(dimension_key)),

        (TriggerCondition::EffectsChanged, TriggerContext::EffectsChanged) => true,

        (
            TriggerCondition::Levitation { distance, duration },
            TriggerContext::Levitation {
                distance: ctx_d,
                duration: ctx_dur,
            },
        ) => {
            distance.as_ref().map_or(true, |r| r.matches(*ctx_d)) && duration.matches(*ctx_dur)
        }

        (
            TriggerCondition::EnchantedItem { item, levels },
            TriggerContext::EnchantedItem {
                item_key,
                item_count,
                levels: ctx_lvl,
            },
        ) => {
            item.as_ref()
                .map_or(true, |p| p.matches_item_key(item_key, *item_count))
                && levels.matches(*ctx_lvl)
        }

        (TriggerCondition::BrewedPotion { potion }, TriggerContext::BrewedPotion { potion_key }) => {
            potion.as_ref().map_or(true, |p| p == potion_key)
        }

        (TriggerCondition::NetherTravel { distance }, TriggerContext::NetherTravel { distance: ctx_d }) => {
            distance.as_ref().map_or(true, |r| r.matches(*ctx_d))
        }

        (
            TriggerCondition::FallFromHeight { distance },
            TriggerContext::FallFromHeight { distance: ctx_d },
        ) => distance.as_ref().map_or(true, |r| r.matches(*ctx_d)),

        (
            TriggerCondition::RideEntityInLava { distance },
            TriggerContext::RideEntityInLava { distance: ctx_d },
        ) => distance.as_ref().map_or(true, |r| r.matches(*ctx_d)),

        (
            TriggerCondition::UsedEnderEye { distance },
            TriggerContext::UsedEnderEye { distance: ctx_d },
        ) => distance.as_ref().map_or(true, |r| r.matches(*ctx_d)),

        (
            TriggerCondition::LightningStrike { lightning, bystander },
            TriggerContext::LightningStrike { lightning_key, bystander_key },
        ) => {
            lightning
                .as_ref()
                .map_or(true, |p| p.matches_entity_key(lightning_key))
                && bystander.as_ref().map_or(true, |p| {
                    bystander_key.map_or(false, |k| p.matches_entity_key(k))
                })
        }

        (
            TriggerCondition::ConstructBeacon { level },
            TriggerContext::ConstructBeacon { level: ctx_lvl },
        ) => level.matches(*ctx_lvl),

        (
            TriggerCondition::PlayerGeneratesContainerLoot { loot_table },
            TriggerContext::PlayerGeneratesContainerLoot { loot_table: ctx_lt },
        ) => loot_table.as_ref().map_or(true, |lt| lt == ctx_lt),

        (
            TriggerCondition::UsedTotem { item },
            TriggerContext::UsedTotem { item_key, item_count },
        ) => item
            .as_ref()
            .map_or(true, |p| p.matches_item_key(item_key, *item_count)),

        // Mismatched condition/context pairs never fire
        _ => false,
    }
}

fn ingredients_match(predicates: &[ItemPredicate], ingredients: &[(String, i32)]) -> bool {
    if predicates.is_empty() {
        return true;
    }
    let mut remaining: Vec<&ItemPredicate> = predicates.iter().collect();
    for (key, count) in ingredients {
        remaining.retain(|p| !p.matches_item_key(key, *count));
        if remaining.is_empty() {
            return true;
        }
    }
    remaining.is_empty()
}

// ─────────────────────────────────────────────────────────────────────────────
// TriggerRegistry
// ─────────────────────────────────────────────────────────────────────────────

/// Singleton parsed during server startup. Maps advancement_id + criterion_name
/// to a parsed `TriggerCondition`.
pub struct TriggerRegistry {
    /// advancement_id → criterion_name → flat index into `conditions`.
    index: FxHashMap<&'static str, FxHashMap<&'static str, u32>>,
    /// Flat storage for all parsed conditions (indexed by the values in `index`).
    conditions: Vec<TriggerCondition>,
}

impl TriggerRegistry {
    /// Parse all criterion conditions from the compiled static advancement data.
    #[must_use]
    pub fn new() -> Self {
        let mut index: FxHashMap<&'static str, FxHashMap<&'static str, u32>> =
            FxHashMap::default();
        let mut conditions: Vec<TriggerCondition> = Vec::new();

        for def in VANILLA_ADVANCEMENTS {
            let crit_map = index.entry(def.id).or_default();
            for crit in def.criteria {
                let condition = parse_condition(crit.trigger, crit.conditions);
                let idx = conditions.len() as u32;
                conditions.push(condition);
                crit_map.insert(crit.name, idx);
            }
        }

        Self { index, conditions }
    }

    /// Get the `TriggerCondition` for a specific criterion.
    pub fn get_condition(
        &self,
        advancement_id: &str,
        criterion: &str,
    ) -> Option<&TriggerCondition> {
        let idx = *self.index.get(advancement_id)?.get(criterion)?;
        self.conditions.get(idx as usize)
    }

    /// Get the flat condition index for a specific criterion.
    pub fn get_condition_idx(&self, advancement_id: &str, criterion: &str) -> Option<u32> {
        Some(*self.index.get(advancement_id)?.get(criterion)?)
    }

    /// Get a condition by its flat index.
    pub fn condition_by_idx(&self, idx: u32) -> Option<&TriggerCondition> {
        self.conditions.get(idx as usize)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Condition parsing (JSON → TriggerCondition)
// ─────────────────────────────────────────────────────────────────────────────

fn parse_condition(trigger: &str, json: &[u8]) -> TriggerCondition {
    match trigger {
        "minecraft:impossible" => TriggerCondition::Impossible,
        "minecraft:tick" => TriggerCondition::Tick,
        "minecraft:location" => parse_location(json),
        "minecraft:slept_in_bed" => parse_slept_in_bed(json),
        "minecraft:changed_dimension" => parse_changed_dimension(json),
        "minecraft:started_riding" => TriggerCondition::StartedRiding,
        "minecraft:avoid_vibration" => TriggerCondition::AvoidVibration,
        "minecraft:inventory_changed" => parse_inventory_changed(json),
        "minecraft:recipe_unlocked" => parse_recipe_unlocked(json),
        "minecraft:consume_item" => parse_consume_item(json),
        "minecraft:item_durability_changed" => parse_item_durability_changed(json),
        "minecraft:using_item" => parse_using_item(json),
        "minecraft:filled_bucket" => parse_filled_bucket(json),
        "minecraft:fishing_rod_hooked" => parse_fishing_rod_hooked(json),
        "minecraft:recipe_crafted" => parse_recipe_crafted(json),
        "minecraft:crafter_recipe_crafted" => parse_crafter_recipe_crafted(json),
        "minecraft:thrown_item_picked_up_by_entity" => parse_thrown_item_by_entity(json),
        "minecraft:thrown_item_picked_up_by_player" => parse_thrown_item_by_player(json),
        "minecraft:enter_block" => parse_enter_block(json),
        "minecraft:placed_block" => parse_placed_block(json),
        "minecraft:item_used_on_block" => parse_item_used_on_block(json),
        "minecraft:default_block_use" => parse_default_block_use(json),
        "minecraft:any_block_use" => parse_any_block_use(json),
        "minecraft:slide_down_block" => parse_slide_down_block(json),
        "minecraft:bee_nest_destroyed" => parse_bee_nest_destroyed(json),
        "minecraft:target_hit" => parse_target_hit(json),
        "minecraft:allay_drop_item_on_block" => parse_allay_drop(json),
        "minecraft:player_killed_entity" => parse_player_killed_entity(json),
        "minecraft:entity_killed_player" => parse_entity_killed_player(json),
        "minecraft:player_hurt_entity" => parse_player_hurt_entity(json),
        "minecraft:entity_hurt_player" => parse_entity_hurt_player(json),
        "minecraft:killed_by_arrow" => TriggerCondition::KilledByArrow { unique_entity_types: None },
        "minecraft:spear_mobs" => parse_spear_mobs(json),
        "minecraft:shot_crossbow" => parse_shot_crossbow(json),
        "minecraft:channeled_lightning" => parse_channeled_lightning(json),
        "minecraft:kill_mob_near_sculk_catalyst" => parse_kill_near_sculk(json),
        "minecraft:fall_after_explosion" => parse_fall_after_explosion(json),
        "minecraft:tame_animal" => parse_tame_animal(json),
        "minecraft:bred_animals" => parse_bred_animals(json),
        "minecraft:villager_trade" => parse_villager_trade(json),
        "minecraft:cured_zombie_villager" => parse_cured_zombie(json),
        "minecraft:summoned_entity" => parse_summoned_entity(json),
        "minecraft:player_interacted_with_entity" => parse_interacted_with_entity(json),
        "minecraft:player_sheared_equipment" => parse_sheared_equipment(json),
        "minecraft:hero_of_the_village" => parse_hero_of_village(json),
        "minecraft:voluntary_exile" => parse_voluntary_exile(json),
        "minecraft:effects_changed" => TriggerCondition::EffectsChanged,
        "minecraft:levitation" => parse_levitation(json),
        "minecraft:enchanted_item" => parse_enchanted_item(json),
        "minecraft:brewed_potion" => parse_brewed_potion(json),
        "minecraft:nether_travel" => parse_nether_travel(json),
        "minecraft:fall_from_height" => parse_fall_from_height(json),
        "minecraft:ride_entity_in_lava" => parse_ride_entity_in_lava(json),
        "minecraft:used_ender_eye" => parse_used_ender_eye(json),
        "minecraft:lightning_strike" => parse_lightning_strike(json),
        "minecraft:construct_beacon" => parse_construct_beacon(json),
        "minecraft:player_generates_container_loot" => parse_container_loot(json),
        "minecraft:used_totem" => parse_used_totem(json),
        _ => TriggerCondition::Unknown,
    }
}

// ── Serde helper structs ──────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
struct LocationConds {
    location: Option<LocationPredicate>,
}

#[derive(Deserialize, Default)]
struct DimConds {
    from: Option<String>,
    to: Option<String>,
}

#[derive(Deserialize, Default)]
struct InvChangedConds {
    #[serde(default)]
    slots: SlotsPredicate,
    #[serde(default)]
    items: Vec<ItemPredicate>,
}

#[derive(Deserialize, Default)]
struct RecipeConds {
    recipe: Option<String>,
}

#[derive(Deserialize, Default)]
struct ItemConds {
    item: Option<ItemPredicate>,
}

#[derive(Deserialize, Default)]
struct DurConds {
    item: Option<ItemPredicate>,
    #[serde(default)]
    delta: MinMaxInt,
    #[serde(default)]
    durability: MinMaxInt,
}

#[derive(Deserialize, Default)]
struct FishingConds {
    rod: Option<ItemPredicate>,
    entity: Option<EntityPredicate>,
    item: Option<ItemPredicate>,
}

#[derive(Deserialize, Default)]
struct RecipeCraftedConds {
    recipe_id: Option<String>,
    #[serde(default)]
    ingredients: Vec<ItemPredicate>,
}

#[derive(Deserialize, Default)]
struct ThrownItemEntityConds {
    item: Option<ItemPredicate>,
    entity: Option<EntityPredicate>,
}

#[derive(Deserialize, Default)]
struct EnterBlockConds {
    block: Option<String>,
    #[serde(default)]
    state: std::collections::BTreeMap<String, String>,
}

#[derive(Deserialize, Default)]
struct PlacedBlockConds {
    block: Option<String>,
    item: Option<ItemPredicate>,
    location: Option<LocationPredicate>,
}

#[derive(Deserialize, Default)]
struct ItemOnBlockConds {
    location: Option<LocationPredicate>,
    item: Option<ItemPredicate>,
}

#[derive(Deserialize, Default)]
struct BlockConds {
    block: Option<String>,
}

#[derive(Deserialize, Default)]
struct BeeNestConds {
    block: Option<String>,
    item: Option<ItemPredicate>,
    #[serde(default)]
    num_bees_inside: MinMaxInt,
}

#[derive(Deserialize, Default)]
struct TargetHitConds {
    #[serde(default)]
    signal_strength: MinMaxInt,
    projectile: Option<EntityPredicate>,
}

#[derive(Deserialize, Default)]
struct KilledConds {
    entity: Option<EntityPredicate>,
    killing_blow: Option<DamageSourcePredicate>,
}

#[derive(Deserialize, Default)]
struct HurtEntityConds {
    entity: Option<EntityPredicate>,
    damage: Option<DamageSourcePredicate>,
}

#[derive(Deserialize, Default)]
struct HurtPlayerConds {
    damage: Option<DamageSourcePredicate>,
}

#[derive(Deserialize, Default)]
struct SpearConds {
    projectile: Option<EntityPredicate>,
    target: Option<EntityPredicate>,
    #[serde(default)]
    thrown: bool,
}

#[derive(Deserialize, Default)]
struct ChanneledConds {
    #[serde(default)]
    victims: Vec<EntityPredicate>,
}

#[derive(Deserialize, Default)]
struct EntityConds {
    entity: Option<EntityPredicate>,
}

#[derive(Deserialize, Default)]
struct BredConds {
    parent: Option<EntityPredicate>,
    partner: Option<EntityPredicate>,
    child: Option<EntityPredicate>,
}

#[derive(Deserialize, Default)]
struct TradeConds {
    villager: Option<EntityPredicate>,
    item: Option<ItemPredicate>,
}

#[derive(Deserialize, Default)]
struct CuredConds {
    villager: Option<EntityPredicate>,
    zombie: Option<EntityPredicate>,
}

#[derive(Deserialize, Default)]
struct EntityItemConds {
    item: Option<ItemPredicate>,
    entity: Option<EntityPredicate>,
}

#[derive(Deserialize, Default)]
struct LevitationConds {
    distance: Option<MinMaxFloat>,
    #[serde(default)]
    duration: MinMaxInt,
}

#[derive(Deserialize, Default)]
struct EnchantConds {
    item: Option<ItemPredicate>,
    #[serde(default)]
    levels: MinMaxInt,
}

#[derive(Deserialize, Default)]
struct PotionConds {
    potion: Option<String>,
}

#[derive(Deserialize, Default)]
struct DistanceConds {
    distance: Option<MinMaxFloat>,
}

#[derive(Deserialize, Default)]
struct LightningConds {
    lightning: Option<EntityPredicate>,
    bystander: Option<EntityPredicate>,
}

#[derive(Deserialize, Default)]
struct BeaconConds {
    #[serde(default)]
    level: MinMaxInt,
}

#[derive(Deserialize, Default)]
struct LootConds {
    loot_table: Option<String>,
}

// ── Parser functions ──────────────────────────────────────────────────────────

fn de<T: for<'de> Deserialize<'de> + Default>(json: &[u8]) -> T {
    serde_json::from_slice(json).unwrap_or_default()
}

fn parse_location(json: &[u8]) -> TriggerCondition {
    let c: LocationConds = de(json);
    TriggerCondition::Location { location: c.location }
}

fn parse_slept_in_bed(json: &[u8]) -> TriggerCondition {
    let c: LocationConds = de(json);
    TriggerCondition::SleptInBed { location: c.location }
}

fn parse_changed_dimension(json: &[u8]) -> TriggerCondition {
    let c: DimConds = de(json);
    TriggerCondition::ChangedDimension { from: c.from, to: c.to }
}

fn parse_inventory_changed(json: &[u8]) -> TriggerCondition {
    let c: InvChangedConds = de(json);
    TriggerCondition::InventoryChanged { slots: c.slots, items: c.items }
}

fn parse_recipe_unlocked(json: &[u8]) -> TriggerCondition {
    let c: RecipeConds = de(json);
    TriggerCondition::RecipeUnlocked { recipe: c.recipe }
}

fn parse_consume_item(json: &[u8]) -> TriggerCondition {
    let c: ItemConds = de(json);
    TriggerCondition::ConsumeItem { item: c.item }
}

fn parse_item_durability_changed(json: &[u8]) -> TriggerCondition {
    let c: DurConds = de(json);
    TriggerCondition::ItemDurabilityChanged { item: c.item, delta: c.delta, durability: c.durability }
}

fn parse_using_item(json: &[u8]) -> TriggerCondition {
    let c: ItemConds = de(json);
    TriggerCondition::UsingItem { item: c.item }
}

fn parse_filled_bucket(json: &[u8]) -> TriggerCondition {
    let c: ItemConds = de(json);
    TriggerCondition::FilledBucket { item: c.item }
}

fn parse_fishing_rod_hooked(json: &[u8]) -> TriggerCondition {
    let c: FishingConds = de(json);
    TriggerCondition::FishingRodHooked { rod: c.rod, entity: c.entity, item: c.item }
}

fn parse_recipe_crafted(json: &[u8]) -> TriggerCondition {
    let c: RecipeCraftedConds = de(json);
    TriggerCondition::RecipeCrafted { recipe_id: c.recipe_id, ingredients: c.ingredients }
}

fn parse_crafter_recipe_crafted(json: &[u8]) -> TriggerCondition {
    let c: RecipeCraftedConds = de(json);
    TriggerCondition::CrafterRecipeCrafted { recipe_id: c.recipe_id, ingredients: c.ingredients }
}

fn parse_thrown_item_by_entity(json: &[u8]) -> TriggerCondition {
    let c: ThrownItemEntityConds = de(json);
    TriggerCondition::ThrownItemPickedUpByEntity { item: c.item, entity: c.entity }
}

fn parse_thrown_item_by_player(json: &[u8]) -> TriggerCondition {
    let c: ThrownItemEntityConds = de(json);
    TriggerCondition::ThrownItemPickedUpByPlayer { item: c.item, entity: c.entity }
}

fn parse_enter_block(json: &[u8]) -> TriggerCondition {
    let c: EnterBlockConds = de(json);
    TriggerCondition::EnterBlock {
        block: c.block,
        state: c.state.into_iter().collect(),
    }
}

fn parse_placed_block(json: &[u8]) -> TriggerCondition {
    let c: PlacedBlockConds = de(json);
    TriggerCondition::PlacedBlock { block: c.block, item: c.item, location: c.location }
}

fn parse_item_used_on_block(json: &[u8]) -> TriggerCondition {
    let c: ItemOnBlockConds = de(json);
    TriggerCondition::ItemUsedOnBlock { location: c.location, item: c.item }
}

fn parse_default_block_use(json: &[u8]) -> TriggerCondition {
    let c: LocationConds = de(json);
    TriggerCondition::DefaultBlockUse { location: c.location }
}

fn parse_any_block_use(json: &[u8]) -> TriggerCondition {
    let c: LocationConds = de(json);
    TriggerCondition::AnyBlockUse { location: c.location }
}

fn parse_slide_down_block(json: &[u8]) -> TriggerCondition {
    let c: BlockConds = de(json);
    TriggerCondition::SlideDownBlock { block: c.block }
}

fn parse_bee_nest_destroyed(json: &[u8]) -> TriggerCondition {
    let c: BeeNestConds = de(json);
    TriggerCondition::BeeNestDestroyed { block: c.block, item: c.item, num_bees: c.num_bees_inside }
}

fn parse_target_hit(json: &[u8]) -> TriggerCondition {
    let c: TargetHitConds = de(json);
    TriggerCondition::TargetHit { signal_strength: c.signal_strength, projectile: c.projectile }
}

fn parse_allay_drop(json: &[u8]) -> TriggerCondition {
    let c: ItemOnBlockConds = de(json);
    TriggerCondition::AllayDropItemOnBlock { location: c.location, item: c.item }
}

fn parse_player_killed_entity(json: &[u8]) -> TriggerCondition {
    let c: KilledConds = de(json);
    TriggerCondition::PlayerKilledEntity { entity: c.entity, killing_blow: c.killing_blow }
}

fn parse_entity_killed_player(json: &[u8]) -> TriggerCondition {
    let c: KilledConds = de(json);
    TriggerCondition::EntityKilledPlayer { entity: c.entity, killing_blow: c.killing_blow }
}

fn parse_player_hurt_entity(json: &[u8]) -> TriggerCondition {
    let c: HurtEntityConds = de(json);
    TriggerCondition::PlayerHurtEntity { entity: c.entity, damage: c.damage }
}

fn parse_entity_hurt_player(json: &[u8]) -> TriggerCondition {
    let c: HurtPlayerConds = de(json);
    TriggerCondition::EntityHurtPlayer { damage: c.damage }
}

fn parse_spear_mobs(json: &[u8]) -> TriggerCondition {
    let c: SpearConds = de(json);
    TriggerCondition::SpearMobs { projectile: c.projectile, target: c.target, thrown: c.thrown }
}

fn parse_shot_crossbow(json: &[u8]) -> TriggerCondition {
    let c: ItemConds = de(json);
    TriggerCondition::ShotCrossbow { item: c.item }
}

fn parse_channeled_lightning(json: &[u8]) -> TriggerCondition {
    let c: ChanneledConds = de(json);
    TriggerCondition::ChanneledLightning { victims: c.victims }
}

fn parse_kill_near_sculk(json: &[u8]) -> TriggerCondition {
    let c: KilledConds = de(json);
    TriggerCondition::KillMobNearSculkCatalyst { entity: c.entity, killing_blow: c.killing_blow }
}

fn parse_fall_after_explosion(json: &[u8]) -> TriggerCondition {
    let c: DistanceConds = de(json);
    TriggerCondition::FallAfterExplosion { distance: c.distance }
}

fn parse_tame_animal(json: &[u8]) -> TriggerCondition {
    let c: EntityConds = de(json);
    TriggerCondition::TameAnimal { entity: c.entity }
}

fn parse_bred_animals(json: &[u8]) -> TriggerCondition {
    let c: BredConds = de(json);
    TriggerCondition::BredAnimals { parent: c.parent, partner: c.partner, child: c.child }
}

fn parse_villager_trade(json: &[u8]) -> TriggerCondition {
    let c: TradeConds = de(json);
    TriggerCondition::VillagerTrade { villager: c.villager, item: c.item }
}

fn parse_cured_zombie(json: &[u8]) -> TriggerCondition {
    let c: CuredConds = de(json);
    TriggerCondition::CuredZombieVillager { villager: c.villager, zombie: c.zombie }
}

fn parse_summoned_entity(json: &[u8]) -> TriggerCondition {
    let c: EntityConds = de(json);
    TriggerCondition::SummonedEntity { entity: c.entity }
}

fn parse_interacted_with_entity(json: &[u8]) -> TriggerCondition {
    let c: EntityItemConds = de(json);
    TriggerCondition::PlayerInteractedWithEntity { item: c.item, entity: c.entity }
}

fn parse_sheared_equipment(json: &[u8]) -> TriggerCondition {
    let c: EntityItemConds = de(json);
    TriggerCondition::PlayerShearedEquipment { item: c.item, entity: c.entity }
}

fn parse_hero_of_village(json: &[u8]) -> TriggerCondition {
    let c: LocationConds = de(json);
    TriggerCondition::HeroOfTheVillage { location: c.location }
}

fn parse_voluntary_exile(json: &[u8]) -> TriggerCondition {
    let c: LocationConds = de(json);
    TriggerCondition::VoluntaryExile { location: c.location }
}

fn parse_levitation(json: &[u8]) -> TriggerCondition {
    let c: LevitationConds = de(json);
    TriggerCondition::Levitation { distance: c.distance, duration: c.duration }
}

fn parse_enchanted_item(json: &[u8]) -> TriggerCondition {
    let c: EnchantConds = de(json);
    TriggerCondition::EnchantedItem { item: c.item, levels: c.levels }
}

fn parse_brewed_potion(json: &[u8]) -> TriggerCondition {
    let c: PotionConds = de(json);
    TriggerCondition::BrewedPotion { potion: c.potion }
}

fn parse_nether_travel(json: &[u8]) -> TriggerCondition {
    let c: DistanceConds = de(json);
    TriggerCondition::NetherTravel { distance: c.distance }
}

fn parse_fall_from_height(json: &[u8]) -> TriggerCondition {
    let c: DistanceConds = de(json);
    TriggerCondition::FallFromHeight { distance: c.distance }
}

fn parse_ride_entity_in_lava(json: &[u8]) -> TriggerCondition {
    let c: DistanceConds = de(json);
    TriggerCondition::RideEntityInLava { distance: c.distance }
}

fn parse_used_ender_eye(json: &[u8]) -> TriggerCondition {
    let c: DistanceConds = de(json);
    TriggerCondition::UsedEnderEye { distance: c.distance }
}

fn parse_lightning_strike(json: &[u8]) -> TriggerCondition {
    let c: LightningConds = de(json);
    TriggerCondition::LightningStrike { lightning: c.lightning, bystander: c.bystander }
}

fn parse_construct_beacon(json: &[u8]) -> TriggerCondition {
    let c: BeaconConds = de(json);
    TriggerCondition::ConstructBeacon { level: c.level }
}

fn parse_container_loot(json: &[u8]) -> TriggerCondition {
    let c: LootConds = de(json);
    TriggerCondition::PlayerGeneratesContainerLoot { loot_table: c.loot_table }
}

fn parse_used_totem(json: &[u8]) -> TriggerCondition {
    let c: ItemConds = de(json);
    TriggerCondition::UsedTotem { item: c.item }
}
