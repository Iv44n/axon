//! The user's settings: `$XDG_CONFIG_HOME/axon/config.toml` (or `~/.config/axon/config.toml`), a small
//! subset of TOML read by our own parser: `[section]` headers, `key = value` lines, `#` comments,
//! strings in double quotes, numbers and `true` / `false`. Nothing in it is required; a mistake is
//! reported with its line number and skipped, never fatal. The environment variables
//! (`AXON_FONT`, `AXON_FONT_SIZE`, `AXON_OPACITY`) still win over the file.
use std::path::PathBuf;

use crate::context;

/// How the grid's text looks. Sizes are in pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct FontCfg {
    /// A font file (`/path/to/font.ttf`, `~` allowed) or a family name found in the font directories.
    pub family: Option<String>,
    pub size: f32,
    /// 100 (thin) to 900 (black); 400 is the font as drawn. Synthesized until the own font reader reads
    /// the variable axis (task 12): heavier smears the glyph sideways, lighter thins its edges.
    pub weight: u16,
    /// Draw bold text (SGR 1) heavier: `weight + 300`. When false, bold text is drawn as regular.
    pub bold: bool,
    /// Extra pixels between cells, at `size`; negative packs the text tighter. Whole pixels in the end.
    pub letter_spacing: f32,
    /// Row height as a multiple of the font's own (ascent + descent).
    pub line_height: f32,
    /// How glyph edges are smoothed: grey levels, or per-subpixel (LCD) coverage in the panel's order.
    pub smoothing: Smoothing,
    /// Snap the glyphs' baseline, x-height and cap height to whole pixel rows, so horizontal strokes are crisp.
    pub hinting: bool,
}

/// Antialiasing of the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Smoothing {
    /// One coverage level per pixel.
    Gray,
    /// LCD: red, green and blue subpixels left to right (what almost every panel is).
    Rgb,
    /// LCD with the subpixels the other way round.
    Bgr,
}

impl Smoothing {
    pub fn name(self) -> &'static str {
        match self {
            Smoothing::Gray => "gray",
            Smoothing::Rgb => "rgb",
            Smoothing::Bgr => "bgr",
        }
    }
}

impl Default for FontCfg {
    fn default() -> Self {
        FontCfg {
            family: None,
            size: 15.0,
            weight: 400,
            bold: true,
            letter_spacing: 0.0,
            line_height: 1.0,
            smoothing: Smoothing::Rgb,
            hinting: true,
        }
    }
}

impl FontCfg {
    /// The defaults at another size (the settings screen's own text, the design board, the tests).
    pub fn sized(size: f32) -> Self {
        FontCfg { size, ..FontCfg::default() }
    }
}

/// A piece of context the input bar can show, in the order the settings file lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chip {
    Time,
    User,
    /// The conda environment or Python virtual environment the shell is in.
    Env,
    Pyenv,
    /// `user@host`.
    Host,
    Dir,
    Branch,
    /// Files changed and lines added and removed, from `git diff HEAD` (run by the shell's hook).
    Changes,
    Kube,
}

impl Chip {
    pub const COUNT: usize = 9;
    /// Every chip, in the default order.
    pub const ALL: [Chip; Chip::COUNT] =
        [Chip::Time, Chip::User, Chip::Env, Chip::Pyenv, Chip::Host, Chip::Dir, Chip::Branch, Chip::Changes, Chip::Kube];

    /// The name used in the settings file.
    pub fn name(self) -> &'static str {
        ["time", "user", "env", "pyenv", "host", "dir", "branch", "changes", "kube"][self as usize]
    }

    pub fn from_name(s: &str) -> Option<Chip> {
        Chip::ALL.into_iter().find(|c| c.name() == s)
    }
}

/// What sits under the grid.
#[derive(Clone, Debug, PartialEq)]
pub struct InputCfg {
    /// Draw the chips on the line above the prompt (`mode = "chips"`) or leave the prompt to the shell
    /// (`"shell"`, the default).
    pub inline: bool,
    /// The chips shown, left to right. One that has nothing to say (no git repository, no
    /// environment) is skipped.
    pub chips: Vec<Chip>,
    /// 24-hour clock; false is `03:48 pm`.
    pub clock24: bool,
    /// The cursor's shape until a program asks for another (`DECSCUSR`).
    pub cursor: CursorKind,
    /// Suggest the rest of a command while it is typed (`autosuggest`). Parsed, saved and shown in the settings
    /// panel; the line editor that draws the suggestion comes with the own input (task 6a, phase 2).
    pub autosuggest: bool,
}

impl Default for InputCfg {
    fn default() -> Self {
        InputCfg { inline: false, chips: Chip::ALL.to_vec(), clock24: false, cursor: CursorKind::Block, autosuggest: true }
    }
}

/// The shape of the cursor: a block, a bar before the character, or a line under it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorKind {
    #[default]
    Block,
    Bar,
    Underline,
}

