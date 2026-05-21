//! Per-player statistics system.
#![expect(missing_docs, reason = "stats methods are self-documenting; full docs live on PlayerStats")]
//!
//! Mirrors vanilla's `ServerStatsCounter` — tracks nine stat categories (mined,
//! crafted, used, broken, picked_up, dropped, killed, killed_by, custom) and
//! flushes only changed entries to the client via `CAwardStats`.
//!
//! **Stat type IDs** (must match vanilla `Stats.java` registration order):
//! 0=mined, 1=crafted, 2=used, 3=broken, 4=picked_up, 5=dropped,
//! 6=killed, 7=killed_by, 8=custom

pub mod custom_stat;

pub use custom_stat::CustomStat;

use rustc_hash::{FxHashMap, FxHashSet};
use steel_registry::RegistryEntry;
use steel_registry::blocks::BlockRef;
use steel_registry::entity_types::EntityTypeRef;
use steel_registry::items::ItemRef;

use steel_protocol::packets::game::{CAwardStats, StatEntry};

use crate::player::Player;

pub(crate) const STAT_MINED: u8 = 0;
pub(crate) const STAT_CRAFTED: u8 = 1;
pub(crate) const STAT_USED: u8 = 2;
pub(crate) const STAT_BROKEN: u8 = 3;
pub(crate) const STAT_PICKED_UP: u8 = 4;
pub(crate) const STAT_DROPPED: u8 = 5;
pub(crate) const STAT_KILLED: u8 = 6;
pub(crate) const STAT_KILLED_BY: u8 = 7;
pub(crate) const STAT_CUSTOM: u8 = 8;

/// All per-player statistics with dirty tracking.
///
/// Registry-keyed stats (blocks, items, entities) are stored in `FxHashMap<usize, i32>` keyed
/// by the registry ID of the subject. Custom stats use a flat array indexed by `CustomStat`
/// discriminant for O(1) access.
///
/// Persisted to disk in Steel binary format (`STLS` magic) via `player_data_storage`.
pub struct PlayerStats {
    /// Blocks mined (stat type 0).
    pub(crate) blocks_mined: FxHashMap<usize, i32>,
    /// Items crafted (stat type 1).
    pub(crate) items_crafted: FxHashMap<usize, i32>,
    /// Items used (stat type 2).
    pub(crate) items_used: FxHashMap<usize, i32>,
    /// Items broken (stat type 3).
    pub(crate) items_broken: FxHashMap<usize, i32>,
    /// Items picked up (stat type 4).
    pub(crate) items_picked_up: FxHashMap<usize, i32>,
    /// Items dropped (stat type 5).
    pub(crate) items_dropped: FxHashMap<usize, i32>,
    /// Entities killed (stat type 6).
    pub(crate) entities_killed: FxHashMap<usize, i32>,
    /// Killed-by entity type (stat type 7).
    pub(crate) entities_killed_by: FxHashMap<usize, i32>,
    /// Custom statistics (stat type 8), indexed by `CustomStat` discriminant.
    pub(crate) custom: [i32; CustomStat::COUNT],
    /// Set of `(stat_type, entry_id)` pairs that have changed since the last flush.
    dirty: FxHashSet<(u8, u32)>,
}

impl PlayerStats {
    pub fn new() -> Self {
        Self {
            blocks_mined: FxHashMap::default(),
            items_crafted: FxHashMap::default(),
            items_used: FxHashMap::default(),
            items_broken: FxHashMap::default(),
            items_dropped: FxHashMap::default(),
            items_picked_up: FxHashMap::default(),
            entities_killed: FxHashMap::default(),
            entities_killed_by: FxHashMap::default(),
            custom: [0; CustomStat::COUNT],
            dirty: FxHashSet::default(),
        }
    }

    pub fn increment_block_mined(&mut self, block: BlockRef, amount: i32) {
        let id = block.id();
        increment_map(&mut self.blocks_mined, id, amount);
        self.dirty.insert((STAT_MINED, id as u32));
    }

    pub fn increment_item_crafted(&mut self, item: ItemRef, amount: i32) {
        let id = item.id();
        increment_map(&mut self.items_crafted, id, amount);
        self.dirty.insert((STAT_CRAFTED, id as u32));
    }

    pub fn increment_item_used(&mut self, item: ItemRef, amount: i32) {
        let id = item.id();
        increment_map(&mut self.items_used, id, amount);
        self.dirty.insert((STAT_USED, id as u32));
    }

    pub fn increment_item_broken(&mut self, item: ItemRef, amount: i32) {
        let id = item.id();
        increment_map(&mut self.items_broken, id, amount);
        self.dirty.insert((STAT_BROKEN, id as u32));
    }

    pub fn increment_item_picked_up(&mut self, item: ItemRef, amount: i32) {
        let id = item.id();
        increment_map(&mut self.items_picked_up, id, amount);
        self.dirty.insert((STAT_PICKED_UP, id as u32));
    }

