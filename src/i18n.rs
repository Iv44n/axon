//! The interface in English and Spanish: every string the screens draw lives here, once per language,
//! in one list, so a string cannot exist in one language and be missing in the other (the macro builds
//! both tables from the same lines, and a test checks them).
//!
//! The language in use is one process-wide byte, set from the settings (`set`); `tr` is a table lookup
//! with no allocation. `[ui] language` is `auto` (from `LC_ALL`, `LC_MESSAGES` or `LANG`), `en` or `es`.
//! The config file, the command line and the shell hooks stay in English: they are for tools and for
//! people who read documentation; this is what the window says.
use std::sync::atomic::{AtomicU8, Ordering};

/// A language the interface is written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Es,
}

/// What `[ui] language` can say: a language, or "the system's".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Setting {
    #[default]
    Auto,
    En,
    Es,
}

impl Setting {
    pub const ALL: [Setting; 3] = [Setting::Auto, Setting::En, Setting::Es];

    /// As the file writes it.
    pub fn name(self) -> &'static str {
        match self {
            Setting::Auto => "auto",
            Setting::En => "en",
            Setting::Es => "es",
        }
    }

    pub fn from_name(s: &str) -> Option<Setting> {
        Setting::ALL.into_iter().find(|v| v.name() == s.to_ascii_lowercase())
    }

    /// The language this setting means given the environment's locale variables (in order of priority).
    pub fn resolve(self, env: &[Option<String>]) -> Lang {
        match self {
            Setting::En => Lang::En,
            Setting::Es => Lang::Es,
            Setting::Auto => {
                let locale = env.iter().flatten().find(|v| !v.is_empty()).map(|v| v.to_ascii_lowercase());
                match locale.as_deref() {
                    Some(l) if l.starts_with("es") => Lang::Es,
                    _ => Lang::En,
                }
            }
        }
    }

    /// `resolve` against the real environment.
    pub fn resolve_here(self) -> Lang {
        let var = |k| std::env::var(k).ok();
        self.resolve(&[var("LC_ALL"), var("LC_MESSAGES"), var("LANG")])
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// Makes `l` the language of every string from now on.
pub fn set(l: Lang) {
    CURRENT.store(l as u8, Ordering::Relaxed);
}

pub fn current() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == 1 { Lang::Es } else { Lang::En }
}

macro_rules! strings {
    ($($key:ident: $en:expr, $es:expr;)*) => {
        /// Every string of the interface.
        #[derive(Clone, Copy, Debug)]
        pub enum Str { $($key),* }
        const EN: &[&str] = &[$($en),*];
        const ES: &[&str] = &[$($es),*];
    };
}

strings! {
    SettingsTitle: "Settings", "Ajustes";
    Windows: "Windows", "Ventanas";
    GroupName: "group", "grupo";
    Pane: "Pane", "Panel";
    HintFont: "drag, or click a number to type", "arrastra, o clic en un número para escribirlo";
    HintInput: "click to choose, drag to reorder", "clic para elegir, arrastra para reordenar";
    TabFont: "Font", "Fuente";
    TabInput: "Input", "Entrada";
    RowFont: "Font", "Fuente";
    RowSize: "Size", "Tamaño";
    RowWeight: "Weight", "Peso";
    RowBold: "Bold text", "Texto en negrita";
    RowSpacing: "Letter spacing", "Espaciado";
    RowLine: "Line height", "Altura de línea";
    RowOpacity: "Opacity", "Opacidad";
    RowBlur: "Blur", "Desenfoque";
    RowLanguage: "Language", "Idioma";
    RowMode: "Input type", "Tipo de entrada";
    RowClock: "Clock", "Reloj";
    RowCursor: "Cursor", "Cursor";
    RowChips: "Chips", "Chips";
    OptShell: "Shell prompt", "Prompt del shell";
    OptChips: "Prompt chips", "Chips de prompt";
    Opt12: "12-hour", "12 horas";
    Opt24: "24-hour", "24 horas";
    OptBlock: "Block", "Bloque";
    OptBar: "Bar", "Barra";
    OptUnderline: "Underline", "Subrayado";
    OptAuto: "Auto", "Auto";
    OptEnglish: "English", "English";
    OptSpanish: "Español", "Español";
    On: "on", "activado";
    Off: "off", "desactivado";
    OnUnsupported: "on, not offered here", "activado, no disponible aquí";
    OffUnsupported: "off, not offered here", "desactivado, no disponible aquí";
    Note1: "The chips replace your prompt's text with a `$`.", "Los chips reemplazan el texto de tu prompt por un `$`.";
    Note2: "Drag a chip to reorder it, or Alt+Left/Right.", "Arrastra un chip para reordenarlo, o Alt+Izq/Der.";
    Note3: "zsh and fish: add `axon shell-init` to the rc file.", "zsh y fish: añade `axon shell-init` al archivo rc.";
    Reset: "Reset", "Restablecer";
    Cancel: "Cancel", "Cancelar";
    Save: "Save", "Guardar";
    Unsaved: "Unsaved changes", "Cambios sin guardar";
    DefaultFont: "Default (JetBrains Mono)", "Por defecto (JetBrains Mono)";
    NotSaved: "not saved: ", "no guardado: ";
    ChipTime: "time", "hora";
    ChipUser: "user", "usuario";
    ChipEnv: "env", "entorno";
    ChipPyenv: "pyenv", "python";
    ChipHost: "host", "host";
    ChipDir: "dir", "carpeta";
    ChipBranch: "branch", "rama";
    ChipChanges: "changes", "cambios";
    ChipKube: "kube", "kube";
    HintHistory: "history", "historial";
    HintComplete: "complete", "completar";
    HintClear: "clear", "limpiar";
    HintSettings: "settings", "ajustes";
    PreviewCommand: "echo \"The quick brown fox\"", "echo \"El veloz murciélago\"";
}

