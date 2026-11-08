//! Per-target result aggregation shared by multi-target commands.
//!
//! Vanilla's `CommandResponseTracker` carries `Messages`, `MessagesWithArg` and
//! `MessagesWithArgs` handler records so Java lambdas can receive extra
//! arguments. Rust closures capture those arguments directly, so Steel exposes
//! a single [`CommandResponseTracker::send_feedback`] taking the `on_single`
//! and `on_multiple` handlers as closures instead.

use text_components::TextComponent;

use super::super::{brigadier::CommandSyntaxError, execution::CommandSource};

/// Which tracked elements count toward the single/multiple feedback choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ElementType {
    /// Every tracked element.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "used by vanilla clear/scoreboard/advancement feedback, not ported yet"
        )
    )]
    Any,
    /// Only elements tracked with a non-zero value.
    NonZero,
}

/// Accumulates per-element command results and picks vanilla's feedback form.
pub(super) struct CommandResponseTracker<E> {
    total_value: i32,
    only_element: Option<E>,
    element_count: i32,
    only_non_zero_element: Option<E>,
    non_zero_element_count: i32,
}

impl<E> Default for CommandResponseTracker<E> {
    fn default() -> Self {
        Self {
            total_value: 0,
            only_element: None,
            element_count: 0,
            only_non_zero_element: None,
            non_zero_element_count: 0,
        }
    }
}

impl<E: Clone> CommandResponseTracker<E> {
    /// Records `element` with its result `value`.
    pub(super) fn track(&mut self, element: E, value: i32) {
        self.total_value = self.total_value.wrapping_add(value);
        self.element_count += 1;
        self.only_element = (self.element_count == 1).then(|| element.clone());

        if value != 0 {
            self.non_zero_element_count += 1;
            self.only_non_zero_element = (self.non_zero_element_count == 1).then_some(element);
        }
    }

    /// Records `element` as succeeded (`1`) or failed (`0`).
    pub(super) fn track_bool(&mut self, element: E, value: bool) {
        self.track(element, i32::from(value));
    }

    /// Returns the sum of all tracked values.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "used by vanilla clear/advancement failure checks, not ported yet"
        )
    )]
    pub(super) const fn total_value(&self) -> i32 {
        self.total_value
    }

    const fn first_element(&self, element_type: ElementType) -> Option<&E> {
        match element_type {
            ElementType::Any => self.only_element.as_ref(),
            ElementType::NonZero => self.only_non_zero_element.as_ref(),
        }
    }

    const fn element_count(&self, element_type: ElementType) -> i32 {
        match element_type {
            ElementType::Any => self.element_count,
            ElementType::NonZero => self.non_zero_element_count,
        }
    }

    /// Builds the single- or multiple-element result.
    ///
    /// `on_single` receives the only counted element and the total value;
    /// `on_multiple` receives the counted element count and the total value.
    pub(super) fn dispatch<R>(
        &self,
        element_type: ElementType,
        on_single: impl FnOnce(&E, i32) -> R,
        on_multiple: impl FnOnce(i32, i32) -> R,
    ) -> R {
        match self.first_element(element_type) {
            Some(element) => on_single(element, self.total_value),
            None => on_multiple(self.element_count(element_type), self.total_value),
        }
    }

    /// Fails with `on_zero` when no element counts, otherwise sends the
    /// success feedback built by [`Self::dispatch`] and returns the total value.
    pub(super) fn send_feedback(
        &self,
        source: &CommandSource,
        broadcast: bool,
        element_type: ElementType,
        on_zero: impl FnOnce() -> CommandSyntaxError,
        on_single: impl FnOnce(&E, i32) -> TextComponent,
        on_multiple: impl FnOnce(i32, i32) -> TextComponent,
    ) -> Result<i32, CommandSyntaxError> {
        if self.element_count(element_type) == 0 {
            return Err(on_zero());
        }
        let message = self.dispatch(element_type, on_single, on_multiple);
        source.send_success(&message, broadcast);
        Ok(self.total_value)
    }
}

#[cfg(test)]
mod tests {
    use super::{CommandResponseTracker, ElementType};

    fn describe(
        tracker: &CommandResponseTracker<&'static str>,
        element_type: ElementType,
    ) -> String {
        tracker.dispatch(
            element_type,
            |element, total| format!("single {element} {total}"),
            |count, total| format!("multiple {count} {total}"),
        )
    }

    #[test]
    fn single_non_zero_element_among_many_uses_single_feedback() {
        let mut tracker = CommandResponseTracker::default();
        tracker.track_bool("a", false);
        tracker.track_bool("b", true);
        tracker.track_bool("c", false);

        assert_eq!(describe(&tracker, ElementType::NonZero), "single b 1");
        assert_eq!(describe(&tracker, ElementType::Any), "multiple 3 1");
    }

    #[test]
    fn several_non_zero_elements_use_multiple_feedback_with_their_count() {
        let mut tracker = CommandResponseTracker::default();
        tracker.track("a", 2);
        tracker.track("b", 0);
        tracker.track("c", 3);

        assert_eq!(describe(&tracker, ElementType::NonZero), "multiple 2 5");
        assert_eq!(tracker.total_value(), 5);
    }
}
