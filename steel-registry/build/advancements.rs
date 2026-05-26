//! Build script for generating vanilla advancement definitions.
//!
//! Reads 1617 JSON files from `build_assets/builtin_datapacks/minecraft/advancement/`,
//! computes tree node positions using the Reingold-Tilford algorithm (port of
//! vanilla's `TreeNodePosition.java`), pre-serializes TextComponent title/description
//! as NBT bytes, and emits a static `VANILLA_ADVANCEMENTS` array.
//!
//! The generated types mirror vanilla's `Advancement` and `DisplayInfo` but store
//! only what the network packet needs — criteria trigger conditions are omitted since
//! the trigger system is not yet implemented.

use std::{
    collections::{BTreeMap, HashMap},
    fs, io,
    path::Path,
};

use proc_macro2::{Literal, TokenStream};
use quote::quote;
use serde::Deserialize;
use serde_json::Value;

// ============================================================================
// JSON deserialization types
// ============================================================================

#[derive(Deserialize, Debug)]
struct AdvancementJson {
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    display: Option<DisplayJson>,
    #[serde(default)]
    criteria: BTreeMap<String, CriterionJson>,
    #[serde(default)]
    requirements: Option<Vec<Vec<String>>>,
    #[serde(default, rename = "sends_telemetry_event")]
    sends_telemetry: bool,
}

#[derive(Deserialize, Debug)]
struct DisplayJson {
    title: Value,
    description: Value,
    icon: IconJson,
    #[serde(default)]
    frame: Option<String>,
    #[serde(default)]
    show_toast: Option<bool>,
    #[serde(default)]
    hidden: Option<bool>,
    #[serde(default)]
    background: Option<String>,
}

#[derive(Deserialize, Debug)]
struct IconJson {
    id: String,
}

#[derive(Deserialize, Debug, Clone)]
struct CriterionJson {
    trigger: String,
    #[serde(default)]
    conditions: Value,
}

// ============================================================================
// Tree position algorithm (port of vanilla TreeNodePosition.java)
// ============================================================================

struct TreeNode {
    id: String,
    children: Vec<usize>, // indices into the flat node array
    // From AdvancementJson
    parent: Option<String>,
    display: Option<DisplayJson>,
    criteria: BTreeMap<String, CriterionJson>,
    requirements: Vec<Vec<String>>,
    sends_telemetry: bool,
    // Computed positions
    x: f32,
    y: f32,
}

/// Per-node working state for the Reingold-Tilford layout algorithm.
struct LayoutNode {
    node_idx: usize,
    parent_layout: Option<usize>,   // index into layout_nodes
    prev_sibling: Option<usize>,    // index into layout_nodes
    child_index: usize,
    children: Vec<usize>,           // indices into layout_nodes
    ancestor_layout: usize,         // self index by default
    thread: Option<usize>,
    x_depth: i32,
    y: f32,
    mod_val: f32,
    change: f32,
    shift: f32,
}

struct Layout {
    nodes: Vec<LayoutNode>,
}

impl Layout {
    fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    fn add_node(
        &mut self,
        node_idx: usize,
        parent_layout: Option<usize>,
        prev_sibling: Option<usize>,
        child_index: usize,
        depth: i32,
        tree_nodes: &[TreeNode],
    ) -> usize {
        let layout_idx = self.nodes.len();
        self.nodes.push(LayoutNode {
            node_idx,
            parent_layout,
            prev_sibling,
            child_index,
            children: Vec::new(),
            ancestor_layout: layout_idx,
            thread: None,
            x_depth: depth,
            y: -1.0,
            mod_val: 0.0,
            change: 0.0,
            shift: 0.0,
        });

        // Add children (only those with display info)
        let mut prev: Option<usize> = None;
        let child_indices: Vec<usize> = tree_nodes[node_idx].children.clone();
        for &child_node_idx in &child_indices {
            if tree_nodes[child_node_idx].display.is_some() {
                let ci = self.nodes[layout_idx].children.len();
                let child_layout_idx = self.add_node(
                    child_node_idx,
                    Some(layout_idx),
                    prev,
                    ci + 1,
                    depth + 1,
                    tree_nodes,
                );
                self.nodes[layout_idx].children.push(child_layout_idx);
                prev = Some(child_layout_idx);
            } else {
                // Skip invisible nodes but adopt their children
                let grandchildren: Vec<usize> = tree_nodes[child_node_idx].children.clone();
                for &gc_idx in &grandchildren {
                    if tree_nodes[gc_idx].display.is_some() {
                        let ci = self.nodes[layout_idx].children.len();
                        let gc_layout_idx = self.add_node(
                            gc_idx,
                            Some(layout_idx),
                            prev,
                            ci + 1,
                            depth + 1,
                            tree_nodes,
                        );
                        self.nodes[layout_idx].children.push(gc_layout_idx);
                        prev = Some(gc_layout_idx);
                    }
                }
            }
        }

        layout_idx
    }

