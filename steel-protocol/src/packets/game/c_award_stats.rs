//! Clientbound award stats packet - sends dirty statistic counters to the client.

use std::io::{Result, Write};

use steel_macros::ClientPacket;
use steel_registry::packets::play::C_AWARD_STATS;
use steel_utils::{codec::VarInt, serial::WriteTo};

/// A single statistic entry to send to the client.
///
/// Wire encoding: VarInt(stat_type_id) + VarInt(entry_id) + VarInt(value).
/// Both stat_type_id and entry_id are indices in their respective vanilla registries.
#[derive(Debug, Clone, Copy)]
pub struct StatEntry {
    /// Index of the stat type in the STAT_TYPE registry (0–8).
    pub stat_type: u8,
    /// Registry index of the stat's subject (block/item/entity type/custom stat).
    pub entry_id: u32,
    /// Absolute value of the counter (not a delta).
    pub value: i32,
}

/// Sends dirty statistic counters to the client.
///
/// Vanilla only sends statistics that changed since the last flush; the client
/// merges the update into its local store. On first join `mark_all_dirty()` is called
/// so the full non-zero snapshot is delivered.
///
/// Corresponds to `ClientboundAwardStatsPacket` in vanilla.
#[derive(ClientPacket, Debug)]
#[packet_id(Play = C_AWARD_STATS)]
pub struct CAwardStats {
    pub stats: Vec<StatEntry>,
}

impl WriteTo for CAwardStats {
    fn write(&self, writer: &mut impl Write) -> Result<()> {
        VarInt(self.stats.len() as i32).write(writer)?;
        for entry in &self.stats {
            VarInt(i32::from(entry.stat_type)).write(writer)?;
            VarInt(entry.entry_id as i32).write(writer)?;
            VarInt(entry.value).write(writer)?;
        }
        Ok(())
    }
}
