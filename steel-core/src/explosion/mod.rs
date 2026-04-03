//! Explosion system.
//!
//! Mirrors vanilla's `ServerExplosion` and associated types. An explosion:
//! 1. Casts 4096 rays outward to find blocks to destroy (`calculate_exploded_positions`)
//! 2. Damages and knocks back nearby entities (`hurt_entities`)
//! 3. Destroys blocks, optionally dropping loot (`interact_with_blocks`)
//! 4. Optionally spreads fire (`create_fire`)
//! 5. Sends `CExplode` to all players within 64 blocks
//!
//! Entry point: `Explosion::explode`.

use std::sync::Arc;

use glam::DVec3;

use crate::entity::damage::DamageSource;
use crate::entity::SharedEntity;
use crate::world::World;

pub mod calculator;
pub mod logic;

pub use calculator::{ExplosionDamageCalculator, SimpleExplosionDamageCalculator};

/// Controls how blocks interact with an explosion.
///
/// Mirrors vanilla's `Explosion.BlockInteraction` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockInteraction {
    /// No blocks are destroyed. Used when `tnt_explodes` gamerule is false.
    Keep,
    /// Blocks are destroyed and always drop all loot.
    Destroy,
    /// Blocks are destroyed; each block has a (1 / explosion_radius) chance to drop loot.
    DestroyWithDecay,
    /// Blocks are not destroyed but their "triggered by explosion" callbacks fire.
    /// Used by wind charges.
    TriggerBlock,
}

impl BlockInteraction {
    /// Returns true if blocks should be physically removed by this interaction.
    #[must_use]
    pub fn destroys_blocks(self) -> bool {
        matches!(self, Self::Destroy | Self::DestroyWithDecay)
    }
}

/// An explosion in the world.
///
/// Mirrors vanilla's `ServerExplosion`. Create via [`Explosion::new`] and trigger
/// via [`Explosion::explode`].
///
/// # Differences from vanilla
/// The damage calculator is a trait object rather than a class hierarchy, matching
/// Rust idioms while remaining extensible for modding.
pub struct Explosion {
    /// The world this explosion occurs in.
    pub world: Arc<World>,
    /// Center of the explosion.
    pub position: DVec3,
    /// Blast radius. Determines which blocks are affected and damage falloff.
    pub radius: f32,
    /// How the explosion interacts with blocks.
    pub block_interaction: BlockInteraction,
    /// Whether the explosion places fire in destroyed blocks (1/3 chance per block).
    pub fire: bool,
    /// The damage source used when hurting entities.
    pub damage_source: DamageSource,
    /// The entity that caused the explosion (e.g. PrimedTnt), if any.
    pub source_entity: Option<SharedEntity>,
    /// Damage and resistance calculator.
    pub damage_calculator: Box<dyn ExplosionDamageCalculator>,
}

impl Explosion {
    /// Creates a new explosion.
    #[must_use]
    pub fn new(
        world: Arc<World>,
        position: DVec3,
        radius: f32,
        fire: bool,
        block_interaction: BlockInteraction,
        damage_source: DamageSource,
        source_entity: Option<SharedEntity>,
        damage_calculator: Box<dyn ExplosionDamageCalculator>,
    ) -> Self {
        Self {
            world,
            position,
            radius,
            block_interaction,
            fire,
            damage_source,
            source_entity,
            damage_calculator,
        }
    }

    /// Executes the explosion: destroys blocks, hurts entities, broadcasts the packet.
    ///
    /// Returns the total number of blocks destroyed.
    pub fn explode(&self) -> i32 {
        logic::explode(self)
    }
}