    fn first_walk(&mut self, idx: usize) {
        let has_children = !self.nodes[idx].children.is_empty();
        if !has_children {
            let y = self.nodes[idx]
                .prev_sibling
                .map(|s| self.nodes[s].y + 1.0)
                .unwrap_or(0.0);
            self.nodes[idx].y = y;
        } else {
            let children: Vec<usize> = self.nodes[idx].children.clone();
            let mut default_ancestor = children[0];

            for child in children {
                self.first_walk(child);
                default_ancestor = self.apportion(child, default_ancestor);
            }

            self.execute_shifts(idx);

            let first_child = self.nodes[idx].children[0];
            let last_child = *self.nodes[idx].children.last().unwrap();
            let midpoint = (self.nodes[first_child].y + self.nodes[last_child].y) / 2.0;

            if let Some(prev_sib) = self.nodes[idx].prev_sibling {
                self.nodes[idx].y = self.nodes[prev_sib].y + 1.0;
                self.nodes[idx].mod_val = self.nodes[idx].y - midpoint;
            } else {
                self.nodes[idx].y = midpoint;
            }
        }
    }

    fn second_walk(&mut self, idx: usize, mod_sum: f32, depth: i32, min: &mut f32) {
        self.nodes[idx].y += mod_sum;
        self.nodes[idx].x_depth = depth;
        if self.nodes[idx].y < *min {
            *min = self.nodes[idx].y;
        }
        let children: Vec<usize> = self.nodes[idx].children.clone();
        for child in children {
            let child_mod = self.nodes[idx].mod_val;
            self.second_walk(child, mod_sum + child_mod, depth + 1, min);
        }
    }

    fn third_walk(&mut self, idx: usize, offset: f32) {
        self.nodes[idx].y += offset;
        let children: Vec<usize> = self.nodes[idx].children.clone();
        for child in children {
            self.third_walk(child, offset);
        }
    }

    fn execute_shifts(&mut self, idx: usize) {
        let mut shift = 0.0f32;
        let mut change = 0.0f32;
        let children: Vec<usize> = self.nodes[idx].children.clone();
        for &child in children.iter().rev() {
            self.nodes[child].y += shift;
            self.nodes[child].mod_val += shift;
            change += self.nodes[child].change;
            shift += self.nodes[child].shift + change;
        }
    }

    fn prev_or_thread(&self, idx: usize) -> Option<usize> {
        if self.nodes[idx].thread.is_some() {
            self.nodes[idx].thread
        } else {
            self.nodes[idx].children.first().copied()
        }
    }

    fn next_or_thread(&self, idx: usize) -> Option<usize> {
        if self.nodes[idx].thread.is_some() {
            self.nodes[idx].thread
        } else {
            self.nodes[idx].children.last().copied()
        }
    }