impl CursorKind {
    pub const ALL: [CursorKind; 3] = [CursorKind::Block, CursorKind::Bar, CursorKind::Underline];

    pub fn name(self) -> &'static str {
        match self {
            CursorKind::Block => "block",
            CursorKind::Bar => "bar",
            CursorKind::Underline => "underline",
        }
    }
}

/// Where the list of windows lives: a panel at the left or the right, or tabs at the top.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabsPos {
    #[default]
    Left,
    Right,
    Top,
}

impl TabsPos {
    pub const ALL: [TabsPos; 3] = [TabsPos::Left, TabsPos::Right, TabsPos::Top];

    pub fn name(self) -> &'static str {
        match self {
            TabsPos::Left => "left",
            TabsPos::Right => "right",
            TabsPos::Top => "top",
        }
    }
}

/// The windows (tabs) and their panel (`[windows]`).
#[derive(Clone, Debug, PartialEq)]
pub struct WindowsCfg {
    pub tabs: TabsPos,
    /// The panel is shown (`Alt+B` toggles it); with the tabs at the top it does not exist.
    pub side: bool,
    /// The panel's width in pixels.
    pub side_w: u32,
    /// Every window's shell starts in a systemd scope of its own (`session::spawn`, ADR 0038), so what runs from a
    /// window is accounted, and killed under memory pressure, apart from the other windows. It only takes effect
    /// where systemd's user manager answers; elsewhere the shell starts as it always did.
    pub scope: bool,
}

impl Default for WindowsCfg {
    fn default() -> Self {
        WindowsCfg { tabs: TabsPos::Left, side: true, side_w: 220, scope: true }
    }
}

/// The side panel's width limits, in pixels. The settings panel's box and slider use the same limits.
pub const SIDE_W: (f32, f32) = (160.0, 320.0);

/// Every field's default is its type's (the status bar is off, as in the reference).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    pub font: FontCfg,
    pub input: InputCfg,
    pub windows: WindowsCfg,
    /// Alpha of the glass body, 0.3 to 1; `None` keeps the backend's default.
    pub opacity: Option<f32>,
    /// Ask the compositor to blur what is behind the glass (`[window] blur`). Only compositors that offer
    /// it (KDE, and those with `ext-background-effect-v1`) do; GNOME's Mutter does not.
    pub blur: bool,
    /// Show the status bar at the bottom of the window (`[window] status_bar`). Off unless asked: the reference has none.
    pub status_bar: bool,
    /// The language of the interface (`[ui] language`): the system's, English or Spanish.
    pub lang: crate::i18n::Setting,
}

/// Where the file is expected, whether it exists or not.
pub fn path() -> Option<PathBuf> {
    let dir = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
    };
    Some(dir.join("axon").join("config.toml"))
}

/// Reads the file and the environment. Problems are printed to stderr, once each.
pub fn load() -> Config {
    let mut warnings = Vec::new();
    let mut cfg = match path().map(|p| (std::fs::read_to_string(&p), p)) {
        Some((Ok(text), _)) => parse(&text, &mut warnings),
        Some((Err(e), p)) if e.kind() != std::io::ErrorKind::NotFound => {
            warnings.push(format!("{}: {e}", p.display()));
            Config::default()
        }
        _ => Config::default(),
    };
    env_overrides(&mut cfg);
    for w in &warnings {
        eprintln!("axon: config: {w}");
    }
    cfg
}

fn env_overrides(cfg: &mut Config) {
    let num = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<f32>().ok());
    if let Some(s) = num("AXON_FONT_SIZE") {
        cfg.font.size = s.clamp(SIZE.0, SIZE.1);
    }
    if let Some(o) = num("AXON_OPACITY") {
        cfg.opacity = Some(o);
    }
    if let Some(f) = std::env::var("AXON_FONT").ok().filter(|v| !v.is_empty()) {
        cfg.font.family = Some(f);
    }
}

/// Returns `text` with `section.key` set to `value` (`None` comments the setting out), keeping every
/// other line and comment. An active `key = ...` line is rewritten in place (its trailing comment
/// stays); else the template's commented `# key = ...` line is turned into the setting; else a new
/// line goes at the end of the section, which is created when missing.
pub fn set_key(text: &str, section: &str, key: &str, value: Option<&str>) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut current = String::new();
    // (line of the active setting, line of its commented template, last line of the section)
    let (mut active, mut commented, mut last) = (None, None, None);
    for (i, raw) in lines.iter().enumerate() {
        let line = strip_comment(raw).trim();
        if let Some(name) = line.strip_prefix('[').and_then(|n| n.strip_suffix(']')) {
            current = name.trim().to_string();
            if current == section {
                last = Some(i);
            }
            continue;
        }
        if current != section {
            continue;
        }
        if !line.is_empty() {
            last = Some(i);
        }
        match line.split_once('=') {
            Some((k, _)) if k.trim() == key => active = active.or(Some(i)),
            None if line.is_empty() => {
                let c = raw.trim().strip_prefix('#').map(str::trim_start).unwrap_or("");
                if c.split_once('=').is_some_and(|(k, _)| k.trim() == key) {
                    commented = commented.or(Some(i));
                }
            }
            _ => {}
        }
    }
    match (active, value) {
        (Some(i), Some(v)) => {
            let tail = lines[i][strip_comment(&lines[i]).len()..].to_string();
            lines[i] = format!("{key} = {v}{}{tail}", if tail.is_empty() { "" } else { "  " });
        }
        (Some(i), None) => lines[i] = format!("# {}", lines[i].trim_start()),
        (None, None) => {}
        (None, Some(v)) => {
            let new = format!("{key} = {v}");
            match (commented, last) {
                (Some(i), _) => lines[i] = new,
                (None, Some(i)) => lines.insert(i + 1, new),
                (None, None) => {
                    if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                        lines.push(String::new());
                    }
                    lines.push(format!("[{section}]"));
                    lines.push(new);
                }
            }
        }
    }
    lines.join("\n") + "\n"
}

