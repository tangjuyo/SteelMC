use std::io::Cursor;

use steel_macros::{ReadFrom, ServerPacket};
use steel_utils::{Identifier, serial::ReadFrom};

/// Action type for `SSeenAdvancements`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ReadFrom)]
#[read(as = VarInt)]
#[repr(i32)]
pub enum SeenAdvancementsAction {
    OpenedTab = 0,
    ClosedScreen = 1,
}

/// Serverbound packet sent when the player opens or closes the advancement screen.
///
/// Mirrors vanilla `ServerboundSeenAdvancementsPacket`.
#[derive(Debug, Clone, ServerPacket)]
pub struct SSeenAdvancements {
    pub action: SeenAdvancementsAction,
    /// The selected tab ID; present only when `action == OpenedTab`.
    pub tab_id: Option<Identifier>,
}

impl ReadFrom for SSeenAdvancements {
    fn read(data: &mut Cursor<&[u8]>) -> std::io::Result<Self> {
        let action = SeenAdvancementsAction::read(data)?;
        let tab_id = if action == SeenAdvancementsAction::OpenedTab {
            Some(Identifier::read(data)?)
        } else {
            None
        };
        Ok(Self { action, tab_id })
    }
}