    fn apportion(&mut self, idx: usize, default_ancestor: usize) -> usize {
        let prev_sibling = match self.nodes[idx].prev_sibling {
            Some(s) => s,
            None => return default_ancestor,
        };

        let parent = self.nodes[idx].parent_layout.unwrap();
        let vol = self.nodes[parent].children[0];

        let mut vir = idx;
        let mut vor = idx;
        let mut vil = prev_sibling;
        let mut vol2 = vol;

        let mut sir = self.nodes[vir].mod_val;
        let mut sor = self.nodes[vor].mod_val;
        let mut sil = self.nodes[vil].mod_val;
        let mut sol = self.nodes[vol2].mod_val;

        loop {
            let next_vil = match self.next_or_thread(vil) {
                Some(n) => n,
                None => break,
            };
            let prev_vir = match self.prev_or_thread(vir) {
                Some(n) => n,
                None => break,
            };

            vil = next_vil;
            vir = prev_vir;

            let prev_vol = match self.prev_or_thread(vol2) {
                Some(n) => n,
                None => break,
            };
            let next_vor = match self.next_or_thread(vor) {
                Some(n) => n,
                None => break,
            };

            vol2 = prev_vol;
            vor = next_vor;

            self.nodes[vor].ancestor_layout = idx;

            let shift = (self.nodes[vil].y + sil) - (self.nodes[vir].y + sir) + 1.0;
            if shift > 0.0 {
                let ancestor = self.get_ancestor(vil, idx, default_ancestor);
                self.move_subtree(ancestor, idx, shift);
                sir += shift;
                sor += shift;
            }

            sil += self.nodes[vil].mod_val;
            sir += self.nodes[vir].mod_val;
            sol += self.nodes[vol2].mod_val;
            sor += self.nodes[vor].mod_val;
        }

        if self.next_or_thread(vil).is_some() && self.next_or_thread(vor).is_none() {
            let thread_target = self.next_or_thread(vil).unwrap();
            self.nodes[vor].thread = Some(thread_target);
            self.nodes[vor].mod_val += sil - sor;
        } else {
            if self.prev_or_thread(vir).is_some() && self.prev_or_thread(vol2).is_none() {
                let thread_target = self.prev_or_thread(vir).unwrap();
                self.nodes[vol2].thread = Some(thread_target);
                self.nodes[vol2].mod_val += sir - sol;
            }
            return idx;
        }

        default_ancestor
    }

    fn move_subtree(&mut self, left: usize, right: usize, shift: f32) {
        let subtrees = (self.nodes[right].child_index as f32)
            - (self.nodes[left].child_index as f32);
        if subtrees != 0.0 {
            self.nodes[right].change -= shift / subtrees;
            self.nodes[left].change += shift / subtrees;
        }
        self.nodes[right].shift += shift;
        self.nodes[right].y += shift;
        self.nodes[right].mod_val += shift;
    }

    fn get_ancestor(&self, vil: usize, idx: usize, default_ancestor: usize) -> usize {
        let ancestor = self.nodes[vil].ancestor_layout;
        let parent = self.nodes[idx].parent_layout.unwrap();
        if self.nodes[parent].children.contains(&ancestor) {
            ancestor
        } else {
            default_ancestor
        }
    }

    /// Collect the final (x, y) positions back into tree_nodes.
    fn finalize(&self, tree_nodes: &mut [TreeNode]) {
        for layout_node in &self.nodes {
            let tn = &mut tree_nodes[layout_node.node_idx];
            tn.x = layout_node.x_depth as f32;
            tn.y = layout_node.y;
        }
    }
}

/// Run the Reingold-Tilford tree layout on all roots and write positions to `tree_nodes`.
fn compute_positions(roots: &[usize], tree_nodes: &mut Vec<TreeNode>) {
    for &root_idx in roots {
        if tree_nodes[root_idx].display.is_none() {
            continue;
        }
        let mut layout = Layout::new();
        let root_layout = layout.add_node(root_idx, None, None, 1, 0, tree_nodes);
        layout.first_walk(root_layout);
        let root_y = layout.nodes[root_layout].y;
        let mut min = root_y;
        layout.second_walk(root_layout, 0.0, 0, &mut min);
        if min < 0.0 {
            layout.third_walk(root_layout, -min);
        }
        layout.finalize(tree_nodes);
    }
}

// ============================================================================
// NBT byte serialisation for TextComponent (translate only)
// ============================================================================

/// Encode a `serde_json::Value` TextComponent as MC protocol NBT bytes.
///
/// The MC protocol writes TextComponents using `TRUSTED_STREAM_CODEC`, which in 1.21
/// is raw NBT bytes (same format as `NbtTag::write` in simdnbt).
///
/// We only need to handle translate components (`{"translate": "..."}`), plain text
/// (`{"text": "..."}`), and recursively structured components. All vanilla advancement
/// titles and descriptions are translate components.
fn json_to_nbt_bytes(json: &Value) -> Vec<u8> {
    let mut buf = Vec::new();
    write_nbt_tag(json, &mut buf);
    buf
}

fn write_nbt_string(s: &str, buf: &mut Vec<u8>) {
    let bytes = s.as_bytes();
    let len = bytes.len() as u16;
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(bytes);
}

