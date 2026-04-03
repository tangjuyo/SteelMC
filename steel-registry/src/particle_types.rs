//! Particle type registry IDs (0-indexed from PARTICLE_TYPE registry).
//!
//! Used with `CExplode` and other packets that encode particle types as a
//! VarInt registry ID (via `ByteBufCodecs.registry(Registries.PARTICLE_TYPE)`).
//!
//! Only `SimpleParticleType` constants are listed here, as those have no
//! extra data following the ID on the wire. Particles with extra data
//! (e.g., `BLOCK`, `DUST`, `ITEM`) require their own serialization structs.
//!
//! # Note
//! These IDs are derived from the vanilla `ParticleTypes.java` registration
//! order. They should be extracted via SteelExtractor in the future, similar
//! to `sound_events.json`.
//!
//! TODO: Extract particle type IDs via SteelExtractor.

/// Large explosion emitter. Used for TNT, beds, creepers, and other large explosions.
///
/// Vanilla: `minecraft:explosion_emitter`
pub const EXPLOSION_EMITTER: i32 = 22;

/// Small explosion particle. Used within explosion animations for block debris.
///
/// Vanilla: `minecraft:explosion`
pub const EXPLOSION: i32 = 23;
