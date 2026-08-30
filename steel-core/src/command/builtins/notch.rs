//! Steel "notch" item commands: give the sender a component-patched vanilla
//! golden apple/potion that applies every registered mob effect.
//!
//! These reuse a real vanilla item id (`golden_apple`/`potion`) rather than
//! a made-up one — the client has no way to represent an item id outside
//! its own compiled-in registry, so a genuinely new item type cannot exist
//! on the wire. See `steel_registry::steel_items` for the stack builders.

use std::sync::Arc;

use steel_protocol::packets::game::SoundSource;
use steel_registry::{item_stack::ItemStack, sound_events, steel_items};
use steel_utils::Identifier;
use text_components::TextComponent;

use super::super::{
    brigadier::CommandSyntaxError,
    execution::{CommandSource, SteelCommandContext, literal},
    registration::CommandRegistration,
};
use crate::{entity::Entity as _, inventory::container::Container as _, player::Player};

pub(super) fn apple_registration() -> CommandRegistration<CommandSource> {
    CommandRegistration::new(Identifier::from_steel("notchapple"), |_| {
        literal("notchapple").executes(give_notch_apple)
    })
}

pub(super) fn potion_registration() -> CommandRegistration<CommandSource> {
    CommandRegistration::new(Identifier::from_steel("notchpotion"), |_| {
        literal("notchpotion").executes(give_notch_potion)
    })
}

fn give_notch_apple(context: &SteelCommandContext<CommandSource>) -> Result<i32, CommandSyntaxError> {
    give_notch_stack(context, steel_items::notch_apple_stack(), "Notch Apple")
}

fn give_notch_potion(
    context: &SteelCommandContext<CommandSource>,
) -> Result<i32, CommandSyntaxError> {
    give_notch_stack(context, steel_items::notch_potion_stack(), "Notch Potion")
}

fn give_notch_stack(
    context: &SteelCommandContext<CommandSource>,
    stack: ItemStack,
    display_name: &str,
) -> Result<i32, CommandSyntaxError> {
    let player = source_player(context)?;
    give_to_player(player, stack);

    context.source().send_success(
        &TextComponent::plain(format!(
            "Gave 1 [{display_name}] to {}",
            player.plain_text_name()
        )),
        true,
    );
    Ok(1)
}

fn source_player(
    context: &SteelCommandContext<CommandSource>,
) -> Result<&Arc<Player>, CommandSyntaxError> {
    context.source().player().ok_or_else(|| {
        CommandSyntaxError::dynamic(TextComponent::plain(
            "This command can only be run by a player",
        ))
    })
}

fn give_to_player(player: &Player, mut stack: ItemStack) {
    let added = player.inventory.lock().add(&mut stack);

    if added && stack.is_empty() {
        player.broadcast_inventory_changes();
    } else if let Some(item) = player.drop_item(stack, false, false) {
        item.set_no_pickup_delay();
        item.set_owner(Some(player.gameprofile.id));
    }

    play_pickup_sound(player);
}

fn play_pickup_sound(player: &Player) {
    let pitch = ((rand::random::<f32>() - rand::random::<f32>()) * 0.7 + 1.0) * 2.0;
    player.get_world().play_sound_at(
        &sound_events::ENTITY_ITEM_PICKUP,
        SoundSource::Players,
        player.position(),
        0.2,
        pitch,
        None,
    );
}
