//! Per-player advancement progress tracking and network synchronisation.
//!
//! Mirrors vanilla's `PlayerAdvancements.java`.
//!
//! ## Design differences from vanilla
//!
//! - Progress is stored in a `FxHashMap<&'static str, FxHashMap<&'static str, i64>>`
//!   keyed by advancement ID → criterion name → completion epoch second (or 0 if not done).
//!   Vanilla uses `LinkedHashMap<AdvancementHolder, AdvancementProgress>`.
//! - Listener storage lives here (per-player), not on the trigger objects.
//!   Vanilla stores `Map<PlayerAdvancements, Set<Listener>>` on each `SimpleCriterionTrigger`,
//!   requiring a cross-player lock. Moving it here eliminates that.
//! - Persistence uses Steel binary format (`STLA` magic) instead of vanilla's JSON.
//!   (TODO: persistence is not yet implemented; progress resets on restart.)

use std::time::{SystemTime, UNIX_EPOCH};

use rustc_hash::{FxHashMap, FxHashSet};
use steel_protocol::packets::game::{
    CSelectAdvancementsTab, CUpdateAdvancements, NetworkAdvancementHolder, NetworkAdvancementProgress,
    NetworkCriterionProgress, NetworkDisplayInfo,
};
use steel_registry::vanilla_advancements::StaticAdvancementDef;
use steel_utils::Identifier;

use crate::{
    player::Player,
    server::{
        advancement_manager::AdvancementManager,
        trigger_registry::{TriggerContext, TriggerRegistry, condition_matches},
    },
};

/// A pending criterion that fires when its trigger is invoked.
struct CriterionListener {
    advancement_id: &'static str,
    criterion: &'static str,
    /// Index into `TriggerRegistry::conditions`.
    condition_idx: u32,
}

/// Three-state visibility rule used by the Reingold-Tilford visibility evaluator.
/// Matches vanilla's `AdvancementVisibilityEvaluator.VisibilityRule`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum VisibilityRule {
    Show,
    Hide,
    NoChange,
}

/// Per-player advancement state.
///
/// Owns progress data for all advancements the player has interacted with, plus
/// the dirty tracking needed to send incremental `CUpdateAdvancements` packets,
/// and the trigger listener table.
pub struct PlayerAdvancements {
    /// Criterion completion times: advancement_id → criterion_name → unix epoch seconds.
    /// Only entries where the criterion is *done* (obtained) are stored.
    progress: FxHashMap<&'static str, FxHashMap<&'static str, i64>>,
    /// Which advancement IDs are currently visible to this player.
    visible: FxHashSet<&'static str>,
    /// Advancement IDs whose progress changed since last flush.
    progress_changed: FxHashSet<&'static str>,
    /// Root IDs whose subtrees need visibility recalculation.
    roots_to_update: FxHashSet<&'static str>,
    /// Currently selected tab (root advancement ID), sent via `CSelectAdvancementsTab`.
    last_selected_tab: Option<&'static str>,
    /// True for the very first `flush_dirty()` after login (triggers `reset=true` packet).
    is_first_packet: bool,
    /// trigger_id → pending listeners for that trigger (only incomplete criteria).
    trigger_listeners: FxHashMap<&'static str, Vec<CriterionListener>>,
}

impl PlayerAdvancements {
    /// Creates a blank advancement state for a new player session.
    pub fn new() -> Self {
        Self {
            progress: FxHashMap::default(),
            visible: FxHashSet::default(),
            progress_changed: FxHashSet::default(),
            roots_to_update: FxHashSet::default(),
            last_selected_tab: None,
            is_first_packet: true,
            trigger_listeners: FxHashMap::default(),
        }
    }

    /// Mark all roots dirty on first join so the full tree is sent.
    pub fn mark_all_roots_dirty(&mut self, manager: &AdvancementManager) {
        for &root_id in manager.roots() {
            self.roots_to_update.insert(root_id);
        }
    }