/// The string `key` in language `l`.
pub fn tr_in(l: Lang, key: Str) -> &'static str {
    match l {
        Lang::En => EN[key as usize],
        Lang::Es => ES[key as usize],
    }
}

/// The string `key` in the language in use.
pub fn tr(key: Str) -> &'static str {
    tr_in(current(), key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_languages_have_every_string_and_none_is_empty() {
        assert_eq!(EN.len(), ES.len());
        assert_eq!(EN.len(), Str::PreviewCommand as usize + 1, "the last key is the last string");
        for (i, (en, es)) in EN.iter().zip(ES).enumerate() {
            assert!(!en.is_empty() && !es.is_empty(), "string {i}");
        }
        // The backticked commands and keys are the same in both.
        for key in [Str::Note1, Str::Note3] {
            let ticks = |l| tr_in(l, key).matches('`').count();
            assert_eq!(ticks(Lang::En), ticks(Lang::Es));
        }
        assert_ne!(tr_in(Lang::En, Str::Save), tr_in(Lang::Es, Str::Save));
    }

    #[test]
    fn auto_follows_the_locale_variables_in_order() {
        let env = |a: Option<&str>, b: Option<&str>, c: Option<&str>| [a, b, c].map(|v| v.map(str::to_string));
        assert_eq!(Setting::Auto.resolve(&env(None, None, Some("es_PE.UTF-8"))), Lang::Es);
        assert_eq!(Setting::Auto.resolve(&env(None, None, Some("en_US.UTF-8"))), Lang::En);
        assert_eq!(Setting::Auto.resolve(&env(Some("en_US.UTF-8"), None, Some("es_ES.UTF-8"))), Lang::En, "LC_ALL wins");
        assert_eq!(Setting::Auto.resolve(&env(Some(""), Some("es_MX"), Some("en_US"))), Lang::Es, "an empty one is skipped");
        assert_eq!(Setting::Auto.resolve(&env(None, None, None)), Lang::En);
        assert_eq!(Setting::Auto.resolve(&env(None, None, Some("C"))), Lang::En);
        assert_eq!(Setting::Es.resolve(&env(None, None, Some("en_US"))), Lang::Es, "a chosen language ignores the locale");
        assert_eq!(Setting::En.resolve(&env(None, None, Some("es_ES"))), Lang::En);
    }

    #[test]
    fn settings_are_read_by_name() {
        assert_eq!(Setting::from_name("es"), Some(Setting::Es));
        assert_eq!(Setting::from_name("AUTO"), Some(Setting::Auto));
        assert_eq!(Setting::from_name("fr"), None);
        for s in Setting::ALL {
            assert_eq!(Setting::from_name(s.name()), Some(s));
        }
    }
}
