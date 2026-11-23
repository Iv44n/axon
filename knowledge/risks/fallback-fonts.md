---
type: risk
status: open
updated: 2026-11-22
related: [../decisions/0002-text-fontdue-atlas.md, ../performance/measurements.md]
---
# Fallback fonts

**Risk 1, portability:** `gpu.rs` looks for the primary font in `AXON_FONT`,
`~/.local/share/fonts` and hardcoded Debian/Ubuntu paths. Fallback fonts (DejaVu, Noto
Symbols, Noto CJK) are only looked up in hardcoded paths (`/usr/share/fonts/...`). On other
distributions, macOS or Windows they are not found. `fontdue` does not render color emoji.

**Risk 2, cost of a font:** `fontdue` parses every glyph when a font is loaded. Measured
2026-11-22: Cascadia Mono NF 45 MB, Noto Sans CJK 314 MB and 624 ms. Loading all fonts at
startup (and again on every scale change) made idle RSS ~900 MB. Now fallbacks load on first
use, the CJK font is only consulted for CJK code points, and a size change re-reads nothing.
What remains: **the first CJK glyph blocks the UI thread ~600 ms and adds ~300 MB** (hard
rule 1), and symbols that only the CJK font covered now render blank.

**Impact:** missing glyphs outside Debian/Ubuntu; emoji in monochrome or missing; a one-time
stall and a RAM jump when CJK text first appears.

**Mitigation options:**
- Load the CJK font on a background thread and draw blanks until it is ready (keeps fontdue).
- Replace the rasterizer with one that parses lazily (e.g. `ab_glyph`/`ttf-parser`): needs a new
  ADR superseding [0002](../decisions/0002-text-fontdue-atlas.md) and a dependency check.
- Discover fonts through fontconfig / system APIs, weighing hard rule #2. Color emoji through a
  second RGBA atlas.
