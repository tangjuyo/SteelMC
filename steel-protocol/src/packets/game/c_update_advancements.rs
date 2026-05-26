use std::io::{Result, Write};

use steel_macros::ClientPacket;
use steel_registry::packets::play::C_UPDATE_ADVANCEMENTS;
use steel_utils::{Identifier, codec::VarInt, serial::{PrefixedWrite, WriteTo}};

/// Pre-serialized display info for an advancement, ready to write to the packet.
///
/// Unlike vanilla's `DisplayInfo` which recomputes NBT on every send, all fields here
/// are either statically baked (title/description NBT bytes) or computed once at startup
/// (x, y positions from `TreeNodePosition`).
#[derive(Debug, Clone)]
pub struct NetworkDisplayInfo {
    /// Pre-serialized NBT bytes for the title TextComponent.
    pub title_nbt: &'static [u8],
    /// Pre-serialized NBT bytes for the description TextComponent.
    pub description_nbt: &'static [u8],
    /// Item registry ID for the icon.
    pub icon_item_id: u32,
    /// Frame type: 0=TASK, 1=CHALLENGE, 2=GOAL (matches vanilla `AdvancementType` ordinals).
    pub frame: u8,
    /// Packed flags: bit0=has_background, bit1=show_toast, bit2=hidden.
    pub flags: i32,
    /// Background texture identifier (only present if `flags & 1 != 0`).
    pub background: Option<&'static str>,
    /// Tree X position (column), computed by `TreeNodePosition`.
    pub x: f32,
    /// Tree Y position (row), computed by `TreeNodePosition`.
    pub y: f32,
}

/// A single advancement entry in the `CUpdateAdvancements` packet.
#[derive(Debug, Clone)]
pub struct NetworkAdvancementHolder {
    /// The advancement's unique identifier.
    pub id: Identifier,
    /// Parent advancement ID, or `None` for root advancements.
    pub parent: Option<Identifier>,
    /// Display info, if this advancement has a visible entry in the advancement screen.
    pub display: Option<NetworkDisplayInfo>,
    /// AND-of-OR requirement groups: each outer group must have at least one criterion
    /// from its inner vec satisfied.
    pub requirements: &'static [&'static [&'static str]],
    /// Whether this advancement sends a telemetry event on completion.
    pub sends_telemetry: bool,
}

/// Per-criterion progress entry.
#[derive(Debug, Clone)]
pub struct NetworkCriterionProgress {
    /// Criterion name (matches a key in the advancement's `criteria` map).
    pub name: &'static str,
    /// Completion time as epoch milliseconds, or `None` if not yet obtained.
    pub obtained: Option<i64>,
}

/// Progress for one advancement, included in the `progress` map of the packet.
#[derive(Debug, Clone)]
pub struct NetworkAdvancementProgress {
    pub advancement_id: Identifier,
    pub criteria: Vec<NetworkCriterionProgress>,
}

/// Clientbound packet that synchronises the advancement tree and per-player progress.
///
/// Mirrors vanilla `ClientboundUpdateAdvancementsPacket`.
///
/// - `reset=true`: sent once on first join; client discards its local state before applying.
/// - `added`: full `AdvancementHolder` definitions (display, requirements, …) for newly
///   visible advancements.
/// - `removed`: IDs of advancements that are no longer visible (became hidden).
/// - `progress`: progress snapshots for advancements that have changed since last flush.
/// - `show_advancements`: mirrors the `announceAdvancements` game rule.
#[derive(Debug, ClientPacket)]
#[packet_id(Play = C_UPDATE_ADVANCEMENTS)]
pub struct CUpdateAdvancements {
    pub reset: bool,
    pub added: Vec<NetworkAdvancementHolder>,
    pub removed: Vec<Identifier>,
    pub progress: Vec<NetworkAdvancementProgress>,
    pub show_advancements: bool,
}

impl WriteTo for CUpdateAdvancements {
    fn write(&self, writer: &mut impl Write) -> Result<()> {
        self.reset.write(writer)?;

        VarInt(self.added.len() as i32).write(writer)?;
        for holder in &self.added {
            write_holder(holder, writer)?;
        }

        VarInt(self.removed.len() as i32).write(writer)?;
        for id in &self.removed {
            id.write(writer)?;
        }

        VarInt(self.progress.len() as i32).write(writer)?;
        for prog in &self.progress {
            prog.advancement_id.write(writer)?;
            write_progress(&prog.criteria, writer)?;
        }

        self.show_advancements.write(writer)
    }
}

fn write_holder(holder: &NetworkAdvancementHolder, writer: &mut impl Write) -> Result<()> {
    holder.id.write(writer)?;

    // Optional<Identifier> parent
    match &holder.parent {
        Some(parent) => {
            true.write(writer)?;
            parent.write(writer)?;
        }
        None => false.write(writer)?,
    }

    // Optional<DisplayInfo>
    match &holder.display {
        Some(display) => {
            true.write(writer)?;
            write_display(display, writer)?;
        }
        None => false.write(writer)?,
    }

    write_requirements(holder.requirements, writer)?;
    holder.sends_telemetry.write(writer)
}

fn write_display(display: &NetworkDisplayInfo, writer: &mut impl Write) -> Result<()> {
    // Title and description as pre-serialized NBT bytes (TRUSTED_STREAM_CODEC format)
    writer.write_all(display.title_nbt)?;
    writer.write_all(display.description_nbt)?;

    // ItemStackTemplate: VarInt(item_id), VarInt(count=1), empty DataComponentPatch
    VarInt(display.icon_item_id as i32).write(writer)?;
    VarInt(1_i32).write(writer)?;
    VarInt(0_i32).write(writer)?; // patch: 0 added components
    VarInt(0_i32).write(writer)?; // patch: 0 removed components

    // AdvancementType as VarInt (ordinal: 0=TASK, 1=CHALLENGE, 2=GOAL)
    VarInt(i32::from(display.frame)).write(writer)?;

    display.flags.write(writer)?;

    if display.flags & 1 != 0 {
        if let Some(bg) = display.background {
            bg.write_prefixed::<VarInt>(writer)?;
        }
    }

    display.x.write(writer)?;
    display.y.write(writer)
}

fn write_requirements(requirements: &[&[&str]], writer: &mut impl Write) -> Result<()> {
    VarInt(requirements.len() as i32).write(writer)?;
    for group in requirements {
        VarInt(group.len() as i32).write(writer)?;
        for criterion in *group {
            criterion.write_prefixed::<VarInt>(writer)?;
        }
    }
    Ok(())
}

fn write_progress(criteria: &[NetworkCriterionProgress], writer: &mut impl Write) -> Result<()> {
    VarInt(criteria.len() as i32).write(writer)?;
    for entry in criteria {
        entry.name.write_prefixed::<VarInt>(writer)?;
        match entry.obtained {
            Some(epoch_millis) => {
                true.write(writer)?;
                epoch_millis.write(writer)?;
            }
            None => false.write(writer)?,
        }
    }
    Ok(())
}
