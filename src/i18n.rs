//! The interface in English and Spanish: every string the screens draw lives here, once per language,
//! in one list, so a string cannot exist in one language and be missing in the other (the macro builds
//! both tables from the same lines, and a test checks them).
//!
//! The language in use is one process-wide byte, set from the settings (`set`); `tr` is a table lookup
//! with no allocation. `[ui] language` is `auto` (from `LC_ALL`, `LC_MESSAGES` or `LANG`), `en` or `es`.
//! The config file, the command line and the shell hooks stay in English: they are for tools and for
//! people who read documentation; this is what the window says.
#[cfg(not(test))]
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
        // Windows has no locale variables: the user's own setting (`es-ES`).
        #[cfg(windows)]
        let system = crate::win::locale_name();
        #[cfg(not(windows))]
        let system = None;
        self.resolve(&[var("LC_ALL"), var("LC_MESSAGES"), var("LANG"), system])
    }
}

#[cfg(not(test))]
static CURRENT: AtomicU8 = AtomicU8::new(0);

// Tests run in parallel and some paint the Spanish board: the language is each test's own, not one all of them share.
#[cfg(test)]
thread_local! {
    static TEST_LANG: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

/// Makes `l` the language of every string from now on.
pub fn set(l: Lang) {
    #[cfg(test)]
    TEST_LANG.with(|t| t.set(l as u8));
    #[cfg(not(test))]
    CURRENT.store(l as u8, Ordering::Relaxed);
}

pub fn current() -> Lang {
    #[cfg(test)]
    let n = TEST_LANG.with(std::cell::Cell::get);
    #[cfg(not(test))]
    let n = CURRENT.load(Ordering::Relaxed);
    if n == 1 { Lang::Es } else { Lang::En }
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
    // What the composer says while nothing is typed at the prompt (the reference's `.ph`).
    TypeACommand: "Type a command", "Escribe un comando";
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
    GroupEdit: "Edit group", "Editar grupo";
    GroupEditHint: "change the name or the color", "cambia el nombre o el color";
    GroupSave: "Save", "Guardar";
    GroupColor: "Color", "Color";
    GroupWindows: "Windows to group", "Ventanas que se agrupan";
    GroupNewWindows: "Create new windows", "Crear ventanas nuevas";
    TplNone: "None", "Ninguna";
    TplNoneHint: "Only the chosen windows", "Solo las ventanas elegidas";
    TplOne: "One window", "Una ventana";
    TplOneHint: "An empty terminal", "Una terminal vacía";
    TplDev: "Development", "Desarrollo";
    TplDevHint: "server · tests · git", "servidor · pruebas · git";
    TplServer: "server", "servidor";
    TplTests: "tests", "pruebas";
    TplGit: "git", "git";
    ErrNameEmpty: "Give the group a name.", "Ponle un nombre al grupo.";
    ErrNameTaken: "There is already a group with that name.", "Ya hay un grupo con ese nombre.";
    ErrNothing: "Choose at least one window or create a new one.", "Elige al menos una ventana o crea una nueva.";
    WinOne: "window", "ventana";
    WinMany: "windows", "ventanas";
    MenuSplitRows: "Split into rows", "Dividir en filas";
    MenuRemoveFromGroup: "Remove from group", "Quitar del grupo";
    MenuCloseWin: "Close window", "Cerrar ventana";
    MenuEditGroup: "Edit name and color…", "Editar nombre y color…";
    MenuCollapse: "Collapse group", "Contraer grupo";
    MenuExpand: "Expand group", "Expandir grupo";
    MenuNewWinInGroup: "New window in the group", "Nueva ventana en el grupo";
    MenuDissolve: "Dissolve group", "Disolver grupo";
    MenuCloseGroup: "Close group", "Cerrar grupo";
    MenuGoPane: "Go to this pane", "Ir a este panel";
    MenuClosePane: "Close pane", "Cerrar panel";
    ToastClosed: "Window “{}” closed", "Ventana «{}» cerrada";
    ToastClosedMany: "{} windows closed", "{} ventanas cerradas";
    ToastGroupClosed: "Group “{}” closed", "Grupo «{}» cerrado";
    ToastMoved: "“{}” moved to “{}”", "«{}» movida a «{}»";
    ToastUngrouped: "“{}” out of the group", "«{}» fuera del grupo";
    ToastDissolved: "Group “{}” dissolved", "Grupo «{}» disuelto";
    ToastGroupMade: "Group “{}” created · {} {}", "Grupo «{}» creado · {} {}";
    ToastGroupSaved: "Group “{}” updated", "Grupo «{}» actualizado";
    ToastNoRoom: "No room for another pane", "No hay sitio para otro panel";
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
    OvTileHint: "or drag a window here to create it with that window", "o arrastra una ventana aquí para crearlo con ella";
    OvLooseEmpty: "Drag a window here.", "Arrastra una ventana aquí.";
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
    CmdWider: "Make the window wider", "Agrandar la ventana a lo ancho";
    CmdNarrower: "Make the window narrower", "Achicar la ventana a lo ancho";
    CmdTaller: "Make the window taller", "Agrandar la ventana a lo alto";
    CmdShorter: "Make the window shorter", "Achicar la ventana a lo alto";
    CmdEvenPanes: "Give the windows the same size", "Igualar el tamaño de las ventanas";
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
    // The panel of SSH servers (`hosts.rs`, ADR 0048), after the reference's `#hosts`.
    HostsTitle: "SSH servers", "Servidores SSH";
    HostsSearch: "Type to search…", "Escribe para buscar…";
    HostsChoose: "choose", "elegir";
    HostsConnect: "connect", "conectar";
    HostsNewWindow: "new window", "ventana nueva";
    HostsLive: "connected", "conectado";
    HostsNow: "now", "ahora";
    HostsMinAgo: "{} min ago", "hace {} min";
    HostsHoursAgo: "{} h ago", "hace {} h";
    HostsYesterday: "yesterday", "ayer";
    HostsDaysAgo: "{} d ago", "hace {} d";
    HostsEmpty: "No servers yet.", "Todavía no hay servidores.";
    HostsEmptyHint: "Press “+ Server” and paste a long ssh command: it becomes an alias.", "Pulsa «+ Servidor» y pega un comando ssh largo: se convierte en un alias.";
    HostsNoMatch: "No server matches “{}”.", "Ningún servidor coincide con «{}».";
    HostsMore: "More", "Más";
    HostsAdd: "+ Server", "+ Servidor";
    HostsMenuImport: "Import ~/.ssh/config…", "Importar ~/.ssh/config…";
    HostsMenuExport: "Export ~/.ssh/config", "Exportar ~/.ssh/config";
    HostsMenuCopy: "Copy ~/.ssh/config", "Copiar ~/.ssh/config";
    HostsNew: "New server", "Nuevo servidor";
    HostsServer: "Server", "Servidor";
    HostsPaste: "Paste an ssh command", "Pega un comando ssh";
    HostsPastePh: "ssh -i ~/.ssh/key.pem user@192.168.1.100 -p 2222", "ssh -i ~/.ssh/llave.pem usuario@192.168.1.100 -p 2222";
    HostsPasteHint: "It fills everything in. Press Enter to save it with the proposed name, or change it first.", "Se rellena todo solo. Pulsa Enter para guardarlo con el nombre propuesto, o cámbialo antes.";
    HostsPasteRead: "Read: {}", "Leído: {}";
    HostsPasteBad: "That does not look like a whole ssh command.", "No parece un comando ssh completo.";
    HostsAlias: "Alias", "Alias";
    HostsAliasPh: "production", "produccion";
    HostsAddress: "Address (IP or name)", "Dirección (IP o nombre)";
    HostsUser: "User", "Usuario";
    HostsPort: "Port", "Puerto";
    HostsKey: "Private key", "Llave privada";
    HostsCmd: "On arrival, run (optional)", "Al entrar, ejecutar (opcional)";
    HostsCmdPh: "ls  ·  cd /var/www/app && ls", "ls  ·  cd /var/www/app && ls";
    HostsDelete: "Delete", "Eliminar";
    HostsDuplicate: "Duplicate", "Duplicar";
    HostsCancel: "Cancel", "Cancelar";
    HostsSave: "Save", "Guardar";
    HostsDefaultAlias: "server", "servidor";
    HostsNoAlias: "Give it an alias.", "Ponle un alias.";
    HostsBadAlias: "An alias takes only letters, numbers, dot, hyphen and underscore (no spaces or patterns).", "El alias solo admite letras, números, punto, guion y guion bajo (sin espacios ni comodines).";
    HostsTaken: "There is already a server called “{}”.", "Ya existe un servidor llamado «{}».";
    HostsNoAddress: "The address is missing (IP or server name).", "Falta la dirección (IP o nombre del servidor).";
    HostsBadAddress: "The address has characters that are not valid.", "La dirección tiene caracteres no válidos.";
    HostsBadPort: "The port must be between 1 and 65535.", "El puerto debe estar entre 1 y 65535.";
    HostsLongAlias: "Long alias: a short one is quicker to type.", "Alias largo: uno corto se escribe más rápido.";
    HostsSameAs: "This same connection already exists as “{}”.", "Esta misma conexión ya existe como «{}».";
    HostsImportTitle: "Import ~/.ssh/config", "Importar ~/.ssh/config";
    HostsImportHint: "What your ~/.ssh/config has. The syntax is checked before anything is imported.", "Lo que tiene tu ~/.ssh/config. Se revisa la sintaxis antes de importar nada.";
    HostsImportNone: "~/.ssh/config is empty or not there.", "~/.ssh/config está vacío o no existe.";
    HostsImportFailed: "Could not read ~/.ssh/config: {}", "No se pudo leer ~/.ssh/config: {}";
    HostsFoundOne: "1 server found", "1 servidor encontrado";
    HostsFoundMany: "{} servers found", "{} servidores encontrados";
    HostsNewOne: "1 new", "1 nuevo";
    HostsNewMany: "{} new", "{} nuevos";
    HostsExistsOne: "1 already exists", "1 ya existe";
    HostsExistsMany: "{} already exist", "{} ya existen";
    HostsLine: "Line {}", "Línea {}";
    HostsIssueNoValue: "“{}” is not a valid directive: the value is missing.", "«{}» no es una directiva válida: falta el valor.";
    HostsIssueRepeated: "Host “{}” repeated: the first one is used.", "Host «{}» repetido: se usará el primero.";
    HostsIssueOutside: "“{}” is outside a Host block: it is ignored.", "«{}» está fuera de un bloque Host: se ignora.";
    HostsIssuePort: "Port “{}” is not valid.", "Puerto no válido «{}».";
    HostsIssueUnknown: "Unknown directive “{}”: check its spelling.", "Directiva desconocida «{}»: revisa si está bien escrita.";
    HostsImport: "Import", "Importar";
    HostsImportN: "Import {}", "Importar {}";
    HostsExportTitle: "Export ~/.ssh/config", "Exportar ~/.ssh/config";
    HostsExportHint: "A plain OpenSSH file, {}, which ~/.ssh/config includes: your aliases work outside axon too.", "Es un archivo normal de OpenSSH, {}, que ~/.ssh/config incluye: tus alias funcionan también fuera de axon.";
    HostsCountOne: "1 server", "1 servidor";
    HostsCountMany: "{} servers", "{} servidores";
    HostsBack: "Back", "Volver";
    HostsCopy: "Copy", "Copiar";
    HostsGroup: "Servers", "Servidores";
    ToastHostSaved: "“{}” saved: type “ssh {}”", "«{}» guardado: escribe «ssh {}»";
    ToastHostIncluded: " · ~/.ssh/config includes axon's servers now", " · ~/.ssh/config incluye ahora los servidores de axon";
    ToastHostDeleted: "“{}” deleted", "«{}» eliminado";
    ToastImportedOne: "1 server imported", "1 servidor importado";
    ToastImportedMany: "{} servers imported", "{} servidores importados";
    ToastConfigCopied: "Configuration copied", "Configuración copiada";
    ToastUseAgain: "Will you use it again?", "¿Lo usarás más veces?";
    ToastSaveAs: "Save as “{}”…", "Guardar como «{}»…";
    ToastHasAlias: "This server already has an alias: “ssh {}” does the same.", "Este servidor ya tiene alias: «ssh {}» hace lo mismo.";
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
    fn sentences_with_names_have_the_same_holes_in_both_languages() {
        for key in [Str::ToastClosed, Str::ToastClosedMany, Str::ToastGroupClosed, Str::ToastMoved, Str::ToastUngrouped, Str::ToastDissolved, Str::ToastGroupMade, Str::ToastGroupSaved] {
            let holes = |l| tr_in(l, key).matches("{}").count();
            assert!(holes(Lang::En) > 0 && holes(Lang::En) == holes(Lang::Es), "{key:?}");
        }
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
