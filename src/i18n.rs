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
    TabFont: "Font", "Fuente";
    TabInput: "Input", "Entrada";
    TabWindows: "Windows", "Ventanas";
    RowFont: "Font", "Fuente";
    RowSize: "Size", "Tamaño";
    RowWeight: "Weight", "Peso";
    RowBold: "Bold text", "Texto en negrita";
    RowSpacing: "Letter spacing", "Espaciado de letras";
    RowLine: "Line height", "Altura de línea";
    RowOpacity: "Opacity", "Opacidad";
    RowBlur: "Blur", "Desenfoque";
    RowLanguage: "Language", "Idioma";
    RowMode: "Input type", "Tipo de entrada";
    RowClock: "Clock", "Reloj";
    RowCursor: "Cursor", "Cursor";
    RowChips: "Chips", "Chips";
    RowAutosuggest: "Autosuggestions", "Autosugerencias";
    RowTabs: "Position", "Posición";
    RowSide: "Panel visible", "Panel visible";
    RowSideW: "Panel width", "Ancho del panel";
    OptShell: "Shell prompt", "Prompt del shell";
    OptChips: "Prompt chips", "Chips de prompt";
    Opt12: "12-hour", "12 horas";
    Opt24: "24-hour", "24 horas";
    OptBlock: "Block", "Bloque";
    OptBar: "Bar", "Barra";
    OptUnderline: "Underline", "Línea";
    OptLeft: "Left", "Izquierda";
    OptRight: "Right", "Derecha";
    OptTop: "Top", "Arriba";
    OptAuto: "Auto", "Auto";
    OptEnglish: "English", "English";
    OptSpanish: "Español", "Español";
    On: "on", "activado";
    Off: "off", "desactivado";
    OnUnsupported: "on, not offered here", "activado, no disponible aquí";
    OffUnsupported: "off, not offered here", "desactivado, no disponible aquí";
    NoteInput: "The chips replace your prompt's text with a `$`.\nDrag a chip to reorder it, or use [Alt] + [←] / [→].\nzsh and fish: add `axon shell-init` to the rc file.", "Los chips reemplazan el texto de tu prompt por un `$`.\nArrastra un chip para reordenarlo, o usa [Alt] + [←] / [→].\nzsh y fish: añade `axon shell-init` al archivo rc.";
    NoteWindows: "Windows and groups can live in a panel at one side or in tabs at the top.\nShow or hide it with [Alt] + [B].", "Las ventanas y los grupos pueden vivir en un panel a un lado o en pestañas arriba.\nMuéstralo u ocúltalo con [Alt] + [B].";
    Reset: "Reset to defaults", "Restablecer valores";
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
    Cancel: "Cancel", "Cancelar";
    OvNoMatches: "Nothing matches", "Nada coincide con";
    OvKeyMove: "move", "moverse";
    OvKeyRun: "run", "ejecutar";
    OvKeyClose: "close", "cerrar";
    OvGroupActions: "Actions", "Acciones";
    OvGroupGroups: "Groups", "Grupos";
    OvSearch: "Search windows, groups, actions…", "Busca ventanas, grupos, acciones…";
    OvActNewWindow: "New window", "Nueva ventana";
    OvActNewGroup: "New group…", "Nuevo grupo…";
    OvActSplit: "Split into columns", "Dividir en columnas";
    OvActOverview: "Overview of windows", "Vista general de ventanas";
    OvWindow1: "window", "ventana";
    OvWindowsN: "windows", "ventanas";
    OvGroup1: "group", "grupo";
    OvGroupsN: "groups", "grupos";
    OvBtnNewGroup: "New group", "Nuevo grupo";
    OvBtnAdd: "Window", "Ventana";
    OvLoose: "No group", "Sin grupo";
    OvNoneBefore: "No window matches “", "Ninguna ventana coincide con «";
    OvNoneAfter: "”.", "».";
    OvTileHint: "around the selected window", "con la ventana seleccionada";
    OvToastNewWindow: "New window created", "Ventana nueva creada";
    OvToastAdded: "Window added to the group", "Ventana añadida al grupo";
    OvActPanel: "Show or hide the windows panel", "Mostrar u ocultar el panel de ventanas";
    OvActClose: "Close the window", "Cerrar la ventana";
    CmdNewPane: "New pane in this window", "Nuevo panel en esta ventana";
    CmdSplitRows: "Split into rows", "Dividir en filas";
    CmdTabsLeft: "Windows and groups on the left", "Ventanas y grupos a la izquierda";
    CmdTabsRight: "Windows and groups on the right", "Ventanas y grupos a la derecha";
    CmdTabsTop: "Windows and groups at the top (tabs)", "Ventanas y grupos arriba (pestañas)";
    CmdHistory: "Search the history", "Buscar en el historial";
    CmdClear: "Clear the screen", "Limpiar la pantalla";
    CmdClosePane: "Close the pane", "Cerrar el panel";
    CmdUngroup: "Take the window out of its group", "Sacar la ventana de su grupo";
    CmdFold: "Collapse group", "Contraer grupo";
    CmdUnfold: "Expand group", "Expandir grupo";
    CmdWindowOne: "window", "ventana";
    CmdWindowMany: "windows", "ventanas";
    KeySpace: "Space", "Espacio";
    ToastPanelLeft: "Windows and groups: panel on the left", "Ventanas y grupos: panel a la izquierda";
    ToastPanelRight: "Windows and groups: panel on the right", "Ventanas y grupos: panel a la derecha";
    ToastTabsTop: "Windows and groups: tabs at the top", "Ventanas y grupos: pestañas arriba";
    ToastNoPanel: "With the tabs at the top there is no side panel", "Con las pestañas arriba no hay panel lateral";
    OvThreeWindows: "3 windows", "3 ventanas";
    OvDialogTitle: "New group", "Nuevo grupo";
    OvDialogHint: "groups related windows", "agrupa ventanas relacionadas";
    OvFieldName: "Name", "Nombre";
    OvFieldNameHint: "e.g. arcton, api, docs", "p. ej. arcton, api, docs";
    OvFieldSearch: "Filter windows…", "Filtrar ventanas…";
    OvCreateGroup: "Create group", "Crear grupo";
    OvMenuRename: "Rename", "Renombrar";
    OvMenuDuplicate: "Duplicate", "Duplicar";
    OvMenuGroup: "Group", "Grupo";
    OvMenuNewGroupWith: "New group with this window…", "Nuevo grupo con esta ventana…";
    OvMenuCloseOthers: "Close the others", "Cerrar las demás";
    OvToastClosed: "Group “arcton” closed", "Grupo «arcton» cerrado";
    OvUndo: "Undo", "Deshacer";
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
        assert_eq!(EN.len(), Str::OvUndo as usize + 1, "the last key is the last string");
        for (i, (en, es)) in EN.iter().zip(ES).enumerate() {
            assert!(!en.is_empty() && !es.is_empty(), "string {i}");
        }
        // The notes' markup (`code`, [keys]) and their lines are the same in both languages.
        for key in [Str::NoteInput, Str::NoteWindows] {
            let count = |l, c: char| tr_in(l, key).matches(c).count();
            for c in ['`', '[', ']', '\n'] {
                assert_eq!(count(Lang::En, c), count(Lang::Es, c), "{key:?} {c:?}");
            }
        }
        assert_ne!(tr_in(Lang::En, Str::Reset), tr_in(Lang::Es, Str::Reset));
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