    /// Register trigger listeners for all incomplete criteria.
    ///
    /// Called once after login (and after progress is loaded, when persistence is added).
    /// Mirrors vanilla's `PlayerAdvancements.registerListeners`.
    pub fn register_all_listeners(&mut self, _manager: &AdvancementManager, registry: &TriggerRegistry) {
        for def in steel_registry::vanilla_advancements::VANILLA_ADVANCEMENTS {
            let criteria_progress = self.progress.get(def.id);
            let is_done = criteria_progress
                .map(|cp| is_advancement_done(def, cp))
                .unwrap_or(false);
            if is_done {
                continue;
            }

            for crit in def.criteria {
                let already_obtained = criteria_progress
                    .and_then(|cp| cp.get(crit.name))
                    .is_some();
                if already_obtained {
                    continue;
                }

                let Some(idx) = registry.get_condition_idx(def.id, crit.name) else {
                    continue;
                };

                self.trigger_listeners
                    .entry(crit.trigger)
                    .or_default()
                    .push(CriterionListener {
                        advancement_id: def.id,
                        criterion: crit.name,
                        condition_idx: idx,
                    });
            }
        }
    }

    /// Fire a trigger for this player. Checks all registered listeners for `ctx.trigger_id()`,
    /// awards matching criteria, and returns `true` if anything changed.
    ///
    /// Mirrors vanilla's `SimpleCriterionTrigger.trigger(ServerPlayer, Predicate<T>)`.
    pub fn fire_trigger(
        &mut self,
        ctx: &TriggerContext<'_>,
        manager: &AdvancementManager,
        registry: &TriggerRegistry,
    ) -> bool {
        let trigger_id = ctx.trigger_id();
        let Some(listeners) = self.trigger_listeners.get(trigger_id) else {
            return false;
        };

        // Collect matching (advancement_id, criterion) pairs without holding borrow.
        let mut to_award: Vec<(&'static str, &'static str)> = Vec::new();
        for listener in listeners {
            let Some(condition) = registry.condition_by_idx(listener.condition_idx) else {
                continue;
            };
            if condition_matches(condition, ctx) {
                to_award.push((listener.advancement_id, listener.criterion));
            }
        }

        let mut any = false;
        for (adv_id, criterion) in to_award {
            if self.award(adv_id, criterion, manager) {
                any = true;
            }
        }
        any
    }

    /// Award a criterion for `advancement_id`. Returns `true` if the criterion was
    /// newly granted.
    ///
    /// If all requirements are now satisfied, marks the advancement done and
    /// unregisters its listeners.
    pub fn award(
        &mut self,
        advancement_id: &'static str,
        criterion: &'static str,
        manager: &AdvancementManager,
    ) -> bool {
        let entry = self.progress.entry(advancement_id).or_default();

        // Already done
        if entry.contains_key(criterion) {
            return false;
        }

        let now_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        entry.insert(criterion, now_epoch);
        self.progress_changed.insert(advancement_id);

        // Check if fully complete → mark root for visibility update + remove listeners.
        // The `is_done` check is scoped so the immutable borrow of `self.progress` ends
        // before the mutable method call.
        let fully_done = manager.get(advancement_id).is_some_and(|node| {
            let criteria_progress = &self.progress[advancement_id];
            is_advancement_done(node.def, criteria_progress)
        });

        if fully_done {
            if let Some(node) = manager.get(advancement_id) {
                let root_id = find_root(advancement_id, manager);
                self.roots_to_update.insert(root_id);
                self.unregister_listeners_for(advancement_id, node.def);
            }
        }

        true
    }

    /// Remove all trigger listeners for a completed advancement.
    ///
    /// Mirrors vanilla's `PlayerAdvancements.unregisterListeners`.
    fn unregister_listeners_for(
        &mut self,
        advancement_id: &'static str,
        def: &steel_registry::vanilla_advancements::StaticAdvancementDef,
    ) {
        for crit in def.criteria {
            if let Some(listeners) = self.trigger_listeners.get_mut(crit.trigger) {
                listeners.retain(|l| l.advancement_id != advancement_id);
                if listeners.is_empty() {
                    self.trigger_listeners.remove(crit.trigger);
                }
            }
        }
    }