/// Writes `changes` (`"font.size"`, new value) into the settings file, starting from the template
/// when there is no file yet. Written to a temporary file and renamed, so a crash cannot leave half
/// a file.
pub fn save(changes: &[(&str, Option<String>)]) -> std::io::Result<PathBuf> {
    let path = path().ok_or_else(|| std::io::Error::other("neither XDG_CONFIG_HOME nor HOME is set"))?;
    save_at(&path, changes)?;
    Ok(path)
}

fn save_at(path: &std::path::Path, changes: &[(&str, Option<String>)]) -> std::io::Result<()> {
    let mut text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => TEMPLATE.to_string(),
        Err(e) => return Err(e),
    };
    for (full, value) in changes {
        let (section, key) = full.split_once('.').unwrap_or(("", full));
        text = set_key(&text, section, key, value.as_deref());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// The commented file `axon config` writes: every setting, all commented out.
const TEMPLATE: &str = include_str!("../assets/config.example");

/// Creates the file with the template unless it exists (an existing file is never touched).
/// Returns whether it was created.
fn init_at(path: &std::path::Path) -> std::io::Result<bool> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    match std::fs::OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut f) => f.write_all(TEMPLATE.as_bytes()).map(|()| true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e),
    }
}

/// The settings in effect, one per line, in the file's own syntax; what has no value yet (the
/// default font, the backend's own opacity) is a comment.
fn describe(c: &Config) -> String {
    let f = &c.font;
    let family = f.family.as_ref().map_or("# family: the default list (JetBrains Mono first)".to_string(), |v| format!("family = {v:?}"));
    let opacity = c.opacity.map_or("# opacity: the backend's default (0.88)".to_string(), |o| format!("opacity = {o}"));
    format!(
        "[font]\n{family}\nsize = {}\nweight = {}\nbold = {}\nletter_spacing = {}\nline_height = {}\nsmoothing = {:?}\nhinting = {}\n\n[window]\n{opacity}\nblur = {}\nstatus_bar = {}\n\n[ui]\nlanguage = {:?}\n\n\
         [input]\nmode = {:?}\nclock = {:?}\ncursor = {:?}\nautosuggest = {}\nchips = {}\n\n\
         [windows]\ntabs = {:?}\nside = {}\nside_width = {}\nscope = {}\n",
        f.size,
        f.weight,
        f.bold,
        f.letter_spacing,
        f.line_height,
        f.smoothing.name(),
        f.hinting,
        c.blur,
        c.status_bar,
        c.lang.name(),
        if c.input.inline { "chips" } else { "shell" },
        if c.input.clock24 { "24h" } else { "12h" },
        c.input.cursor.name(),
        c.input.autosuggest,
        chip_list_text(&c.input.chips),
        c.windows.tabs.name(),
        c.windows.side,
        c.windows.side_w,
        c.windows.scope,
    )
}

const USAGE: &str = "usage: axon [command]

  (none)           open a terminal window
  config           create the settings file if it does not exist, and print where it is
  config path      print where the settings file is
  config show      print the settings in effect (file and environment), and any mistakes in the file
  config edit      create the file if needed and open it in $VISUAL or $EDITOR
  shell-init SHELL print what to add to your shell's rc file (bash, zsh or fish) so the prompt chips
                   work in it: eval \"$(axon shell-init bash)\" (axon starts bash with it already)
  --help, --version

Settings: edit the file, then press Ctrl+Shift+R in a running window to apply it.";

/// Handles the command line. `Some(exit code)` when it was a command and the program should stop;
/// `None` to open the window (any other argument is ignored, as a launcher may pass files or URLs).
pub fn command(args: &[String]) -> Option<i32> {
    match args.first().map(String::as_str)? {
        "--help" | "-h" | "help" => println!("{USAGE}"),
        "--version" | "-V" => println!("axon {}", env!("CARGO_PKG_VERSION")),
        "config" => return Some(config_command(args.get(1).map(String::as_str))),
        "shell-init" => return Some(shell_init(args.get(1).map(String::as_str))),
        _ => return None,
    }
    Some(0)
}