    pub fn increment_item_dropped(&mut self, item: ItemRef, amount: i32) {
        let id = item.id();
        increment_map(&mut self.items_dropped, id, amount);
        self.dirty.insert((STAT_DROPPED, id as u32));
    }

    pub fn increment_entity_killed(&mut self, entity: EntityTypeRef, amount: i32) {
        let id = entity.id();
        increment_map(&mut self.entities_killed, id, amount);
        self.dirty.insert((STAT_KILLED, id as u32));
    }

    pub fn increment_killed_by(&mut self, entity: EntityTypeRef, amount: i32) {
        let id = entity.id();
        increment_map(&mut self.entities_killed_by, id, amount);
        self.dirty.insert((STAT_KILLED_BY, id as u32));
    }

    pub fn increment_custom(&mut self, stat: CustomStat, amount: i32) {
        let idx = stat as usize;
        self.custom[idx] = self.custom[idx].saturating_add(amount);
        self.dirty.insert((STAT_CUSTOM, idx as u32));
    }

    pub fn set_custom(&mut self, stat: CustomStat, value: i32) {
        let idx = stat as usize;
        self.custom[idx] = value;
        self.dirty.insert((STAT_CUSTOM, idx as u32));
    }

    pub fn get_custom(&self, stat: CustomStat) -> i32 {
        self.custom[stat as usize]
    }

    /// Marks every non-zero statistic dirty so the next flush sends the full snapshot.
    /// Called on first join so the client receives the persisted counters.
    pub fn mark_all_dirty(&mut self) {
        for (&id, &val) in &self.blocks_mined {
            if val != 0 {
                self.dirty.insert((STAT_MINED, id as u32));
            }
        }
        for (&id, &val) in &self.items_crafted {
            if val != 0 {
                self.dirty.insert((STAT_CRAFTED, id as u32));
            }
        }
        for (&id, &val) in &self.items_used {
            if val != 0 {
                self.dirty.insert((STAT_USED, id as u32));
            }
        }
        for (&id, &val) in &self.items_broken {
            if val != 0 {
                self.dirty.insert((STAT_BROKEN, id as u32));
            }
        }
        for (&id, &val) in &self.items_picked_up {
            if val != 0 {
                self.dirty.insert((STAT_PICKED_UP, id as u32));
            }
        }
        for (&id, &val) in &self.items_dropped {
            if val != 0 {
                self.dirty.insert((STAT_DROPPED, id as u32));
            }
        }
        for (&id, &val) in &self.entities_killed {
            if val != 0 {
                self.dirty.insert((STAT_KILLED, id as u32));
            }
        }
        for (&id, &val) in &self.entities_killed_by {
            if val != 0 {
                self.dirty.insert((STAT_KILLED_BY, id as u32));
            }
        }
        for (idx, &val) in self.custom.iter().enumerate() {
            if val != 0 {
                self.dirty.insert((STAT_CUSTOM, idx as u32));
            }
        }
    }

    /// Sends all dirty statistics to the client and clears the dirty set.
    /// No-op if nothing has changed.
    pub fn flush_dirty(&mut self, player: &Player) {
        if self.dirty.is_empty() {
            return;
        }

        let dirty: Vec<(u8, u32)> = self.dirty.drain().collect();
        let entries = dirty
            .into_iter()
            .map(|(stat_type, entry_id)| StatEntry {
                stat_type,
                entry_id,
                value: self.value_for(stat_type, entry_id),
            })
            .collect();

        player.send_packet(CAwardStats { stats: entries });
    }

    fn value_for(&self, stat_type: u8, entry_id: u32) -> i32 {
        let id = entry_id as usize;
        match stat_type {
            STAT_MINED => self.blocks_mined.get(&id).copied().unwrap_or(0),
            STAT_CRAFTED => self.items_crafted.get(&id).copied().unwrap_or(0),
            STAT_USED => self.items_used.get(&id).copied().unwrap_or(0),
            STAT_BROKEN => self.items_broken.get(&id).copied().unwrap_or(0),
            STAT_PICKED_UP => self.items_picked_up.get(&id).copied().unwrap_or(0),
            STAT_DROPPED => self.items_dropped.get(&id).copied().unwrap_or(0),
            STAT_KILLED => self.entities_killed.get(&id).copied().unwrap_or(0),
            STAT_KILLED_BY => self.entities_killed_by.get(&id).copied().unwrap_or(0),
            STAT_CUSTOM => self.custom.get(id).copied().unwrap_or(0),
            _ => 0,
        }
    }
}

fn increment_map(map: &mut FxHashMap<usize, i32>, id: usize, amount: i32) {
    let entry = map.entry(id).or_insert(0);
    *entry = entry.saturating_add(amount);
}