    /// Flush pending changes to the client.
    ///
    /// If nothing changed (and this is not the first packet), no packet is sent.
    /// Called every player tick.
    pub fn flush_dirty(&mut self, player: &Player, manager: &AdvancementManager) {
        if !self.is_first_packet
            && self.roots_to_update.is_empty()
            && self.progress_changed.is_empty()
        {
            return;
        }

        let mut added: Vec<&'static str> = Vec::new();
        let mut removed: Vec<&'static str> = Vec::new();

        let roots: Vec<&'static str> = self.roots_to_update.drain().collect();
        for root_id in roots {
            self.update_tree_visibility(root_id, &mut added, &mut removed, manager);
        }

        let mut progress_entries: Vec<NetworkAdvancementProgress> = Vec::new();
        let changed: Vec<&'static str> = self.progress_changed.drain().collect();
        for adv_id in changed {
            if self.visible.contains(adv_id) {
                let criteria = self.build_network_progress(adv_id, manager);
                if !criteria.is_empty() {
                    progress_entries.push(NetworkAdvancementProgress {
                        advancement_id: static_id(adv_id),
                        criteria,
                    });
                }
            }
        }

        if !progress_entries.is_empty() || !added.is_empty() || !removed.is_empty() || self.is_first_packet {
            let added_holders = added
                .iter()
                .filter_map(|id| manager.get(id).map(|node| build_holder(node.def)))
                .collect();

            let removed_ids = removed
                .iter()
                .map(|id| static_id(id))
                .collect();

            player.send_packet(CUpdateAdvancements {
                reset: self.is_first_packet,
                added: added_holders,
                removed: removed_ids,
                progress: progress_entries,
                show_advancements: true,
            });
        }

        self.is_first_packet = false;
    }

