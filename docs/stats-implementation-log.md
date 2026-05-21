# Statistics Implementation Log

## What was implemented

### Protocol (`steel-protocol`)

**`c_award_stats.rs`** — New clientbound packet.
- `StatEntry { stat_type: u8, entry_id: u32, value: i32 }`
- `CAwardStats { stats: Vec<StatEntry> }` with manual `WriteTo` (VarInt triple per entry)
- Exported from `packets/game/mod.rs`

### Core (`steel-core/src/player/stats/`)

**`custom_stat.rs`** — `CustomStat` enum, 77 variants, `#[repr(u32)]`.
- Registry order matches vanilla `Stats.java` exactly (discriminant == protocol entry_id)
- `const ALL: [Self; 77]` for safe iteration without unsafe
- `identifier(self) -> Identifier` for JSON serialization
- `from_identifier(id: &str) -> Option<Self>` for JSON loading

**`mod.rs`** — `PlayerStats` struct.
- 8 `FxHashMap<usize, i32>` fields keyed by registry ID: `blocks_mined`, `items_crafted`, `items_used`, `items_broken`, `items_picked_up`, `items_dropped`, `entities_killed`, `entities_killed_by`
- `[i32; 77]` flat array for custom stats (O(1) access by discriminant)
- `FxHashSet<(u8, u32)>` dirty set tracking `(stat_type, entry_id)` pairs
- Increment/set/get methods for all 9 stat types
- `mark_all_dirty()` — marks all non-zero stats dirty (called on join to push full snapshot)
- `flush_dirty(&Player)` — drains dirty set, sends `CAwardStats`, no-op if empty
- `to_json(&StatsRegistry) -> Value` / `from_json(&Value, &StatsRegistry) -> Self` — vanilla-compatible JSON format (`{"stats": {"minecraft:mined": {...}, ...}, "DataVersion": 4325}`)
- `StatsRegistry` trait decoupling ID<->identifier lookups from concrete registry types
- `GlobalStatsRegistry` unit struct implementing `StatsRegistry` via the global `REGISTRY`

### Wiring

| Location | What was added |
|---|---|
| `player/mod.rs` | `pub stats: SyncMutex<PlayerStats>` field; flush in `tick()`; `mark_all_dirty()` on `RequestStats` client command |
| `player_data_storage.rs` | `load_stats(domain, uuid)` reads `{domain}/stats/{uuid}.json`; `save_stats(domain, uuid, &Value)` writes it |
| `server/mod.rs` `add_player()` | Load stats, `mark_all_dirty()`, so first tick flushes full snapshot to client |
| `server/mod.rs` `process_domain_switch()` | Save stats for source domain; load stats for target domain |
| `world/world_entities.rs` `remove_player()` | Save stats on disconnect |
| `crafting_table_block.rs` | `InteractWithCraftingTable` incremented on use |
| `barrel_block.rs` | `OpenBarrel` incremented on open |

### Storage format

Vanilla-compatible JSON so vanilla tools (Minecraft itself, third-party stat viewers) can read the files:
```
{save_root}/{domain}/stats/{uuid}.json
```
```json
{
  "stats": {
    "minecraft:mined": { "minecraft:stone": 42 },
    "minecraft:custom": { "minecraft:play_time": 1234 }
  },
  "DataVersion": 4325
}
```

---

## What is NOT implemented yet — and why

See `stats-advancements-design.md` for the full design. Short answer: advancements depend on too many systems that don't exist yet.

### Missing foundations

**1. Build-time advancement JSON compilation**
1617 advancement JSON files from the datapack need to be compiled into typed Rust data at build time (per CLAUDE.md: no runtime JSON parsing). This alone is a significant build script effort — the extractor needs to be updated or a dedicated builder written.

**2. `CUpdateAdvancements` packet**
The packet is complex: it sends a tree of advancement descriptors (display info, criteria names, requirements) plus per-player progress (granted criteria timestamps), plus a set of removed advancements. Multiple new types needed (`AdvancementDisplay`, `AdvancementProgress`, `CriterionProgress`, etc.) with non-trivial wire encoding.

**3. Criterion trigger infrastructure**
50 trigger types (`CriteriaTriggers.java`). Each trigger needs:
- A typed Rust enum variant
- Callsites throughout the codebase (block break, entity kill, item use, location check, etc.)
- Per-player subscription tracking (which advancements are listening for which trigger)

**4. `PlayerAdvancements` per-player state machine**
Vanilla's `PlayerAdvancements.java` is ~600 lines. It manages:
- Advancement tree visibility (which ones are visible/hidden to the client)
- Progress per criterion per advancement
- Award logic (checking all criteria satisfied → grant advancement → trigger rewards)
- Tab selection (`CSelectAdvancementsTab`)
- `SSeenAdvancements` serverbound packet handler

**5. Advancement rewards**
Granting an advancement triggers: experience rewards, loot table rewards, recipe unlocks, and the "fireworks + chat message" announcement. Recipe unlocks alone depend on a recipe registry that isn't exposed to advancements yet.

**6. Missing callsites**
Many trigger callsites are in systems not yet implemented or partially implemented (entity kills, item crafting, location-based triggers require chunk/world access patterns that aren't fully wired).

### Summary

Statistics were straightforward: one packet, one data structure, dirty tracking, JSON I/O. Advancements are a full subsystem with ~5 interdependent pieces, none of which exist yet. Implementing advancements correctly without the foundations would require stubs and `todo!()` — which CLAUDE.md explicitly forbids.
