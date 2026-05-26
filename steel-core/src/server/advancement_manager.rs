//! Global advancement registry shared by all players.
//!
//! Mirrors vanilla's `ServerAdvancementManager`. Loads all 1617 vanilla advancement
//! definitions from the compile-time generated `VANILLA_ADVANCEMENTS` array and
//! provides lookup by ID.
//!
//! Advancement definitions are static (built from the datapack at compile time); this
//! struct just organises them into a lookup map and records parent→children relationships
//! needed by the visibility evaluator.

use rustc_hash::FxHashMap;
use steel_registry::vanilla_advancements::{
    StaticAdvancementDef, VANILLA_ADVANCEMENTS,
};
use steel_utils::Identifier;

/// A single advancement node in the server's tree, combining the compiled definition
/// with its resolved parent/children indices.
pub struct AdvancementNode {
    /// Static definition (display, requirements, criteria, …).
    pub def: &'static StaticAdvancementDef,
    /// Parent advancement ID, if any.
    pub parent: Option<&'static str>,
    /// Children advancement IDs.
    pub children: Vec<&'static str>,
}

/// Global advancement registry. Built once at server startup from `VANILLA_ADVANCEMENTS`.
pub struct AdvancementManager {
    /// All advancements keyed by their string ID (e.g. `"minecraft:story/root"`).
    nodes: FxHashMap<&'static str, AdvancementNode>,
    /// Root advancement IDs (those with no parent).
    roots: Vec<&'static str>,
}

impl AdvancementManager {
    /// Build the advancement tree from the compiled static data.
    pub fn new() -> Self {
        let mut nodes: FxHashMap<&'static str, AdvancementNode> =
            FxHashMap::with_capacity_and_hasher(VANILLA_ADVANCEMENTS.len(), Default::default());

        // First pass: create nodes
        for def in VANILLA_ADVANCEMENTS.iter() {
            nodes.insert(
                def.id,
                AdvancementNode {
                    def,
                    parent: def.parent,
                    children: Vec::new(),
                },
            );
        }

        // Second pass: wire children
        for def in VANILLA_ADVANCEMENTS.iter() {
            if let Some(parent_id) = def.parent {
                if let Some(parent_node) = nodes.get_mut(parent_id) {
                    parent_node.children.push(def.id);
                }
            }
        }

        let roots: Vec<&'static str> = VANILLA_ADVANCEMENTS
            .iter()
            .filter(|d| d.parent.is_none())
            .map(|d| d.id)
            .collect();

        Self { nodes, roots }
    }

    /// Look up an advancement node by its full ID string (e.g. `"minecraft:story/root"`).
    pub fn get(&self, id: &str) -> Option<&AdvancementNode> {
        self.nodes.get(id)
    }

    /// Look up an advancement node by its parsed `Identifier`.
    pub fn get_by_identifier(&self, id: &Identifier) -> Option<&AdvancementNode> {
        self.nodes.get(id.to_string().as_str())
    }

    /// All root advancement IDs (no parent).
    pub fn roots(&self) -> &[&'static str] {
        &self.roots
    }

    /// Iterate over all advancement nodes.
    pub fn iter(&self) -> impl Iterator<Item = &AdvancementNode> {
        self.nodes.values()
    }

    /// Walk the full subtree rooted at `root_id` and call `visitor(id, node)` for
    /// each node in DFS order. Used by the visibility evaluator.
    pub fn visit_subtree(
        &self,
        root_id: &str,
        visitor: &mut impl FnMut(&str, &AdvancementNode),
    ) {
        if let Some(root) = self.nodes.get(root_id) {
            visitor(root_id, root);
            for &child_id in &root.children {
                self.visit_subtree(child_id, visitor);
            }
        }
    }
}
