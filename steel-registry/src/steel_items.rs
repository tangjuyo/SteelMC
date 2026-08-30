//! Steel-original "fun" item stacks with no vanilla counterpart, built from
//! the existing consume-effect/mob-effect data model — no new behavior
//! code, just data.
//!
//! These are component patches on top of a *real* vanilla item id
//! (`golden_apple`/`potion`), not brand-new item types: `minecraft:item` is
//! a vanilla built-in registry, never synced to the client, so the client
//! can only ever represent an item id it was compiled with. A genuinely new
//! item id (e.g. `steel:notch_apple`) corrupts the network stream the
//! moment it's sent. Item *components*, on the other hand, travel per-stack
//! over the network exactly like an enchanted or renamed item's do — that's
//! the supported way to customize an existing item.

use text_components::TextComponent;

use crate::consume_effect::{ApplyStatusEffectsConsumeEffect, vanilla_consume_effect_types};
use crate::data_components::PotionContents;
use crate::data_components::vanilla_components::{self, Consumable, ItemUseAnimation};
use crate::item_stack::ItemStack;
use crate::mob_effect::MobEffectCategory;
use crate::mob_effect_instance::MobEffectInstance;
use crate::sound_event::SoundEventHolder;
use crate::{ConsumeEffectData, sound_events, vanilla_items, vanilla_mob_effects};

/// How long each granted effect lasts: 5 minutes.
const NOTCH_EFFECT_DURATION: i32 = 6000;
/// Amplifier for each granted effect: level V.
const NOTCH_EFFECT_AMPLIFIER: i32 = 4;

/// Every vanilla mob effect that exists — the full roster `all_mob_effects`
/// and `beneficial_mob_effects` filter down from.
const ALL_EFFECTS: &[crate::mob_effect::MobEffectRef] = &[
    vanilla_mob_effects::SPEED,
    vanilla_mob_effects::SLOWNESS,
    vanilla_mob_effects::HASTE,
    vanilla_mob_effects::MINING_FATIGUE,
    vanilla_mob_effects::STRENGTH,
    vanilla_mob_effects::INSTANT_HEALTH,
    vanilla_mob_effects::INSTANT_DAMAGE,
    vanilla_mob_effects::JUMP_BOOST,
    vanilla_mob_effects::NAUSEA,
    vanilla_mob_effects::REGENERATION,
    vanilla_mob_effects::RESISTANCE,
    vanilla_mob_effects::FIRE_RESISTANCE,
    vanilla_mob_effects::WATER_BREATHING,
    vanilla_mob_effects::INVISIBILITY,
    vanilla_mob_effects::BLINDNESS,
    vanilla_mob_effects::NIGHT_VISION,
    vanilla_mob_effects::HUNGER,
    vanilla_mob_effects::WEAKNESS,
    vanilla_mob_effects::POISON,
    vanilla_mob_effects::WITHER,
    vanilla_mob_effects::HEALTH_BOOST,
    vanilla_mob_effects::ABSORPTION,
    vanilla_mob_effects::SATURATION,
    vanilla_mob_effects::GLOWING,
    vanilla_mob_effects::LEVITATION,
    vanilla_mob_effects::LUCK,
    vanilla_mob_effects::UNLUCK,
    vanilla_mob_effects::SLOW_FALLING,
    vanilla_mob_effects::CONDUIT_POWER,
    vanilla_mob_effects::DOLPHINS_GRACE,
    vanilla_mob_effects::BAD_OMEN,
    vanilla_mob_effects::HERO_OF_THE_VILLAGE,
    vanilla_mob_effects::DARKNESS,
    vanilla_mob_effects::TRIAL_OMEN,
    vanilla_mob_effects::RAID_OMEN,
    vanilla_mob_effects::WIND_CHARGED,
    vanilla_mob_effects::WEAVING,
    vanilla_mob_effects::OOZING,
    vanilla_mob_effects::INFESTED,
    vanilla_mob_effects::BREATH_OF_THE_NAUTILUS,
];

/// Builds `MobEffectInstance`s for effects passing `filter`.
///
/// Instantaneous effects (Instant Health/Damage, Saturation — see
/// `MobEffectBehavior::is_instantaneous`) get the vanilla default duration
/// of 1 tick instead of `NOTCH_EFFECT_DURATION`: vanilla never puts an
/// instantaneous effect in an `apply_effects`-style list with a long
/// duration, because `shouldApplyEffectTickThisTick` fires on nearly every
/// tick of the remaining duration for those effects, not just once.
fn mob_effects_where(filter: impl Fn(MobEffectCategory) -> bool) -> Vec<MobEffectInstance> {
    let instantaneous = [
        vanilla_mob_effects::INSTANT_HEALTH,
        vanilla_mob_effects::INSTANT_DAMAGE,
        vanilla_mob_effects::SATURATION,
    ];

    ALL_EFFECTS
        .iter()
        .copied()
        .filter(|effect| filter(effect.category))
        .map(|effect| {
            let duration = if instantaneous.iter().any(|i| i.key == effect.key) {
                1
            } else {
                NOTCH_EFFECT_DURATION
            };
            MobEffectInstance::simple(effect, duration, NOTCH_EFFECT_AMPLIFIER)
        })
        .collect()
}

/// Every registered vanilla mob effect, including harmful ones (Poison,
/// Wither, Blindness, ...) — deliberately exhaustive and deliberately
/// chaotic, not a curated "best effects" list.
fn all_mob_effects() -> Vec<MobEffectInstance> {
    mob_effects_where(|_| true)
}