    /// Handle `SSeenAdvancements` — player opened or closed the advancement screen tab.
    pub fn set_selected_tab(
        &mut self,
        tab_id: Option<&'static str>,
        player: &Player,
        manager: &AdvancementManager,
    ) {
        let new_tab = tab_id.and_then(|id| {
            manager
                .get(id)
                .filter(|node| node.parent.is_none() && node.def.display.is_some())
                .map(|_| id)
        });

        if self.last_selected_tab == new_tab {
            return;
        }

        self.last_selected_tab = new_tab;
        player.send_packet(CSelectAdvancementsTab {
            tab_id: new_tab.map(static_id),
        });
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Visibility
    // ─────────────────────────────────────────────────────────────────────────

    /// Recalculate visibility for the subtree rooted at `root_id`.
    ///
    /// Direct port of `PlayerAdvancements.updateTreeVisibility` and
    /// `AdvancementVisibilityEvaluator.evaluateVisibility`.
    fn update_tree_visibility(
        &mut self,
        root_id: &'static str,
        added: &mut Vec<&'static str>,
        removed: &mut Vec<&'static str>,
        manager: &AdvancementManager,
    ) {
        // 3 levels of ancestor stack, initialised to NoChange (matches vanilla)
        let mut ascendants = [VisibilityRule::NoChange; 3 + 1]; // +1 for the root itself
        let mut ascendant_top: usize = 0;

        // Push 3 NoChange entries (depth = VISIBILITY_DEPTH = 2, so 3 slots)
        for _ in 0..3 {
            ascendants[ascendant_top] = VisibilityRule::NoChange;
            ascendant_top += 1;
        }

        self.evaluate_visibility_dfs(
            root_id,
            &mut ascendants,
            &mut ascendant_top,
            added,
            removed,
            manager,
        );
    }

    /// DFS visibility evaluation — port of `AdvancementVisibilityEvaluator.evaluateVisibility`.
    ///
    /// Returns true if `node` itself or any descendant is done.
    fn evaluate_visibility_dfs(
        &mut self,
        id: &'static str,
        ascendants: &mut [VisibilityRule; 4],
        ascendant_top: &mut usize,
        added: &mut Vec<&'static str>,
        removed: &mut Vec<&'static str>,
        manager: &AdvancementManager,
    ) -> bool {
        let node = match manager.get(id) {
            Some(n) => n,
            None => return false,
        };

        let is_done = self.is_done(id, node.def);
        let rule = evaluate_visibility_rule(node.def, is_done);
        let mut is_self_or_descendant_done = is_done;

        // Push onto ascendant stack
        if *ascendant_top < ascendants.len() {
            ascendants[*ascendant_top] = rule;
        }
        *ascendant_top += 1;

        let children: Vec<&'static str> = node.children.clone();
        for child_id in children {
            is_self_or_descendant_done |=
                self.evaluate_visibility_dfs(child_id, ascendants, ascendant_top, added, removed, manager);
        }

        let visible = is_self_or_descendant_done
            || evaluate_visibility_for_unfinished(ascendants, *ascendant_top);

        *ascendant_top -= 1;

        if visible {
            if self.visible.insert(id) {
                added.push(id);
                if self.progress.contains_key(id) {
                    self.progress_changed.insert(id);
                }
            }
        } else if self.visible.remove(id) {
            removed.push(id);
        }

        is_self_or_descendant_done
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Helpers
    // ─────────────────────────────────────────────────────────────────────────

    fn is_done(&self, id: &str, def: &StaticAdvancementDef) -> bool {
        let Some(criteria_progress) = self.progress.get(id) else {
            return false;
        };
        is_advancement_done(def, criteria_progress)
    }

    fn build_network_progress(
        &self,
        id: &'static str,
        manager: &AdvancementManager,
    ) -> Vec<NetworkCriterionProgress> {
        let node = match manager.get(id) {
            Some(n) => n,
            None => return Vec::new(),
        };

        let obtained_map = self.progress.get(id);

        node.def
            .criteria
            .iter()
            .map(|crit| NetworkCriterionProgress {
                name: crit.name,
                obtained: obtained_map
                    .and_then(|m| m.get(crit.name))
                    .map(|&epoch_sec| epoch_sec * 1_000),
            })
            .collect()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Free helpers
// ─────────────────────────────────────────────────────────────────────────────

fn is_advancement_done(
    def: &StaticAdvancementDef,
    criteria_progress: &FxHashMap<&'static str, i64>,
) -> bool {
    if def.requirements.is_empty() {
        return false;
    }
    def.requirements.iter().all(|group| {
        group
            .iter()
            .any(|&crit| criteria_progress.contains_key(crit))
    })
}

fn evaluate_visibility_rule(def: &StaticAdvancementDef, is_done: bool) -> VisibilityRule {
    let Some(ref display) = def.display else {
        return VisibilityRule::Hide;
    };
    if is_done {
        return VisibilityRule::Show;
    }
    if display.flags & 4 != 0 {
        // hidden flag
        VisibilityRule::Hide
    } else {
        VisibilityRule::NoChange
    }
}

/// Mirrors `AdvancementVisibilityEvaluator.evaluateVisiblityForUnfinishedNode`.
/// Looks back up to `VISIBILITY_DEPTH = 2` ancestors.
fn evaluate_visibility_for_unfinished(ascendants: &[VisibilityRule; 4], top: usize) -> bool {
    for i in 0..=2 {
        let j = top.wrapping_sub(1 + i);
        let rule = if top > i && j < ascendants.len() { ascendants[j] } else { VisibilityRule::NoChange };
        match rule {
            VisibilityRule::Show => return true,
            VisibilityRule::Hide => return false,
            VisibilityRule::NoChange => {}
        }
    }
    false
}

fn find_root(mut id: &'static str, manager: &AdvancementManager) -> &'static str {
    loop {
        let node = match manager.get(id) {
            Some(n) => n,
            None => return id,
        };
        match node.parent {
            Some(parent_id) => id = parent_id,
            None => {
                // id is the root; return the static ref from the def
                return node.def.id;
            }
        }
    }
}

fn build_holder(def: &'static StaticAdvancementDef) -> NetworkAdvancementHolder {
    let display = def.display.as_ref().map(|d| NetworkDisplayInfo {
        title_nbt: d.title_nbt,
        description_nbt: d.description_nbt,
        icon_item_id: d.icon_item_id,
        frame: d.frame,
        flags: d.flags,
        background: d.background,
        x: d.x,
        y: d.y,
    });

    NetworkAdvancementHolder {
        id: static_id(def.id),
        parent: def.parent.map(static_id),
        display,
        requirements: def.requirements,
        sends_telemetry: def.sends_telemetry,
    }
}

/// Construct an `Identifier` from a static `"namespace:path"` string (e.g., `"minecraft:story/root"`).
fn static_id(id: &'static str) -> Identifier {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    Identifier::new_static(ns, path)
}