fn write_nbt_tag(value: &Value, buf: &mut Vec<u8>) {
    match value {
        Value::String(s) => {
            buf.push(8); // TAG_String
            write_nbt_string(s, buf);
        }
        Value::Bool(b) => {
            buf.push(1); // TAG_Byte
            buf.push(*b as u8);
        }
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                buf.push(3); // TAG_Int
                buf.extend_from_slice(&(i as i32).to_be_bytes());
            } else if let Some(f) = n.as_f64() {
                buf.push(5); // TAG_Float
                buf.extend_from_slice(&(f as f32).to_be_bytes());
            }
        }
        Value::Array(arr) => {
            buf.push(9); // TAG_List
            if arr.is_empty() {
                buf.push(0); // element type END
                buf.extend_from_slice(&0i32.to_be_bytes()); // length 0
            } else {
                // Determine element type from first element
                let elem_type = nbt_type_id(&arr[0]);
                buf.push(elem_type);
                buf.extend_from_slice(&(arr.len() as i32).to_be_bytes());
                for elem in arr {
                    write_nbt_tag_body(elem, buf);
                }
            }
        }
        Value::Object(obj) => {
            buf.push(10); // TAG_Compound
            for (key, val) in obj {
                let elem_type = nbt_type_id(val);
                buf.push(elem_type);
                write_nbt_string(key, buf);
                write_nbt_tag_body(val, buf);
            }
            buf.push(0); // TAG_End
        }
        Value::Null => {
            buf.push(1); // TAG_Byte, value 0
            buf.push(0);
        }
    }
}

fn nbt_type_id(value: &Value) -> u8 {
    match value {
        Value::Bool(_) => 1,
        Value::Number(n) => {
            if n.as_i64().is_some() { 3 } else { 5 }
        }
        Value::String(_) => 8,
        Value::Array(_) => 9,
        Value::Object(_) => 10,
        Value::Null => 1,
    }
}

/// Write the tag body (without type prefix).
fn write_nbt_tag_body(value: &Value, buf: &mut Vec<u8>) {
    match value {
        Value::Bool(b) => buf.push(*b as u8),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                buf.extend_from_slice(&(i as i32).to_be_bytes());
            } else if let Some(f) = n.as_f64() {
                buf.extend_from_slice(&(f as f32).to_be_bytes());
            }
        }
        Value::String(s) => write_nbt_string(s, buf),
        Value::Array(arr) => {
            if arr.is_empty() {
                buf.push(0);
                buf.extend_from_slice(&0i32.to_be_bytes());
            } else {
                let elem_type = nbt_type_id(&arr[0]);
                buf.push(elem_type);
                buf.extend_from_slice(&(arr.len() as i32).to_be_bytes());
                for elem in arr {
                    write_nbt_tag_body(elem, buf);
                }
            }
        }
        Value::Object(obj) => {
            for (key, val) in obj {
                let elem_type = nbt_type_id(val);
                buf.push(elem_type);
                write_nbt_string(key, buf);
                write_nbt_tag_body(val, buf);
            }
            buf.push(0); // TAG_End
        }
        Value::Null => buf.push(0),
    }
}

// ============================================================================
// Token generation helpers
// ============================================================================

