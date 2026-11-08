//! Vanilla mob-effect command.

use std::{borrow::Cow, slice};

use steel_registry::mob_effect::{MobEffectRef, instance::INFINITE_EFFECT_DURATION};
use steel_utils::{Identifier, translations};
use text_components::{
    TextComponent,
    translation::{TranslatedMessage, Translation},
};

use super::{
    super::{
        brigadier::{ArgumentType, CommandNodeBuilder, CommandSyntaxError},
        execution::{
            CommandSource, SteelArgumentType, SteelCommandContext, SteelCommandRuntime, argument,
            literal,
        },
        registration::CommandRegistration,
    },
    response_tracker::{CommandResponseTracker, ElementType},
};
use crate::{
    behavior::MOB_EFFECT_BEHAVIORS,
    entity::{MobEffectInstance, SharedEntity},
};

/// Duration of `/effect give` without a `seconds` argument for non-instant effects.
const DEFAULT_DURATION_TICKS: i32 = 600;
/// Duration of `/effect give` without a `seconds` argument for instant effects.
const DEFAULT_INSTANTANEOUS_DURATION_TICKS: i32 = 1;

pub(super) fn registration() -> CommandRegistration<CommandSource> {
    CommandRegistration::new(Identifier::vanilla_static("effect"), |_| command())
}

fn command() -> CommandNodeBuilder<CommandSource, SteelCommandRuntime> {
    literal("effect")
        .then(
            literal("clear").executes(clear_self).then(
                argument("targets", SteelArgumentType::entities())
                    .executes(clear_targets)
                    .then(
                        argument("effect", SteelArgumentType::mob_effect())
                            .executes(clear_specific),
                    ),
            ),
        )
        .then(
            literal("give").then(
                argument("targets", SteelArgumentType::entities()).then(
                    argument("effect", SteelArgumentType::mob_effect())
                        .executes(|context| give(context, None, 0, true))
                        .then(
                            argument("seconds", ArgumentType::integer(1, 1_000_000))
                                .executes(give_seconds)
                                .then(
                                    argument("amplifier", ArgumentType::integer(0, 255))
                                        .executes(give_seconds_amplifier)
                                        .then(
                                            argument("hideParticles", ArgumentType::bool())
                                                .executes(give_seconds_amplifier_particles),
                                        ),
                                ),
                        )
                        .then(
                            literal("infinite")
                                .executes(|context| {
                                    give(context, Some(INFINITE_EFFECT_DURATION), 0, true)
                                })
                                .then(
                                    argument("amplifier", ArgumentType::integer(0, 255))
                                        .executes(give_infinite_amplifier)
                                        .then(
                                            argument("hideParticles", ArgumentType::bool())
                                                .executes(give_infinite_amplifier_particles),
                                        ),
                                ),
                        ),
                ),
            ),
        )
}

fn give_seconds(context: &SteelCommandContext<CommandSource>) -> Result<i32, CommandSyntaxError> {
    give(context, Some(context.integer("seconds")?), 0, true)
}

fn give_seconds_amplifier(
    context: &SteelCommandContext<CommandSource>,
) -> Result<i32, CommandSyntaxError> {
    let seconds = context.integer("seconds")?;
    give(context, Some(seconds), context.integer("amplifier")?, true)
}

fn give_seconds_amplifier_particles(
    context: &SteelCommandContext<CommandSource>,
) -> Result<i32, CommandSyntaxError> {
    let seconds = context.integer("seconds")?;
    let amplifier = context.integer("amplifier")?;
    let particles = !context.boolean("hideParticles")?;
    give(context, Some(seconds), amplifier, particles)
}

fn give_infinite_amplifier(
    context: &SteelCommandContext<CommandSource>,
) -> Result<i32, CommandSyntaxError> {
    let amplifier = context.integer("amplifier")?;
    give(context, Some(INFINITE_EFFECT_DURATION), amplifier, true)
}

fn give_infinite_amplifier_particles(
    context: &SteelCommandContext<CommandSource>,
) -> Result<i32, CommandSyntaxError> {
    let amplifier = context.integer("amplifier")?;
    let particles = !context.boolean("hideParticles")?;
    give(
        context,
        Some(INFINITE_EFFECT_DURATION),
        amplifier,
        particles,
    )
}

/// `seconds` is `None` without a duration argument and
/// [`INFINITE_EFFECT_DURATION`] for the `infinite` literal.
fn give(
    context: &SteelCommandContext<CommandSource>,
    seconds: Option<i32>,
    amplifier: i32,
    particles: bool,
) -> Result<i32, CommandSyntaxError> {
    let targets = context.entities("targets")?;
    let effect = context.mob_effect("effect")?;
    let duration = compute_duration_in_ticks(seconds, effect);
    let source = context.source();
    let source_entity = source.entity().map(|entity| &**entity);

    let mut tracker = CommandResponseTracker::default();
    for target in &targets {
        let Some(living) = target.as_living_entity() else {
            continue;
        };
        // Vanilla's 5-argument constructor reuses `visible` for `showIcon`.
        let instance = MobEffectInstance::with_duration(effect, duration, amplifier)
            .with_visible(particles)
            .with_show_icon(particles);
        tracker.track_bool(
            target,
            living.add_mob_effect_with_source(instance, source_entity),
        );
    }

    let seconds_arg = || TextComponent::plain((duration / 20).to_string());
    tracker.send_feedback(
        source,
        true,
        ElementType::NonZero,
        || translated_error(&translations::COMMANDS_EFFECT_GIVE_FAILED),
        // The vanilla English strings ignore the third (seconds) argument,
        // but it is still sent for other languages.
        |target, _| {
            give_success_message(
                &translations::COMMANDS_EFFECT_GIVE_SUCCESS_SINGLE,
                [effect.display_name(), target.display_name(), seconds_arg()],
            )
        },
        |count, _| {
            give_success_message(
                &translations::COMMANDS_EFFECT_GIVE_SUCCESS_MULTIPLE,
                [
                    effect.display_name(),
                    TextComponent::plain(count.to_string()),
                    seconds_arg(),
                ],
            )
        },
    )
}

