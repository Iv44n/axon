---
type: decision
status: accepted
updated: 2026-11-23
related: [../core/0010-own-implementations.md, ../core/0009-data-oriented-no-caches.md, ../../plan/next-steps.md]
---
# 0016 · Settings live in `~/.config/axon/config.toml`, read by our own parser

**Decision:** axon reads `$XDG_CONFIG_HOME/axon/config.toml` (else `~/.config/axon/config.toml`) once at startup, and again
on `Ctrl+Shift+R`. The first settings are the text's: family, size, weight, bold, letter spacing, line height, plus the
glass opacity. The parser is `src/config.rs` (about 150 lines: sections, `key = value`, `#` comments, strings, numbers,
booleans). The environment variables (`AXON_FONT`, `AXON_FONT_SIZE`, `AXON_OPACITY`) stay and win over the file.

**Why a file:** fonts are something a user sets once and keeps; environment variables do not survive a launcher
and cannot hold five values. **Why a TOML subset:** editors highlight it and the user may know it; it needs no `serde`
or `toml` crate ([0010](../core/0010-own-implementations.md)). Anything it does not read (arrays, nested tables) is a
warning with its line number, never a crash. Out-of-range numbers are clamped.

**How the settings act** (`font.rs`, all at tile-bake time, so the renderers do nothing new per frame):
- *letter_spacing* changes `cell_w` by whole pixels (it scales with zoom) and shifts the glyph by half the change,
  so the text stays centred in its cell. *line_height* scales `cell_h` and moves the baseline by half the change.
- *weight* is **synthesized**: `fontdue` cannot select a variable-font instance, so above 400 the coverage is smeared
  one pixel to the right by `(weight - 400) / 300` (700 is the one-pixel bold axon always had, 900 is two) and below
  400 it is raised to a power, which thins the soft edges. Bold text (SGR 1) is `weight + 300`; `bold = false` draws
  it regular. The own font reader (task 12) replaces this with the real `wght` axis.
- *family* is a path (`/`, `~/`) or a name matched against file names in the font directories (letters and digits
  only, so "JetBrains Mono" finds `JetBrainsMono[wght].ttf`; italics and non-regular styles are passed over). A family
  that is not found is reported and the default list is used. Names are matched by file name, not by the font's own
  name table: no `fontconfig`, no new dependency.

**From the terminal:** `axon config` writes the file from `assets/config.example` (embedded with `include_str!`, so the
binary needs no data files) and never overwrites an existing one; `config edit` opens it in `$VISUAL` / `$EDITOR`,
`config show` prints the settings in effect and the file's mistakes, `config path` prints the path. A test keeps the
template valid: it parses clean as shipped and with every line uncommented.

**Reload:** `wlapp.rs` rebuilds the fonts from the new file, clears the glyph tiles and relays the grid out. The
`winit` fallback reads the file at startup only.

**Rejected:**
- *A settings window or an in-terminal menu:* judged too much for five values at the time; the user then asked for a
  screen, which is [0017](0017-settings-screen.md) (inside the window, on top of this file).
- *Writing the file on first start without being asked:* a terminal that creates files on its own is surprising; the
  user runs `axon config`.
- *A file watcher (inotify):* a second fd in the event loop for something a user does a few times; one key does it.
- *Asking `fontconfig` for families:* a dependency and a process for what a directory walk does at startup.

**Revisit if:** colours, shortcuts or themes become settings (the parser then needs lists), or the own font reader can
list family names from the `name` table.