fn bytes_literal(bytes: &[u8]) -> TokenStream {
    let elems = bytes.iter().map(|b| {
        let lit = Literal::u8_suffixed(*b);
        quote! { #lit }
    });
    quote! { &[#(#elems),*] }
}

fn option_str_literal(opt: &Option<String>) -> TokenStream {
    match opt {
        Some(s) => quote! { Some(#s) },
        None => quote! { None },
    }
}

fn bool_lit(b: bool) -> TokenStream {
    if b { quote! { true } } else { quote! { false } }
}

// ============================================================================
// Item ID lookup
// ============================================================================

#[derive(serde::Deserialize)]
struct ItemEntry {
    id: u32,
    name: String,
}

#[derive(serde::Deserialize)]
struct ItemsJson {
    items: Vec<ItemEntry>,
}

fn build_item_id_map() -> HashMap<String, u32> {
    let json = fs::read_to_string("build_assets/items.json").expect("items.json not found");
    let items: ItemsJson = serde_json::from_str(&json).expect("invalid items.json");
    items
        .items
        .into_iter()
        .map(|i| (format!("minecraft:{}", i.name), i.id))
        .collect()
}

// ============================================================================
// Main build function
// ============================================================================

pub(crate) fn build() -> TokenStream {
    println!("cargo:rerun-if-changed=build_assets/builtin_datapacks/minecraft/advancement/");
    println!("cargo:rerun-if-changed=build_assets/items.json");

    let adv_dir = Path::new("build_assets/builtin_datapacks/minecraft/advancement");
    let item_ids = build_item_id_map();

    // ── Load all JSONs ────────────────────────────────────────────────────────
    let mut raw: Vec<(String, AdvancementJson)> = Vec::new();
    collect_advancements(adv_dir, adv_dir, &mut raw);
    raw.sort_by(|a, b| a.0.cmp(&b.0));

    // ── Build the tree ────────────────────────────────────────────────────────
    let mut id_to_idx: HashMap<String, usize> = HashMap::new();
    let mut nodes: Vec<TreeNode> = Vec::with_capacity(raw.len());

    // First pass: create nodes
    for (id, adv) in &raw {
        let requirements = adv
            .requirements
            .clone()
            .unwrap_or_else(|| adv.criteria.keys().map(|k| vec![k.clone()]).collect());

        let idx = nodes.len();
        id_to_idx.insert(id.clone(), idx);
        nodes.push(TreeNode {
            id: id.clone(),
            children: Vec::new(),
            parent: adv.parent.clone(),
            display: None, // placeholder — we'll move display data in
            criteria: adv.criteria.clone(),
            requirements,
            sends_telemetry: adv.sends_telemetry,
            x: 0.0,
            y: 0.0,
        });
    }

    // Move display from raw (can't borrow raw and nodes mutably at once, so build a temp vec)
    let display_vec: Vec<Option<DisplayJson>> = raw.into_iter().map(|(_, adv)| adv.display).collect();
    for (i, display) in display_vec.into_iter().enumerate() {
        nodes[i].display = display;
    }

    // Second pass: wire parent→children
    let ids: Vec<String> = nodes.iter().map(|n| n.id.clone()).collect();
    let parents: Vec<Option<String>> = nodes.iter().map(|n| n.parent.clone()).collect();
    for (idx, parent_id) in parents.iter().enumerate() {
        if let Some(pid) = parent_id {
            if let Some(&parent_idx) = id_to_idx.get(pid.as_str()) {
                nodes[parent_idx].children.push(idx);
            }
        }
    }

    // Collect roots (no parent)
    let roots: Vec<usize> = ids
        .iter()
        .enumerate()
        .filter(|(i, _)| nodes[*i].parent.is_none())
        .map(|(i, _)| i)
        .collect();

    // ── Compute positions ─────────────────────────────────────────────────────
    compute_positions(&roots, &mut nodes);

    // ── Generate Rust code ────────────────────────────────────────────────────
    let def_count = nodes.len();

    let node_tokens: Vec<TokenStream> = nodes
        .iter()
        .map(|node| {
            let id_str = &node.id;

            let parent_token = match &node.parent {
                Some(p) => quote! { Some(#p) },
                None => quote! { None },
            };

            // Requirements: &'static [&'static [&'static str]]
            let req_groups: Vec<TokenStream> = node
                .requirements
                .iter()
                .map(|group| {
                    let names: Vec<&str> = group.iter().map(|s| s.as_str()).collect();
                    quote! { &[#(#names),*] }
                })
                .collect();
            let requirements_token = quote! { &[#(#req_groups),*] };

            // Criteria: &'static [StaticCriterionDef]
            let criteria_tokens: Vec<TokenStream> = node
                .criteria
                .iter()
                .map(|(name, crit)| {
                    let n = name.as_str();
                    let t = crit.trigger.as_str();
                    let cond_bytes = serde_json::to_vec(&crit.conditions).unwrap_or_default();
                    let cond_lit = bytes_literal(&cond_bytes);
                    quote! {
                        StaticCriterionDef { name: #n, trigger: #t, conditions: #cond_lit }
                    }
                })
                .collect();
            let criteria_token = quote! { &[#(#criteria_tokens),*] };

            let sends_telemetry = bool_lit(node.sends_telemetry);

            let display_token = if let Some(ref display) = node.display {
                let title_bytes = bytes_literal(&json_to_nbt_bytes(&display.title));
                let desc_bytes = bytes_literal(&json_to_nbt_bytes(&display.description));

                let icon_id = item_ids
                    .get(&display.icon.id)
                    .copied()
                    .unwrap_or_else(|| {
                        eprintln!(
                            "WARNING: unknown icon item '{}' for advancement '{}'",
                            display.icon.id, node.id
                        );
                        0
                    });

                let frame = match display.frame.as_deref() {
                    Some("challenge") => 1u8,
                    Some("goal") => 2u8,
                    _ => 0u8, // task (default)
                };

                let mut flags: i32 = 0;
                if display.background.is_some() { flags |= 1; }
                if display.show_toast.unwrap_or(true) { flags |= 2; }
                if display.hidden.unwrap_or(false) { flags |= 4; }

                let background_token = option_str_literal(&display.background);

                let x = node.x;
                let y = node.y;

                quote! {
                    Some(StaticDisplayDef {
                        title_nbt: #title_bytes,
                        description_nbt: #desc_bytes,
                        icon_item_id: #icon_id,
                        frame: #frame,
                        flags: #flags,
                        background: #background_token,
                        x: #x,
                        y: #y,
                    })
                }
            } else {
                quote! { None }
            };

            quote! {
                StaticAdvancementDef {
                    id: #id_str,
                    parent: #parent_token,
                    display: #display_token,
                    requirements: #requirements_token,
                    criteria: #criteria_token,
                    sends_telemetry: #sends_telemetry,
                }
            }
        })
        .collect();

    quote! {
        /// Display info for a vanilla advancement, as compiled by the build script.
        pub struct StaticDisplayDef {
            /// Pre-serialized NBT bytes for the title TextComponent (TRUSTED_STREAM_CODEC).
            pub title_nbt: &'static [u8],
            /// Pre-serialized NBT bytes for the description TextComponent.
            pub description_nbt: &'static [u8],
            /// Item registry ID for the icon.
            pub icon_item_id: u32,
            /// Frame type: 0=TASK, 1=CHALLENGE, 2=GOAL.
            pub frame: u8,
            /// Packed flags: bit0=has_background, bit1=show_toast, bit2=hidden.
            pub flags: i32,
            /// Background texture identifier (only when flags & 1 != 0).
            pub background: Option<&'static str>,
            /// Tree X position (column, computed by Reingold-Tilford at build time).
            pub x: f32,
            /// Tree Y position (row, computed by Reingold-Tilford at build time).
            pub y: f32,
        }

        /// A single criterion definition with its trigger type and raw condition JSON bytes.
        pub struct StaticCriterionDef {
            pub name: &'static str,
            pub trigger: &'static str,
            /// Raw JSON bytes of the criterion's `conditions` object (may be `b"null"` if absent).
            pub conditions: &'static [u8],
        }

        /// Static advancement definition compiled from the datapack JSON at build time.
        pub struct StaticAdvancementDef {
            pub id: &'static str,
            pub parent: Option<&'static str>,
            pub display: Option<StaticDisplayDef>,
            /// AND-of-OR requirements: each outer group must have ≥1 criterion satisfied.
            pub requirements: &'static [&'static [&'static str]],
            pub criteria: &'static [StaticCriterionDef],
            pub sends_telemetry: bool,
        }

        /// All 1617 vanilla advancement definitions, sorted lexicographically by ID.
        /// Positions (x, y) are pre-computed by the Reingold-Tilford layout algorithm.
        pub static VANILLA_ADVANCEMENTS: &[StaticAdvancementDef] = &[
            #(#node_tokens),*
        ];

        /// Total number of vanilla advancements.
        pub const VANILLA_ADVANCEMENT_COUNT: usize = #def_count;
    }
}

/// Recursively collect all advancement JSON files under `dir`.
/// `base` is the root advancement directory; `id` is derived from the relative path.
fn collect_advancements(
    base: &Path,
    dir: &Path,
    out: &mut Vec<(String, AdvancementJson)>,
) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_advancements(base, &path, out);
        } else if path.extension().is_some_and(|e| e == "json") {
            // Derive the advancement ID from the relative path
            // e.g. "adventure/kill_a_mob.json" → "minecraft:adventure/kill_a_mob"
            let rel = path.strip_prefix(base).unwrap();
            let id_path = rel.with_extension("");
            let id = format!(
                "minecraft:{}",
                id_path.to_string_lossy().replace('\\', "/")
            );

            let json_str = match fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("WARNING: could not read {}: {e}", path.display());
                    continue;
                }
            };

            match serde_json::from_str::<AdvancementJson>(&json_str) {
                Ok(adv) => out.push((id, adv)),
                Err(e) => eprintln!("WARNING: could not parse {}: {e}", path.display()),
            }
        }
    }
}

// Suppress unused import warning in some configurations
#[allow(unused_imports)]
use io::Write as _;