/// Converts the command's `seconds` argument to the instance duration.
///
/// Instant effects take the argument as a raw tick count.
fn compute_duration_in_ticks(seconds: Option<i32>, effect: MobEffectRef) -> i32 {
    let instantaneous = MOB_EFFECT_BEHAVIORS
        .get_behavior(effect)
        .as_instantaneous()
        .is_some();
    match seconds {
        Some(seconds) if instantaneous => seconds,
        Some(INFINITE_EFFECT_DURATION) => INFINITE_EFFECT_DURATION,
        Some(seconds) => seconds * 20,
        None if instantaneous => DEFAULT_INSTANTANEOUS_DURATION_TICKS,
        None => DEFAULT_DURATION_TICKS,
    }
}

fn clear_self(context: &SteelCommandContext<CommandSource>) -> Result<i32, CommandSyntaxError> {
    let entity = context.source().entity_or_error()?;
    clear_all(context.source(), slice::from_ref(entity))
}

fn clear_targets(context: &SteelCommandContext<CommandSource>) -> Result<i32, CommandSyntaxError> {
    let targets = context.entities("targets")?;
    clear_all(context.source(), &targets)
}

fn clear_all(source: &CommandSource, targets: &[SharedEntity]) -> Result<i32, CommandSyntaxError> {
    let mut tracker = CommandResponseTracker::default();
    for target in targets {
        let Some(living) = target.as_living_entity() else {
            continue;
        };
        tracker.track_bool(target, living.remove_all_mob_effects());
    }

    tracker.send_feedback(
        source,
        true,
        ElementType::NonZero,
        || translated_error(&translations::COMMANDS_EFFECT_CLEAR_EVERYTHING_FAILED),
        |target, _| {
            translations::COMMANDS_EFFECT_CLEAR_EVERYTHING_SUCCESS_SINGLE
                .message([target.display_name()])
                .component()
        },
        |count, _| {
            translations::COMMANDS_EFFECT_CLEAR_EVERYTHING_SUCCESS_MULTIPLE
                .message([TextComponent::plain(count.to_string())])
                .component()
        },
    )
}

fn clear_specific(context: &SteelCommandContext<CommandSource>) -> Result<i32, CommandSyntaxError> {
    let targets = context.entities("targets")?;
    let effect = context.mob_effect("effect")?;

    let mut tracker = CommandResponseTracker::default();
    for target in &targets {
        let Some(living) = target.as_living_entity() else {
            continue;
        };
        tracker.track_bool(target, living.remove_mob_effect(effect));
    }

    tracker.send_feedback(
        context.source(),
        true,
        ElementType::NonZero,
        || translated_error(&translations::COMMANDS_EFFECT_CLEAR_SPECIFIC_FAILED),
        |target, _| {
            translations::COMMANDS_EFFECT_CLEAR_SPECIFIC_SUCCESS_SINGLE
                .message([effect.display_name(), target.display_name()])
                .component()
        },
        |count, _| {
            translations::COMMANDS_EFFECT_CLEAR_SPECIFIC_SUCCESS_MULTIPLE
                .message([
                    effect.display_name(),
                    TextComponent::plain(count.to_string()),
                ])
                .component()
        },
    )
}

fn translated_error(translation: &Translation<0>) -> CommandSyntaxError {
    CommandSyntaxError::dynamic(TextComponent::from(translation))
}

/// Builds a give-success message with the seconds argument vanilla appends
/// after the two placeholders of the English string.
fn give_success_message(translation: &Translation<2>, args: [TextComponent; 3]) -> TextComponent {
    TextComponent::translated(TranslatedMessage {
        key: Cow::Borrowed(translation.0),
        fallback: None,
        args: Some(Box::new(args)),
    })
}

#[cfg(test)]
mod tests {
    use steel_registry::{
        init_vanilla_registry, mob_effect::instance::INFINITE_EFFECT_DURATION, vanilla_mob_effects,
    };

    use super::compute_duration_in_ticks;
    use crate::behavior::init_behaviors;

    #[test]
    fn duration_uses_seconds_for_lasting_effects_and_ticks_for_instant_ones() {
        init_vanilla_registry();
        init_behaviors();
        let speed = vanilla_mob_effects::SPEED;
        let instant_health = vanilla_mob_effects::INSTANT_HEALTH;

        assert_eq!(compute_duration_in_ticks(None, speed), 600);
        assert_eq!(compute_duration_in_ticks(Some(5), speed), 100);
        assert_eq!(
            compute_duration_in_ticks(Some(INFINITE_EFFECT_DURATION), speed),
            INFINITE_EFFECT_DURATION
        );
        assert_eq!(compute_duration_in_ticks(None, instant_health), 1);
        assert_eq!(compute_duration_in_ticks(Some(5), instant_health), 5);
        assert_eq!(
            compute_duration_in_ticks(Some(INFINITE_EFFECT_DURATION), instant_health),
            INFINITE_EFFECT_DURATION
        );
    }
}