fn shell_init(shell: Option<&str>) -> i32 {
    match shell.and_then(context::shell_init_script) {
        Some(script) => print!("{script}"),
        None => {
            eprintln!("usage: axon shell-init bash|zsh|fish\n\nAdd to ~/.bashrc:  eval \"$(axon shell-init bash)\"\n       ~/.zshrc:   eval \"$(axon shell-init zsh)\"\n       config.fish: axon shell-init fish | source");
            return 2;
        }
    }
    0
}

fn config_command(sub: Option<&str>) -> i32 {
    let Some(path) = path() else {
        eprintln!("axon: cannot tell where the settings go: neither XDG_CONFIG_HOME nor HOME is set");
        return 1;
    };
    match sub {
        Some("path") => println!("{}", path.display()),
        Some("show") => {
            let mut warnings = Vec::new();
            let mut cfg = match std::fs::read_to_string(&path) {
                Ok(text) => parse(&text, &mut warnings),
                Err(_) => Config::default(),
            };
            env_overrides(&mut cfg);
            print!("# {}\n{}", path.display(), describe(&cfg));
            for w in warnings {
                eprintln!("axon: config: {w}");
            }
        }
        None | Some("init" | "edit") => {
            match init_at(&path) {
                Ok(true) => println!("created {}", path.display()),
                Ok(false) => println!("{} already exists, left as it is", path.display()),
                Err(e) => {
                    eprintln!("axon: {}: {e}", path.display());
                    return 1;
                }
            }
            if sub == Some("edit") {
                let editor = ["VISUAL", "EDITOR"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()));
                let Some(editor) = editor else {
                    eprintln!("axon: set VISUAL or EDITOR to open the file, or edit it yourself");
                    return 1;
                };
                // The variable may carry arguments ("code --wait").
                let mut parts = editor.split_whitespace();
                let status = std::process::Command::new(parts.next().unwrap_or("vi")).args(parts).arg(&path).status();
                return match status {
                    Ok(s) if s.success() => 0,
                    Ok(s) => s.code().unwrap_or(1),
                    Err(e) => {
                        eprintln!("axon: cannot run {editor:?}: {e}");
                        1
                    }
                };
            }
            if sub.is_none() {
                println!("Edit it, then press Ctrl+Shift+R in axon to apply it (`axon config show` lists what is in effect).");
            }
        }
        Some(other) => {
            eprintln!("axon: unknown config command {other:?}\n\n{USAGE}");
            return 2;
        }
    }
    0
}

/// What each number may be; anything outside is clamped. The settings panel's boxes use the same limits.
pub const SIZE: (f32, f32) = (6.0, 96.0);
pub const WEIGHT: (f32, f32) = (100.0, 900.0);
pub const SPACING: (f32, f32) = (-8.0, 32.0);
pub const LINE_HEIGHT: (f32, f32) = (0.8, 3.0);
pub const OPACITY: (f32, f32) = (0.3, 1.0);

/// Parses `text`. Every problem becomes a line in `warnings` and the offending line is ignored;
/// a number outside its range is clamped.
pub fn parse(text: &str, warnings: &mut Vec<String>) -> Config {
    let mut cfg = Config::default();
    let mut section = String::new();
    for (i, raw) in text.lines().enumerate() {
        let no = i + 1;
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[') {
            match name.strip_suffix(']') {
                Some(n) => section = n.trim().to_string(),
                None => warnings.push(format!("line {no}: unterminated section header")),
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            warnings.push(format!("line {no}: expected `key = value`"));
            continue;
        };
        let key = if section.is_empty() { key.trim().to_string() } else { format!("{section}.{}", key.trim()) };
        if let Err(e) = apply(&mut cfg, &key, value.trim()) {
            warnings.push(format!("line {no}: {key}: {e}"));
        }
    }
    cfg
}

/// Removes a trailing `# comment`, leaving a `#` inside a quoted string alone.
fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '#' if !quoted => return &line[..i],
            _ => {}
        }
    }
    line
}

fn string(v: &str) -> Result<String, String> {
    let inner = v.strip_prefix('"').and_then(|r| r.strip_suffix('"')).ok_or("expected a string in double quotes")?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some(e @ ('"' | '\\')) => out.push(e),
            _ => return Err("only \\\" and \\\\ are escapes".into()),
        }
    }
    Ok(out)
}

fn number(v: &str, (lo, hi): (f32, f32)) -> Result<f32, String> {
    let n: f32 = v.parse().map_err(|_| format!("expected a number, got `{v}`"))?;
    if !n.is_finite() {
        return Err(format!("expected a finite number, got `{v}`"));
    }
    Ok(n.clamp(lo, hi))
}

