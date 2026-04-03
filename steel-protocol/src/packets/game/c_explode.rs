use std::io::Write;

use steel_macros::ClientPacket;
use steel_registry::packets::play::C_EXPLODE;
use steel_utils::codec::VarInt;
use steel_utils::serial::WriteTo;

/// A single particle entry in an explosion's block particle weighted list.
///
/// Mirrors vanilla's `ExplosionParticleInfo` + `Weighted` wrapper as serialized
/// inside `WeightedList<ExplosionParticleInfo>`.
///
/// Only `SimpleParticleType` particles are supported here (no extra particle data
/// on the wire beyond the type ID). Use `steel_registry::particle_types` constants.
#[derive(Clone, Debug)]
pub struct ExplosionBlockParticle {
    /// Particle type ID (0-indexed from PARTICLE_TYPE registry, VarInt).
    pub particle_id: i32,
    /// Particle size scaling factor.
    pub scaling: f32,
    /// Particle speed multiplier.
    pub speed: f32,
    /// Relative weight for random selection in the weighted list (VarInt).
    pub weight: i32,
}

/// Sent to a player when an explosion occurs within 64 blocks of them.
///
/// Drives all client-side explosion visuals: main particle, block debris particles,
/// explosion sound, and the player's own knockback impulse.
///
/// Each player receives their own copy with a player-specific `player_knockback`.
/// Players outside 64 blocks do not receive this packet.
///
/// Vanilla: `ClientboundExplodePacket`
#[derive(ClientPacket, Clone, Debug)]
#[packet_id(Play = C_EXPLODE)]
pub struct CExplode {
    /// Explosion center X.
    pub x: f64,
    /// Explosion center Y.
    pub y: f64,
    /// Explosion center Z.
    pub z: f64,
    /// Explosion radius, used by the client to scale block particle effects.
    pub radius: f32,
    /// Total number of blocks destroyed by this explosion.
    pub block_count: i32,
    /// Knockback velocity to apply to this player, if they are affected.
    /// `None` if the player is outside the knockback range or is flying/creative.
    pub player_knockback: Option<[f64; 3]>,
    /// Main explosion particle type ID (VarInt, 0-indexed from PARTICLE_TYPE registry).
    ///
    /// Use `steel_registry::particle_types::EXPLOSION_EMITTER` for large explosions
    /// (TNT, creeper) or `EXPLOSION` for small ones.
    pub explosion_particle_id: i32,
    /// Sound event ID for the explosion (from `steel_registry::sound_events`).
    ///
    /// Encoded on the wire as a `Holder<SoundEvent>` reference: `VarInt(id + 1)`.
    /// ID 0 would mean a direct (inline) sound definition; registry references start at 1.
    pub explosion_sound_id: i32,
    /// Block debris particle entries for the explosion animation.
    ///
    /// The client randomly samples from this weighted list when spawning debris particles.
    pub block_particles: Vec<ExplosionBlockParticle>,
}

impl WriteTo for CExplode {
    fn write(&self, writer: &mut impl Write) -> std::io::Result<()> {
        // Vec3 center (3x f64 big-endian)
        writer.write_all(&self.x.to_be_bytes())?;
        writer.write_all(&self.y.to_be_bytes())?;
        writer.write_all(&self.z.to_be_bytes())?;

        // float radius
        writer.write_all(&self.radius.to_be_bytes())?;

        // int blockCount — vanilla uses ByteBufCodecs.INT (big-endian i32, not VarInt)
        writer.write_all(&self.block_count.to_be_bytes())?;

        // Optional<Vec3> playerKnockback
        match self.player_knockback {
            None => writer.write_all(&[0u8])?,
            Some([kx, ky, kz]) => {
                writer.write_all(&[1u8])?;
                writer.write_all(&kx.to_be_bytes())?;
                writer.write_all(&ky.to_be_bytes())?;
                writer.write_all(&kz.to_be_bytes())?;
            }
        }

        // ParticleOptions explosionParticle
        // Encoded via ByteBufCodecs.registry (0-indexed VarInt, no holder offset)
        VarInt(self.explosion_particle_id).write(writer)?;

        // Holder<SoundEvent> explosionSound
        // Encoded via ByteBufCodecs.holder: registry reference = VarInt(id + 1)
        VarInt(self.explosion_sound_id + 1).write(writer)?;

        // WeightedList<ExplosionParticleInfo>
        // = VarInt count + [ParticleOptions + f32 scaling + f32 speed + VarInt weight]*
        VarInt(self.block_particles.len() as i32).write(writer)?;
        for entry in &self.block_particles {
            VarInt(entry.particle_id).write(writer)?;
            writer.write_all(&entry.scaling.to_be_bytes())?;
            writer.write_all(&entry.speed.to_be_bytes())?;
            VarInt(entry.weight).write(writer)?;
        }

        Ok(())
    }
}

impl CExplode {
    /// Creates a standard TNT explosion packet.
    ///
    /// Uses `EXPLOSION_EMITTER` particle and `ENTITY_GENERIC_EXPLODE` sound with the
    /// default block particle (`EXPLOSION`, scale 0.3, speed 1.0, weight 1).
    #[must_use]
    pub fn tnt(
        x: f64,
        y: f64,
        z: f64,
        radius: f32,
        block_count: i32,
        player_knockback: Option<[f64; 3]>,
    ) -> Self {
        Self {
            x,
            y,
            z,
            radius,
            block_count,
            player_knockback,
            explosion_particle_id: steel_registry::particle_types::EXPLOSION_EMITTER,
            explosion_sound_id: steel_registry::sound_events::ENTITY_GENERIC_EXPLODE,
            block_particles: vec![ExplosionBlockParticle {
                particle_id: steel_registry::particle_types::EXPLOSION,
                scaling: 0.3,
                speed: 1.0,
                weight: 1,
            }],
        }
    }
}
