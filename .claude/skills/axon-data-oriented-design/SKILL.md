---
name: axon-data-oriented-design
description: axon's data-oriented design rules and the questions to ask of every data structure - flat arrays instead of hash tables, indices instead of pointers, hot data packed in index bits, struct-of-arrays vs array-of-structs, no caches, own implementations instead of dependencies. Use it before adding or changing any data structure, loop over data, dependency or "cache".
---

# Data-oriented design in axon

axon exists to use less RAM and CPU than other terminals, and to reinvent the wheel to get there:
a dependency is scaffolding with an exit plan ([0010](../../../knowledge/decisions/0010-own-implementations.md)),
and a cache is a symptom, not a feature ([0009](../../../knowledge/decisions/0009-data-oriented-no-caches.md)).
The data layout is the design; the code is what walks it.

## Rules
1. **No caches.** Do not add one; when you find one, redesign the data so the work is cheap or unnecessary.
   See "What counts as a cache" below.
2. **No hash tables on a hot path.** A key with a small dense domain is an array index. A sparse domain is a
   two-level table (page directory, then page). A key that is a position *is* the index.
3. **Handles are indices, not pointers.** `u32` (or smaller) into an array: half the size of a pointer, relocatable,
   bounds checked in one place, trivially serializable. No `Box`/`Rc`/`Arc` per item.
4. **Pack the hot data into the bits of the index/handle.** Flags, kind, style and generation travel with the
   handle so the common case needs no second load. `Cell.tf`: tile in the low 16 bits, underline/strike/cursor
   shape in the high 16.
5. **Zero means empty.** Make the all-zero value a valid "nothing here" so arrays start cleared (`calloc`
   pages that are never touched cost no RSS) and "absent" needs no `Option`/branch.
6. **Fixed stride beats variable length.** If every row/cell/item has the same size, offsets are arithmetic and a
   partial update is a byte range. Variable-length data gets a flat pool plus `(start, len)` indices.
7. **Store the minimum, derive the rest.** The position of a cell is its index; it is not stored.
8. **One representation for producer and consumer.** Every conversion pass (`Term` cell to GPU cell) is a copy
   to remove one day. Prefer to make the producer write the consumer's layout.
9. **Lifetimes are explicit.** Frame scratch is reused (`clear`, never reallocated); session data lives in
   arrays owned by the session; nothing allocates per item.

## Questions to ask (answer them in the code review, the ADR or the commit message)
- What is the access pattern: which code reads which fields, how often, in what order?
- Can this hash table be a flat array? What is the key domain: dense, sparse, or a position?
- Can this pointer, reference or `Box` be an index into an array?
- Which bits of the index can carry the most frequently used data?
- Is this a cache? What flaw in the design is it hiding, and what redesign makes it unnecessary?
- Should this be a struct of arrays instead of an array of structs? (below)
- Can the variable-length part become fixed-stride, with offsets computed instead of stored?
- Which fields are derivable and should not be stored at all?
- How many bytes per item? Would `u8`/`u16` do? Does the hot part fit one cache line?
- Can the layout remove a branch (sentinel value, zero = empty, sorted order)?
- What is the lifetime, and what is the per-frame allocation count? (must be 0)
- Do we need this data structure, or this dependency, at all? What would it take to write it ourselves?

## Struct of arrays or array of structs?
- **SoA** when different loops or passes touch different subsets of the fields (VT parser reading only
  characters, damage scan reading only a dirty bit, selection reading only positions), or when a field is
  rarely used and would waste cache lines in every item.
- **AoS** when every consumer reads all the fields of one item together. The GPU cell (`tf`, `fg`, `bg`) is
  fetched whole as vertex attributes, so interleaved 12-byte cells are right.
- Decide from the access pattern, then measure ([axon-performance](../axon-performance/SKILL.md)).
  Neither is the default.

## What counts as a cache
A cache is a stored result of a computation that could be recomputed, kept to avoid recomputing it. The test:
does it have a **miss path**, can it go **stale**, does it need **invalidation or eviction**? If yes, it is a
cache: redesign instead.

Not caches:
- authoritative state (the terminal grid);
- dirty bits and damage over authoritative state: the protocol that says which rows to rewrite
  ([0005](../../../knowledge/decisions/0005-damage-tracking.md));
- a table baked once up front by a deterministic pass, with no miss path (an atlas of every glyph of a font,
  tile = glyph id);
- scratch that is dead when the call returns (`Gpu.stage`).

## Current state (keep it up to date; one row per structure)
| Structure | Verdict | Redesign | Status |
|---|---|---|---|
| Per-row instance vectors + flat mirror of the GPU buffer | cache | Fixed-stride cell buffer indexed by `row*cols+col`, written per damaged row run | done 2026-11-22 |
| Extra instances for underline, strike and cursor | wasted entries | Flags in `Cell.tf` drawn by the shader: always one cell per position | done 2026-11-22 |
| `HashMap<(char, u8), u32>` glyph map | hash table | Two-level flat table (`GlyphTable`); still baked lazily, so cache-like | partial |
| Lazy-baked glyph atlas | cache | Own font parser, eager bake of the primary font, tile = glyph id, no lookup | pending |
| Lazy fallback fonts (`OnceCell`) | memoized load | Memory-mapped font, tables read in place, nothing parsed up front | pending |
| `fontdue` (parses every glyph on load: 45 MB, CJK 314 MB) | dependency | Own TrueType/CFF reader and rasterizer | pending |
| `alacritty_terminal` `Term` (AoS cells, converted to GPU cells every frame) | dependency + mirror | Own VT parser and grid: SoA arrays the GPU can read directly | pending |
| `Rgba16Float` swapchain (8 bytes per pixel, chosen by accident) | oversized | `Bgra8Unorm` if the look allows | pending |
| `dirty`, `last_cursor`, `last_sel` in `State` | damage protocol | Keep | n/a |

Plan and order: `knowledge/plan/own-implementations.md`.

## Process
1. Write the access pattern before the structure.
2. Answer the questions above; if the answer is "a cache", stop and redesign.
3. Measure before and after (RSS, `build()` time, throughput) and log it in `knowledge/performance/measurements.md`.
4. A renderer refactor must be pixel-exact: dump the swapchain to raw RGBA before and after on fixed scenes
   (blocks, beam and underline cursors, hollow cursor, hidden cursor, wide characters, every SGR attribute) and
   diff the bytes; also compare a long incremental run against a full rebuild every frame. Do this with
   temporary code in a copy of the project under `/tmp`, never in the repo (surface usage needs `COPY_SRC`;
   the swapchain format may not be 4 bytes per pixel).
5. Update the table above and the knowledge tree.
