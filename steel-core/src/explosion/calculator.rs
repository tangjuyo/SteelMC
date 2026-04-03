//! Explosion damage calculators.
//!
//! Mirrors vanilla's `ExplosionDamageCalculator` class hierarchy.

use glam::DVec3;
use steel_registry::blocks::BlockRef;
use steel_registry::fluid::FluidRef;
use steel_utils::BlockStateId;

use crate::entity::SharedEntity;

/// Determines how an explosion calculates block resistance and entity damage.
///
/// Mirrors vanilla's `ExplosionDamageCalculator`. Implement this to customise
/// explosion behaviour (e.g. the portal-avoiding TNT calculator).
pub trait ExplosionDamageCalculator: Send + Sync {
    /// Returns the explosion resistance of a block, or `None` to skip it (treat as air).
    ///
    /// Mirrors vanilla's `ExplosionDamageCalculator.getBlockExplosionResistance`.
    fn block_explosion_resistance(
        &self,
        state: BlockStateId,
        block: BlockRef,
        fluid: Option<FluidRef>,
    ) -> Option<f32>;

    /// Returns whether a block should be destroyed by the explosion ray.
    ///
    /// Mirrors vanilla's `ExplosionDamageCalculator.shouldBlockExplode`.
    fn should_block_explode(&self, state: BlockStateId, block: BlockRef, power: f32) -> bool;

    /// Returns whether the entity should take damage from the explosion.
    ///
    /// Mirrors vanilla's `ExplosionDamageCalculator.shouldDamageEntity`.
    fn should_damage_entity(&self, entity: &SharedEntity) -> bool {
        true
    }

    /// Returns the knockback multiplier for this entity.
    ///
    /// Mirrors vanilla's `ExplosionDamageCalculator.getKnockbackMultiplier`.
    fn knockback_multiplier(&self, entity: &SharedEntity) -> f32 {
        let _ = entity;
        1.0
    }

    /// Returns the damage dealt to an entity.
    ///
    /// `distance_factor` is `sqrt(dist²) / (radius * 2)` clamped to [0, 1].
    /// `seen_percent` is the fraction of sample rays that reached the entity unobstructed.
    ///
    /// Mirrors vanilla's `ExplosionDamageCalculator.getEntityDamageAmount`.
    fn entity_damage_amount(
        &self,
        entity: &SharedEntity,
        explosion_center: DVec3,
        radius: f32,
        distance_factor: f64,
        seen_percent: f64,
    ) -> f32;
}

/// Default damage calculator — matches vanilla's `SimpleExplosionDamageCalculator`.
pub struct SimpleExplosionDamageCalculator;

impl ExplosionDamageCalculator for SimpleExplosionDamageCalculator {
    fn block_explosion_resistance(
        &self,
        _state: BlockStateId,
        block: BlockRef,
        fluid: Option<FluidRef>,
    ) -> Option<f32> {
        // Vanilla: returns the max of block resistance and any fluid resistance.
        // If the block is air (resistance 0 and no special shape), vanilla returns empty Optional.
        // We replicate: resistance 0 with no fluid → None (treat as transparent).
        let block_res = block.config.explosion_resistance;
        // FluidRef is &'static Fluid, so explosion_resistance is accessed directly.
        let fluid_res = fluid.map(|f| f.explosion_resistance).unwrap_or(0.0);

        let resistance = block_res.max(fluid_res);

        // Vanilla skips blocks with zero resistance AND no shape (i.e., air-like).
        // We use block.config.is_air as the marker.
        if resistance == 0.0 && block.config.is_air {
            None
        } else {
            Some(resistance)
        }
    }

    fn should_block_explode(&self, _state: BlockStateId, _block: BlockRef, _power: f32) -> bool {
        true
    }

    fn entity_damage_amount(
        &self,
        _entity: &SharedEntity,
        _explosion_center: DVec3,
        radius: f32,
        distance_factor: f64,
        seen_percent: f64,
    ) -> f32 {
        // Vanilla formula (ServerExplosion.hurtEntities / ExplosionDamageCalculator):
        //   d  = distance_factor          (already computed by caller)
        //   e  = (1 - d) * seen_percent
        //   damage = ((e*e + e) / 2) * 7 * (radius * 2) + 1
        let e = (1.0 - distance_factor) * seen_percent;
        let damage = ((e * e + e) / 2.0) * 7.0 * f64::from(radius * 2.0) + 1.0;
        damage as f32
    }
}