/// Only effects vanilla itself classifies as `MobEffectCategory::Beneficial`
/// — excludes both harmful ones (Poison, Wither, ...) and neutral ones
/// (Glowing, Bad Omen, ...), matching vanilla's own tooltip-color
/// classification rather than a hand-curated list.
fn beneficial_mob_effects() -> Vec<MobEffectInstance> {
    mob_effects_where(|category| category == MobEffectCategory::Beneficial)
}

/// A golden apple stack renamed "Notch Apple" whose `Consumable` component
/// applies every registered mob effect on consumption, through the ordinary
/// `apply_effects` consume effect — the same mechanism a real golden apple
/// uses, just with a longer effect list.
#[must_use]
pub fn notch_apple_stack() -> ItemStack {
    let mut stack = ItemStack::new(&vanilla_items::GOLDEN_APPLE);
    stack.set(vanilla_components::CUSTOM_NAME, TextComponent::plain("Notch Apple"));
    stack.set(
        vanilla_components::CONSUMABLE,
        Consumable::new(
            Consumable::DEFAULT_CONSUME_SECONDS,
            ItemUseAnimation::Eat,
            SoundEventHolder::registry(&sound_events::ENTITY_GENERIC_EAT),
            true,
            vec![ConsumeEffectData::new(
                &vanilla_consume_effect_types::APPLY_EFFECTS,
                ApplyStatusEffectsConsumeEffect::new(all_mob_effects(), 1.0)
                    .expect("probability 1.0 is always valid"),
            )],
        )
        .expect("consume_seconds is a valid constant"),
    );
    stack
}

/// A potion stack renamed "Notch Potion" whose `PotionContents` component
/// lists every *beneficial* mob effect (see `beneficial_mob_effects`),
/// applied through the same `PotionItem` behavior a real potion uses (the
/// item id itself is `minecraft:potion`, which already carries the right
/// item behavior).
#[must_use]
pub fn notch_potion_stack() -> ItemStack {
    let mut stack = ItemStack::new(&vanilla_items::POTION);
    stack.set(vanilla_components::CUSTOM_NAME, TextComponent::plain("Notch Potion"));
    stack.set(
        vanilla_components::POTION_CONTENTS,
        PotionContents::new(None, None, beneficial_mob_effects(), None),
    );
    stack
}

#[cfg(test)]
mod tests {
    use super::{all_mob_effects, beneficial_mob_effects, notch_apple_stack, notch_potion_stack};
    use crate::consume_effect::ApplyStatusEffectsConsumeEffect;
    use crate::data_components::vanilla_components;
    use crate::mob_effect::MobEffectCategory;
    use crate::{
        REGISTRY, RegistryExt as _, init_vanilla_registry, vanilla_items, vanilla_mob_effects,
    };

    #[test]
    fn all_mob_effects_covers_every_registered_effect_exactly_once() {
        init_vanilla_registry();
        let effects = all_mob_effects();
        assert_eq!(effects.len(), REGISTRY.mob_effects.len());

        let mut seen = rustc_hash::FxHashSet::default();
        for effect in &effects {
            assert!(
                seen.insert(effect.effect().key.clone()),
                "{} listed more than once",
                effect.effect().key
            );
        }
    }

    #[test]
    fn instantaneous_effects_get_a_one_tick_duration() {
        init_vanilla_registry();
        let effects = all_mob_effects();
        for instant in [
            vanilla_mob_effects::INSTANT_HEALTH,
            vanilla_mob_effects::INSTANT_DAMAGE,
            vanilla_mob_effects::SATURATION,
        ] {
            let found = effects
                .iter()
                .find(|effect| effect.effect().key == instant.key)
                .expect("instantaneous effect should be in the list");
            assert_eq!(found.duration(), 1);
        }
    }

    #[test]
    fn notch_apple_stack_keeps_the_real_golden_apple_item_id() {
        init_vanilla_registry();
        let stack = notch_apple_stack();
        assert!(stack.is(&vanilla_items::GOLDEN_APPLE));

        let consumable = stack
            .get(vanilla_components::CONSUMABLE)
            .expect("notch apple should have a Consumable component");
        let apply_effects = consumable
            .on_consume_effects()
            .iter()
            .find_map(|effect| effect.downcast_ref::<ApplyStatusEffectsConsumeEffect>())
            .expect("notch apple should have an apply_effects consume effect");
        assert_eq!(apply_effects.effects().len(), REGISTRY.mob_effects.len());
    }

    #[test]
    fn notch_potion_stack_keeps_the_real_potion_item_id() {
        init_vanilla_registry();
        let stack = notch_potion_stack();
        assert!(stack.is(&vanilla_items::POTION));

        let contents = stack
            .get(vanilla_components::POTION_CONTENTS)
            .expect("notch potion should have a PotionContents component");
        assert_eq!(
            contents.all_effects().len(),
            beneficial_mob_effects().len()
        );
    }

    #[test]
    fn beneficial_mob_effects_excludes_harmful_and_neutral_categories() {
        init_vanilla_registry();
        let effects = beneficial_mob_effects();
        assert!(effects.len() > 1);
        for effect in &effects {
            assert_eq!(effect.effect().category, MobEffectCategory::Beneficial);
        }
        // Poison (Harmful) and Glowing (Neutral) must not sneak into a
        // "positive effects only" list.
        assert!(
            !effects
                .iter()
                .any(|effect| effect.effect().key == vanilla_mob_effects::POISON.key)
        );
        assert!(
            !effects
                .iter()
                .any(|effect| effect.effect().key == vanilla_mob_effects::GLOWING.key)
        );
    }
}
