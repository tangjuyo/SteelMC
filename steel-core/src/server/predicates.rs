//! Predicate types that mirror vanilla's criterion condition system.
//!
//! These are deliberately simplified — fields that require the full loot-context
//! system (entity equipment, effects, NBT) are left as `Option<_>` and treated as
//! "always pass" until those sub-systems exist.
//!
//! All types support `serde::Deserialize` so that `TriggerRegistry` can parse them
//! from the raw JSON bytes stored in `StaticCriterionDef::conditions`.

use serde::Deserialize;

// ─────────────────────────────────────────────────────────────────────────────
// Range bounds
// ─────────────────────────────────────────────────────────────────────────────

/// Inclusive integer range where absent bounds mean "unbounded".
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MinMaxInt {
    /// Lower bound (inclusive). `None` means no lower bound.
    pub min: Option<i32>,
    /// Upper bound (inclusive). `None` means no upper bound.
    pub max: Option<i32>,
}

impl MinMaxInt {
    /// Returns true if `value` falls within `[min, max]`.
    pub fn matches(&self, value: i32) -> bool {
        self.min.map_or(true, |m| value >= m) && self.max.map_or(true, |m| value <= m)
    }
}

/// Inclusive float range where absent bounds mean "unbounded".
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MinMaxFloat {
    /// Lower bound (inclusive). `None` means no lower bound.
    pub min: Option<f64>,
    /// Upper bound (inclusive). `None` means no upper bound.
    pub max: Option<f64>,
}

impl MinMaxFloat {
    /// Returns true if `value` falls within `[min, max]`.
    pub fn matches(&self, value: f64) -> bool {
        self.min.map_or(true, |m| value >= m) && self.max.map_or(true, |m| value <= m)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Item predicate
// ─────────────────────────────────────────────────────────────────────────────

/// Matches an item stack by optional type and count.
///
/// `items` holds item keys like `"minecraft:diamond"` or tag refs like `"#minecraft:logs"`.
/// Tag refs are not yet resolved — they are silently ignored (always pass) until
/// the item tag system is wired.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ItemPredicate {
    /// Accepted item identifiers. `None` matches any item.
    #[serde(default)]
    pub items: Option<ItemsField>,
    /// Required stack size range. Defaults to unbounded.
    #[serde(default)]
    pub count: MinMaxInt,
}

/// Vanilla 1.21 allows `items` to be a single string or an array.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ItemsField {
    /// A single item identifier or tag ref.
    One(String),
    /// A list of item identifiers or tag refs (any match passes).
    Many(Vec<String>),
}

impl ItemsField {
    /// Returns true if `key` (e.g. `"minecraft:diamond"`) is in this field.
    /// Tag refs (`#...`) are not yet resolved and always pass.
    pub fn matches(&self, key: &str) -> bool {
        let ids: &[String] = match self {
            Self::One(s) => std::slice::from_ref(s),
            Self::Many(v) => v.as_slice(),
        };
        ids.iter().any(|id| {
            if id.starts_with('#') {
                true // TODO: resolve tags
            } else {
                id == key
            }
        })
    }
}

impl ItemPredicate {
    /// Returns true if both the item key and count satisfy this predicate.
    pub fn matches_item_key(&self, key: &str, count: i32) -> bool {
        self.items.as_ref().map_or(true, |f| f.matches(key)) && self.count.matches(count)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Slot predicate (used by inventory_changed)
// ─────────────────────────────────────────────────────────────────────────────

/// Matches inventory slot statistics by occupied, full, and empty slot counts.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SlotsPredicate {
    /// Range for the number of occupied (non-empty) slots.
    #[serde(default)]
    pub occupied: MinMaxInt,
    /// Range for the number of fully-stacked slots.
    #[serde(default)]
    pub full: MinMaxInt,
    /// Range for the number of empty slots.
    #[serde(default)]
    pub empty: MinMaxInt,
}

impl SlotsPredicate {
    /// Returns true if all three slot count ranges match.
    pub fn matches(&self, occupied: i32, full: i32, empty: i32) -> bool {
        self.occupied.matches(occupied) && self.full.matches(full) && self.empty.matches(empty)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Entity predicate
// ─────────────────────────────────────────────────────────────────────────────

/// Simplified entity predicate: only checks entity type key.
/// All other vanilla fields (effects, equipment, location, nbt, …) are unimplemented
/// and treated as always-pass.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EntityPredicate {
    /// Entity type key, e.g. `"minecraft:zombie"`. `None` matches any entity.
    #[serde(rename = "type")]
    pub entity_type: Option<EntityTypeField>,
}

/// `type` can be a single key or a tag ref (`#tag`).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum EntityTypeField {
    /// A single entity type key or tag ref.
    One(String),
    /// A list of entity type keys or tag refs (any match passes).
    Many(Vec<String>),
}

impl EntityTypeField {
    /// Returns true if `key` matches any entry. Tag refs always pass.
    pub fn matches(&self, key: &str) -> bool {
        let ids: &[String] = match self {
            Self::One(s) => std::slice::from_ref(s),
            Self::Many(v) => v.as_slice(),
        };
        ids.iter().any(|id| {
            if id.starts_with('#') {
                true // TODO: resolve entity type tags
            } else {
                id == key
            }
        })
    }
}

impl EntityPredicate {
    /// Returns true if the entity type matches (or no type constraint is set).
    pub fn matches_entity_key(&self, key: &str) -> bool {
        self.entity_type.as_ref().map_or(true, |t| t.matches(key))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Location predicate
// ─────────────────────────────────────────────────────────────────────────────

/// Simplified location predicate: only checks dimension key and biome.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LocationPredicate {
    /// Dimension key, e.g. `"minecraft:the_nether"`. `None` matches any dimension.
    pub dimension: Option<String>,
    /// Biome key. `None` matches any biome.
    pub biome: Option<String>,
    // x, y, z, light, structure left for future
}

impl LocationPredicate {
    /// Returns true if the dimension matches (or no dimension constraint is set).
    pub fn matches_dimension(&self, dimension_key: &str) -> bool {
        self.dimension.as_ref().map_or(true, |d| d == dimension_key)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Damage source predicate
// ─────────────────────────────────────────────────────────────────────────────

/// Simplified damage source predicate: checks damage type tags and source entity.
/// Full vanilla implementation uses loot context; this version is tag-name only.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DamageSourcePredicate {
    /// Damage type tags to check. Each entry is `(tag_id, expected_present)`.
    #[serde(default)]
    pub tags: Vec<TagCheck>,
    // direct_entity, source_entity left for future
}

/// A single damage-type tag assertion: the tag `id` must be present or absent.
#[derive(Debug, Clone, Deserialize)]
pub struct TagCheck {
    /// The damage type tag identifier, e.g. `"minecraft:is_fire"`.
    pub id: String,
    /// Whether the tag must be present (`true`) or absent (`false`).
    pub expected: bool,
}

impl DamageSourcePredicate {
    /// Returns true when no conditions are set (always passes).
    /// TODO: actually resolve damage type tags once tag system is wired.
    pub fn matches(&self) -> bool {
        // Until tag resolution is implemented, always pass.
        // This is safe: conditions will fire more broadly than vanilla but won't miss things.
        true
    }
}