/// `["time", "dir"]`: chip names in double quotes, in the order to show them. A name that is not a
/// chip is an error for the whole line, so a typo does not silently drop a chip.
fn chip_list(v: &str) -> Result<Vec<Chip>, String> {
    let inner = v.strip_prefix('[').and_then(|r| r.strip_suffix(']')).ok_or("expected a list like [\"time\", \"dir\"]")?;
    let mut out = Vec::new();
    for item in inner.split(',').map(str::trim).filter(|i| !i.is_empty()) {
        let name = string(item)?;
        let chip = Chip::from_name(&name).ok_or_else(|| format!("unknown chip {name:?} (one of: {})", chip_names()))?;
        if !out.contains(&chip) {
            out.push(chip);
        }
    }
    Ok(out)
}

fn chip_names() -> String {
    Chip::ALL.map(Chip::name).join(", ")
}

/// The list as the file writes it.
pub fn chip_list_text(chips: &[Chip]) -> String {
    let names: Vec<String> = chips.iter().map(|c| format!("{:?}", c.name())).collect();
    format!("[{}]", names.join(", "))
}

fn boolean(v: &str) -> Result<bool, String> {
    match v {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("expected true or false, got `{v}`")),
    }
}

fn apply(cfg: &mut Config, key: &str, v: &str) -> Result<(), String> {
    match key {
        "font.family" => {
            let s = string(v)?;
            cfg.font.family = (!s.is_empty()).then_some(s);
        }
        "font.size" => cfg.font.size = number(v, SIZE)?,
        "font.weight" => cfg.font.weight = (number(v, WEIGHT)? / 10.0).round() as u16 * 10,
        "font.bold" => cfg.font.bold = boolean(v)?,
        "font.letter_spacing" => cfg.font.letter_spacing = number(v, SPACING)?,
        "font.line_height" => cfg.font.line_height = number(v, LINE_HEIGHT)?,
        "font.smoothing" => {
            cfg.font.smoothing = match string(v)?.as_str() {
                "gray" | "grey" => Smoothing::Gray,
                "rgb" => Smoothing::Rgb,
                "bgr" => Smoothing::Bgr,
                other => return Err(format!("smoothing is \"gray\", \"rgb\" or \"bgr\", not {other:?}")),
            }
        }
        "font.hinting" => cfg.font.hinting = boolean(v)?,
        "window.opacity" => cfg.opacity = Some(number(v, OPACITY)?),
        "window.blur" => cfg.blur = boolean(v)?,
        "window.status_bar" => cfg.status_bar = boolean(v)?,
        "ui.language" => {
            let name = string(v)?;
            cfg.lang = crate::i18n::Setting::from_name(&name).ok_or_else(|| format!("language is \"auto\", \"en\" or \"es\", not {name:?}"))?
        }
        "input.cursor" => {
            cfg.input.cursor = match string(v)?.as_str() {
                "block" => CursorKind::Block,
                "bar" | "beam" => CursorKind::Bar,
                "underline" | "line" => CursorKind::Underline,
                other => return Err(format!("cursor is \"block\", \"bar\" or \"underline\", not {other:?}")),
            }
        }
        "input.mode" => match string(v)?.as_str() {
            // "bar" is what the first version of the setting was called.
            "chips" | "bar" => cfg.input.inline = true,
            "shell" => cfg.input.inline = false,
            other => return Err(format!("expected \"shell\" or \"chips\", got {other:?}")),
        },
        "input.clock" => match string(v)?.as_str() {
            "12h" => cfg.input.clock24 = false,
            "24h" => cfg.input.clock24 = true,
            other => return Err(format!("expected \"12h\" or \"24h\", got {other:?}")),
        },
        "input.chips" => cfg.input.chips = chip_list(v)?,
        "input.autosuggest" => cfg.input.autosuggest = boolean(v)?,
        "windows.tabs" => {
            let name = string(v)?;
            cfg.windows.tabs = TabsPos::ALL.into_iter().find(|t| t.name() == name).ok_or_else(|| format!("tabs is \"left\", \"right\" or \"top\", not {name:?}"))?
        }
        "windows.side" => cfg.windows.side = boolean(v)?,
        "windows.side_width" => cfg.windows.side_w = number(v, SIDE_W)?.round() as u32,
        "windows.scope" => cfg.windows.scope = boolean(v)?,
        _ => return Err("unknown setting".into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> (Config, Vec<String>) {
        let mut w = Vec::new();
        (parse(text, &mut w), w)
    }

    #[test]
    fn an_empty_file_is_the_defaults() {
        assert_eq!(parsed(""), (Config::default(), vec![]));
        assert_eq!(parsed("# only a comment\n\n   \n").0, Config::default());
    }

    #[test]
    fn every_font_setting_is_read() {
        let (c, w) = parsed(
            "[font]\nfamily = \"JetBrains Mono\"  # the family\nsize = 17.5\nweight = 500\nbold = false\n\
             letter_spacing = 1\nline_height = 1.2\n[window]\nopacity = 0.9\n",
        );
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(
            c,
            Config {
                font: FontCfg {
                    family: Some("JetBrains Mono".into()),
                    size: 17.5,
                    weight: 500,
                    bold: false,
                    letter_spacing: 1.0,
                    line_height: 1.2,
                    ..FontCfg::default()
                },
                opacity: Some(0.9),
                input: InputCfg::default(),
                windows: WindowsCfg::default(),
                blur: false,
                status_bar: false,
                lang: Default::default(),
            }
        );
    }

    #[test]
    fn the_language_is_read_and_a_wrong_one_is_refused() {
        assert_eq!(parsed("[ui]\nlanguage = \"es\"\n").0.lang, crate::i18n::Setting::Es);
        assert_eq!(parsed("").0.lang, crate::i18n::Setting::Auto);
        let (c, w) = parsed("[ui]\nlanguage = \"fr\"\n");
        assert_eq!(c.lang, crate::i18n::Setting::Auto);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn the_windows_panel_is_read_and_clamped() {
        assert_eq!(parsed("").0.windows, WindowsCfg { tabs: TabsPos::Left, side: true, side_w: 220, scope: true });
        let (c, w) = parsed("[windows]\ntabs = \"right\"\nside = false\nside_width = 999\n");
        assert_eq!(c.windows, WindowsCfg { tabs: TabsPos::Right, side: false, side_w: 320, scope: true });
        assert!(w.is_empty(), "{w:?}");
        let (c, w) = parsed("[windows]\ntabs = \"bottom\"\nside_width = 10\n");
        assert_eq!((c.windows.tabs, c.windows.side_w), (TabsPos::Left, 160));
        assert_eq!(w.len(), 1, "{w:?}");
    }

    #[test]
    fn a_window_gets_its_own_scope_unless_the_file_says_not() {
        assert!(parsed("").0.windows.scope, "on by default");
        let (c, w) = parsed("[windows]\nscope = false\n");
        assert!(!c.windows.scope);
        assert!(w.is_empty(), "{w:?}");
        let (c, w) = parsed("[windows]\nscope = \"no\"\n");
        assert!(c.windows.scope, "a mistake keeps the default");
        assert_eq!(w.len(), 1, "{w:?}");
    }

    #[test]
    fn the_status_bar_and_the_cursor_are_read() {
        let (c, w) = parsed("[window]\nstatus_bar = true\n[input]\ncursor = \"bar\"\n");
        assert!(w.is_empty(), "{w:?}");
        assert!(c.status_bar);
        assert_eq!(c.input.cursor, CursorKind::Bar);
        let (c, w) = parsed("[input]\ncursor = \"star\"\n");
        assert_eq!(c.input.cursor, CursorKind::Block, "an unknown shape keeps the default");
        assert_eq!(w.len(), 1);
        assert!(!parsed("").0.status_bar, "the status bar is off by default, as in the reference");
    }

    #[test]
    fn the_input_section_is_read() {
        let (c, w) = parsed("[input]\nmode = \"bar\"\nclock = \"24h\"\nchips = [\"dir\", \"time\", \"dir\"]\n");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.input, InputCfg { inline: true, chips: vec![Chip::Dir, Chip::Time], clock24: true, ..InputCfg::default() });
        // Nothing set: the bar is off, every chip is listed, 12-hour clock.
        let d = InputCfg::default();
        assert!(!d.inline && !d.clock24 && d.chips == Chip::ALL);
        assert_eq!(parsed("input.chips = []").0.input.chips, vec![]);
    }

    #[test]
    fn autosuggest_is_on_by_default_and_is_read_saved_and_shown() {
        assert!(InputCfg::default().autosuggest);
        assert!(parsed("").0.input.autosuggest);
        let (c, w) = parsed("[input]\nautosuggest = false\n");
        assert!(w.is_empty(), "{w:?}");
        assert!(!c.input.autosuggest);
        // A word that is not a boolean is named and the default stays.
        let (c, w) = parsed("[input]\nautosuggest = \"no\"\n");
        assert!(c.input.autosuggest);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("autosuggest") && w[0].contains("true or false"), "{}", w[0]);
        // The template line is turned into the setting, not duplicated, and `config show` reads back.
        let out = set_key(TEMPLATE, "input", "autosuggest", Some("false"));
        assert_eq!(out.matches("autosuggest = ").count(), 1);
        assert!(!parsed(&out).0.input.autosuggest);
        let off = parsed(&out).0;
        assert_eq!(parsed(&describe(&off)).0, off);
        assert!(describe(&off).contains("autosuggest = false"));
    }

    #[test]
    fn input_mistakes_are_named_and_change_nothing() {
        let (c, w) = parsed("input.mode = \"warp\"\ninput.clock = 24\ninput.chips = [\"dir\", \"nope\"]\ninput.chips = \"dir\"\n");
        assert_eq!(c.input, InputCfg::default());
        assert_eq!(w.len(), 4, "{w:?}");
        assert!(w[0].contains("expected \"shell\" or \"chips\""));
        assert!(w[2].contains("unknown chip \"nope\""), "{}", w[2]);
    }

    #[test]
    fn chip_names_round_trip_through_the_list_text() {
        for c in Chip::ALL {
            assert_eq!(Chip::from_name(c.name()), Some(c));
        }
        assert_eq!(chip_list(&chip_list_text(&Chip::ALL)).unwrap(), Chip::ALL);
        assert_eq!(chip_list_text(&[Chip::Dir]), "[\"dir\"]");
    }

    #[test]
    fn shell_init_prints_a_script_per_shell_and_refuses_others() {
        for sh in ["bash", "zsh", "fish"] {
            let s = context::shell_init_script(sh).unwrap();
            assert!(s.contains("AXON_CONTEXT") && s.contains("AXON_CHIPS") && s.contains("kubeconfig="), "{sh}");
        }
        assert_eq!(shell_init(Some("bash")), 0);
        assert_eq!(shell_init(Some("tcsh")), 2);
        assert_eq!(shell_init(None), 2);
    }

    #[test]
    fn dotted_keys_work_without_a_section() {
        assert_eq!(parsed("font.size = 12\n").0.font.size, 12.0);
    }

    #[test]
    fn a_hash_inside_a_string_is_not_a_comment() {
        let (c, w) = parsed("font.family = \"My #1 Font\" # real comment\n");
        assert!(w.is_empty());
        assert_eq!(c.font.family.as_deref(), Some("My #1 Font"));
        assert_eq!(parsed("font.family = \"a\\\"b\\\\c\"").0.font.family.as_deref(), Some("a\"b\\c"));
    }

    #[test]
    fn numbers_are_clamped_and_weights_snap_to_tens() {
        let (c, w) = parsed("font.size = 500\nfont.line_height = 0\nfont.letter_spacing = -50\nfont.weight = 1000\n");
        assert!(w.is_empty());
        assert_eq!((c.font.size, c.font.line_height, c.font.letter_spacing, c.font.weight), (96.0, 0.8, -8.0, 900));
        assert_eq!(parsed("font.weight = 447").0.font.weight, 450);
    }

    #[test]
    fn mistakes_name_their_line_and_leave_the_rest_alone() {
        let (c, w) = parsed("font.size = big\n[font\nfont.bold = maybe\nnonsense\nfont.colour = 1\nfont.weight = 600\n");
        assert_eq!(c.font, FontCfg { weight: 600, ..FontCfg::default() });
        assert_eq!(w.len(), 5, "{w:?}");
        assert!(w[0].starts_with("line 1: font.size: expected a number"), "{}", w[0]);
        assert!(w[1].starts_with("line 2:"));
        assert!(w[2].starts_with("line 3: font.bold"));
        assert!(w[3].starts_with("line 4:"));
        assert!(w[4].starts_with("line 5: font.colour: unknown setting"));
    }

    #[test]
    fn strings_need_quotes_and_nan_is_refused() {
        assert!(parsed("font.family = JetBrains\n").1[0].contains("double quotes"));
        assert!(parsed("font.size = NaN\n").1[0].contains("finite"));
        assert_eq!(parsed("font.family = \"\"\n").0.font.family, None);
    }

    #[test]
    fn init_creates_the_template_once_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("axon-config-{}", std::process::id()));
        let path = dir.join("axon").join("config.toml");
        assert!(init_at(&path).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), TEMPLATE);
        std::fs::write(&path, "[font]\nsize = 20\n").unwrap();
        assert!(!init_at(&path).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[font]\nsize = 20\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_template_is_valid_and_changes_nothing_until_uncommented() {
        let (c, w) = parsed(TEMPLATE);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c, Config::default());
        // Uncommenting every setting line must parse cleanly, each to a legal value.
        let live: String = TEMPLATE
            .lines()
            .map(|l| l.strip_prefix("# ").filter(|r| r.contains(" = ")).unwrap_or(l))
            .map(|l| format!("{l}\n"))
            .collect();
        let (_, w) = parsed(&live);
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn what_show_prints_reads_back_as_the_same_settings() {
        let c = parsed("[font]\nfamily = \"JetBrains Mono\"\nweight = 600\nletter_spacing = 1.5\n[window]\nopacity = 0.9\n").0;
        // Every section away from its defaults, the windows' and the input's too (what `axon config show` prints).
        let every = parsed(
            "[font]\nsmoothing = \"gray\"\nhinting = false\nbold = false\n[window]\nblur = true\nstatus_bar = false\n[ui]\nlanguage = \"es\"\n\
             [input]\nmode = \"chips\"\nclock = \"24h\"\ncursor = \"underline\"\nautosuggest = false\nchips = [\"dir\", \"kube\"]\n\
             [windows]\ntabs = \"top\"\nside = false\nside_width = 300\nscope = false\n",
        )
        .0;
        assert_ne!(every.windows, WindowsCfg::default());
        assert_ne!(every.input, InputCfg::default());
        for c in [c, every, Config::default()] {
            let (back, warnings) = parsed(&describe(&c));
            assert_eq!(back, c);
            assert!(warnings.is_empty(), "{warnings:?}");
        }
    }

    #[test]
    fn only_known_words_are_commands() {
        let a = |s: &[&str]| command(&s.iter().map(|x| x.to_string()).collect::<Vec<_>>());
        assert_eq!(a(&[]), None);
        assert_eq!(a(&["file.txt"]), None);
        assert_eq!(a(&["--version"]), Some(0));
        assert_eq!(a(&["config", "nonsense"]), Some(2));
    }

    #[test]
    fn set_key_rewrites_in_place_and_keeps_comments() {
        let text = "# mine\n[font]\nsize = 15  # my size\nweight = 400\n\n[window]\nopacity = 0.9\n";
        let out = set_key(text, "font", "size", Some("18"));
        assert_eq!(out, "# mine\n[font]\nsize = 18  # my size\nweight = 400\n\n[window]\nopacity = 0.9\n");
        // Only the right section: `size` elsewhere is not touched.
        let both = "[a]\nsize = 1\n[font]\nsize = 2\n";
        assert_eq!(set_key(both, "font", "size", Some("3")), "[a]\nsize = 1\n[font]\nsize = 3\n");
    }

    #[test]
    fn set_key_uses_the_commented_template_line_or_adds_one() {
        let out = set_key(TEMPLATE, "font", "weight", Some("600"));
        assert_eq!(parsed(&out).0.font.weight, 600);
        assert_eq!(out.matches("weight = ").count(), 1, "the template line is replaced, not duplicated");
        // No such line: appended to the section, which may not exist yet.
        assert_eq!(set_key("[font]\nsize = 12\n\n[window]\n", "font", "bold", Some("false")), "[font]\nsize = 12\nbold = false\n\n[window]\n");
        assert_eq!(set_key("", "font", "size", Some("9")), "[font]\nsize = 9\n");
        assert_eq!(set_key("[font]\nsize = 9\n", "window", "opacity", Some("0.5")), "[font]\nsize = 9\n\n[window]\nopacity = 0.5\n");
    }

    #[test]
    fn set_key_none_comments_out_and_every_template_key_round_trips() {
        assert_eq!(set_key("[font]\nfamily = \"x\"\n", "font", "family", None), "[font]\n# family = \"x\"\n");
        assert_eq!(set_key("[font]\nsize = 1\n", "font", "weight", None), "[font]\nsize = 1\n");
        let mut text = TEMPLATE.to_string();
        for (sec, key, v) in [
            ("font", "family", "\"Fira Code\""), ("font", "size", "20"), ("font", "weight", "700"),
            ("font", "bold", "false"), ("font", "letter_spacing", "1.5"), ("font", "line_height", "1.25"),
            ("font", "smoothing", "\"bgr\""), ("font", "hinting", "false"),
            ("window", "opacity", "0.7"), ("window", "blur", "true"), ("window", "status_bar", "false"), ("ui", "language", "\"es\""), ("input", "cursor", "\"bar\""), ("input", "mode", "\"chips\""), ("input", "clock", "\"24h\""),
            ("input", "chips", "[\"dir\", \"time\"]"), ("input", "autosuggest", "false"),
            ("windows", "tabs", "\"right\""), ("windows", "side", "false"), ("windows", "side_width", "260"), ("windows", "scope", "false"),
        ] {
            text = set_key(&text, sec, key, Some(v));
        }
        let (c, w) = parsed(&text);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(
            c,
            Config {
                font: FontCfg { family: Some("Fira Code".into()), size: 20.0, weight: 700, bold: false, letter_spacing: 1.5, line_height: 1.25, smoothing: Smoothing::Bgr, hinting: false },
                opacity: Some(0.7),
                input: InputCfg { inline: true, chips: vec![Chip::Dir, Chip::Time], clock24: true, cursor: CursorKind::Bar, autosuggest: false },
                windows: WindowsCfg { tabs: TabsPos::Right, side: false, side_w: 260, scope: false },
                blur: true,
                status_bar: false,
                lang: crate::i18n::Setting::Es,
            }
        );
    }

    #[test]
    fn save_creates_from_the_template_and_updates_the_file() {
        let dir = std::env::temp_dir().join(format!("axon-save-{}", std::process::id()));
        let path = dir.join("axon").join("config.toml");
        save_at(&path, &[("font.size", Some("19".into()))]).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(parsed(&text).0.font.size, 19.0);
        assert!(text.contains("# letter_spacing = 0"), "the template's comments survive");
        save_at(&path, &[("font.size", Some("21".into())), ("font.family", Some("\"A\"".into()))]).unwrap();
        let c = parsed(&std::fs::read_to_string(&path).unwrap()).0;
        assert_eq!((c.font.size, c.font.family.as_deref()), (21.0, Some("A")));
        assert!(!path.with_extension("toml.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
