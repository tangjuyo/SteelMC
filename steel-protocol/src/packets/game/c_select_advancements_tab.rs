use std::io::{Result, Write};

use steel_macros::ClientPacket;
use steel_registry::packets::play::C_SELECT_ADVANCEMENTS_TAB;
use steel_utils::{Identifier, serial::WriteTo};

/// Sent when the server changes the currently displayed advancement tab, or closes it.
///
/// Mirrors vanilla `ClientboundSelectAdvancementsTabPacket`.
/// `tab_id = None` closes the advancement screen tab selection.
#[derive(Debug, ClientPacket)]
#[packet_id(Play = C_SELECT_ADVANCEMENTS_TAB)]
pub struct CSelectAdvancementsTab {
    pub tab_id: Option<Identifier>,
}

impl WriteTo for CSelectAdvancementsTab {
    fn write(&self, writer: &mut impl Write) -> Result<()> {
        match &self.tab_id {
            Some(id) => {
                true.write(writer)?;
                id.write(writer)
            }
            None => false.write(writer),
        }
    }
}
