//! SSH servers by alias ([0048](../knowledge/decisions/workspace/0048-ssh-servers-by-alias.md)): the model behind the panel of
//! servers (`hosts.rs`, `Alt+H`), after the reference's `parseSshArgs`, `suggestAlias`, `validateHost`, `genConfig` and
//! `parseConfig`.
//!
//! The servers live in an OpenSSH file of axon's own, `ssh_config` beside `config.toml`, and `~/.ssh/config` includes it:
//! axon puts one `Include` line at the top of that file the first time it saves a server, and never rewrites anything else
//! in it. So `ssh web` works in every shell and every program, inside axon or not, and Tab completes the aliases wherever the
//! shell's completion reads the `Include`s. When each server was last used, and how often, is state rather than settings:
//! `ssh-used` in the state directory, beside the session.
//!
//! Pure but for `Book::load`, `Book::save`, `Book::save_used` and `Book::read_ssh_config`: the parsing of a command line, of a
//! config file and of a form are functions of their text, tested here.
use std::{
    io,
    path::{Path, PathBuf},
};

/// One server: what its `Host` block says, and how it was used.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Host {
    pub alias: String,
    pub hostname: String,
    /// Empty: the user ssh picks (the local one).
    pub user: String,
    /// 0: the default, 22.
    pub port: u16,
    pub identity: String,
    /// What runs on arrival; the session then goes on in a login shell (`RemoteCommand <cmd>; exec $SHELL -l`).
    pub cmd: String,
    /// When it was last used (seconds since the epoch, 0: never) and how many times.
    pub last: u64,
    pub uses: u32,
}

impl Host {
    pub fn port(&self) -> u16 {
        if self.port == 0 { 22 } else { self.port }
    }

    /// `user@hostname`, and `:port` when it is not 22 (the reference's `target`); `local` stands in for no user.
    pub fn target(&self, local: &str) -> String {
        let user = if self.user.is_empty() { local } else { &self.user };
        let mut s = if user.is_empty() { self.hostname.clone() } else { format!("{user}@{}", self.hostname) };
        if self.port() != 22 {
            s.push(':');
            s.push_str(&self.port().to_string());
        }
        s
    }
}

/// The user this machine's ssh connects as when none is said.
pub fn local_user() -> String {
    ["USER", "LOGNAME", "USERNAME"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty())).unwrap_or_default()
}

// ------------------------------------------------------------ command lines --

/// What the options of an ssh command line say, as far as a server is concerned.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Args<'a> {
    pub user: Option<&'a str>,
    pub port: Option<&'a str>,
    pub identity: Option<&'a str>,
    /// The first word that is not an option or an option's value: the server.
    pub target: Option<&'a str>,
    /// Words follow the server: a command to run there, not a session.
    pub command: bool,
}

/// The options of ssh that take a value (`ssh(1)`), glued (`-p2222`) or in the next word (`-p 2222`).
const WITH_VALUE: &str = "BbcDEeFIiJLlmOoPpQRSWw";

/// Reads ssh's arguments (the words after `ssh`): options may come before and after the server, as ssh takes them, and
/// what follows the server that is not an option is a command to run there.
pub fn args<'a>(words: &[&'a str]) -> Args<'a> {
    let mut a = Args::default();
    let mut i = 0;
    let mut options = true;
    while i < words.len() {
        let w = words[i];
        i += 1;
        if options && w == "--" {
            options = false;
            continue;
        }
        if options && w.len() > 1 && w.starts_with('-') {
            for (at, c) in w.char_indices().skip(1) {
                if !WITH_VALUE.contains(c) {
                    continue; // a flag (`-t`, `-A`, `-v`...), alone or in a cluster
                }
                let glued = &w[at + c.len_utf8()..];
                let value = if !glued.is_empty() {
                    Some(glued)
                } else {
                    i += 1;
                    words.get(i - 1).copied()
                };
                match c {
                    'l' => a.user = value,
                    'p' => a.port = value,
                    'i' => a.identity = value,
                    _ => {}
                }
                break;
            }
            continue;
        }
        if a.target.is_none() {
            a.target = Some(w);
        } else {
            a.command = true;
            break;
        }
    }
    a
}

/// The words of a command line, each without the quotes around it (`'-p' "22"`): enough for the ssh lines people type.
pub fn words(line: &str) -> Vec<&str> {
    line.split_whitespace()
        .map(|w| {
            let b = w.as_bytes();
            if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0] { &w[1..w.len() - 1] } else { w }
        })
        .collect()
}

/// Where an ssh command line goes: the user, server and port it names (an alias is a server), and its key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dest {
    pub user: String,
    pub host: String,
    /// 0: none said.
    pub port: u16,
    pub identity: String,
}

/// `user@host`, `host`, or `ssh://[user@]host[:port]`.
fn split_target(t: &str) -> (Option<&str>, &str, Option<&str>) {
    if let Some(rest) = t.strip_prefix("ssh://") {
        let rest = rest.trim_end_matches('/');
        let (user, hp) = match rest.rsplit_once('@') {
            Some((u, h)) => (Some(u), h),
            None => (None, rest),
        };
        return match hp.rsplit_once(':') {
            Some((h, p)) if p.bytes().all(|b| b.is_ascii_digit()) && !p.is_empty() => (user, h, Some(p)),
            _ => (user, hp, None),
        };
    }
    match t.rsplit_once('@') {
        Some((u, h)) => (Some(u), h, None),
        None => (None, t, None),
    }
}

/// Where `a` goes, if it names a server.
pub fn dest(a: &Args) -> Option<Dest> {
    let (user, host, port) = split_target(a.target?);
    if host.is_empty() {
        return None;
    }
    let port = port.or(a.port).and_then(|p| p.parse::<u16>().ok()).unwrap_or(0);
    Some(Dest { user: user.or(a.user).unwrap_or("").to_string(), host: host.to_string(), port, identity: a.identity.unwrap_or("").to_string() })
}

/// Whether the first word of `line` that is not a `VAR=value` is ssh (a path to it, `ssh.exe`).
fn is_ssh(word: &str) -> bool {
    let name = word.rsplit(['/', '\\']).next().unwrap_or(word);
    name == "ssh" || name.eq_ignore_ascii_case("ssh.exe")
}

/// Where a command line opens a session, when it is an ssh to a server with nothing to run there: `ssh web`,
/// `ssh -p 2222 me@10.0.0.1`. None for another program, or for `ssh web uptime`, which runs and comes back.
pub fn session(line: &str) -> Option<Dest> {
    let w = words(line);
    let first = w.iter().position(|x| !x.contains('='))?;
    if !is_ssh(w[first]) {
        return None;
    }
    let a = args(&w[first + 1..]);
    if a.command {
        return None;
    }
    dest(&a)
}

/// An address rather than an alias: has a user, is an IPv4 address or a dotted name (`looksAddr`).
pub fn looks_address(t: &str) -> bool {
    if t.contains('@') {
        return true;
    }
    let parts: Vec<&str> = t.split('.').collect();
    if parts.len() == 4 && parts.iter().all(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit())) {
        return true;
    }
    parts.len() > 1 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
}

fn is_ipv4(t: &str) -> bool {
    let parts: Vec<&str> = t.split('.').collect();
    parts.len() == 4 && parts.iter().all(|p| !p.is_empty() && p.len() <= 3 && p.bytes().all(|b| b.is_ascii_digit()))
}

// ------------------------------------------------------------------ forms --

/// A server as the form edits it: text in every field (a port being typed is text).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Form {
    pub alias: String,
    pub hostname: String,
    pub user: String,
    pub port: String,
    pub identity: String,
    pub cmd: String,
}

impl Form {
    pub fn of(h: &Host) -> Form {
        Form {
            alias: h.alias.clone(),
            hostname: h.hostname.clone(),
            user: h.user.clone(),
            port: if h.port == 0 { String::new() } else { h.port.to_string() },
            identity: h.identity.clone(),
            cmd: h.cmd.clone(),
        }
    }

    /// The server the form says, trimmed (`check` has found nothing wrong with it).
    pub fn host(&self) -> Host {
        Host {
            alias: self.alias.trim().to_string(),
            hostname: self.hostname.trim().to_string(),
            user: self.user.trim().to_string(),
            port: self.port.trim().parse().unwrap_or(0),
            identity: self.identity.trim().to_string(),
            cmd: self.cmd.trim().to_string(),
            last: 0,
            uses: 0,
        }
    }
}

/// What a long ssh command pasted into the form says (`ssh -i ~/.ssh/key.pem me@192.168.1.100 -p 2222`): the address,
/// user, port and key, the alias left as it was. None when it does not name an address.
pub fn pasted(text: &str) -> Option<Form> {
    let mut w = words(text.trim());
    if w.first().is_some_and(|f| is_ssh(f)) {
        w.remove(0);
    }
    let a = args(&w);
    if !a.target.is_some_and(looks_address) {
        return None;
    }
    let d = dest(&a)?;
    Some(Form {
        alias: String::new(),
        hostname: d.host,
        user: d.user,
        port: if d.port == 0 || d.port == 22 { String::new() } else { d.port.to_string() },
        identity: d.identity,
        cmd: String::new(),
    })
}

/// A name for a server from its key's file name (`servidor-prod.pem`: `servidor-prod`), else from its address
/// (`192.168.1.100`: `srv-100`, `api.example.com`: `api`), else `fallback`; with `-2`, `-3`... when taken (`suggestAlias`).
pub fn suggest_alias(f: &Form, hosts: &[Host], fallback: &str) -> String {
    let file = f.identity.trim().rsplit(['/', '\\']).next().unwrap_or("");
    let lower = file.to_ascii_lowercase();
    let stem = [".pem", ".key", ".pub"].iter().find(|e| lower.ends_with(*e)).map_or(file, |e| &file[..file.len() - e.len()]);
    let stem = stem.strip_prefix("id_").unwrap_or(stem);
    let ok = |s: &str| {
        let b = s.as_bytes();
        b.len() >= 2 && b[0].is_ascii_alphabetic() && b.iter().all(|&c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
    };
    let mut base = if ok(stem) {
        stem.to_string()
    } else {
        let h = f.hostname.trim();
        if is_ipv4(h) {
            format!("srv-{}", h.rsplit('.').next().unwrap_or(""))
        } else {
            match h.split('.').next() {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => fallback.to_string(),
            }
        }
    };
    base = base.chars().take(16).collect();
    let mut name = base.clone();
    let mut n = 2;
    while hosts.iter().any(|h| h.alias.eq_ignore_ascii_case(&name)) {
        name = format!("{base}-{n}");
        n += 1;
    }
    name
}

/// What is wrong with a form (it cannot be saved) or worth saying (it can).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    NoAlias,
    /// Letters, digits, `.`, `_` and `-` only: no spaces, no patterns.
    BadAlias,
    TakenAlias(String),
    NoAddress,
    BadAddress,
    BadPort,
    /// Longer than 16 characters: a short one is quicker to type.
    LongAlias,
    /// The same user, address and port are already saved under this alias.
    SameAs(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Check {
    pub errors: Vec<Problem>,
    pub warnings: Vec<Problem>,
}

fn alias_ok(a: &str) -> bool {
    let b = a.as_bytes();
    !b.is_empty() && b[0].is_ascii_alphanumeric() && b.iter().all(|&c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

fn port_ok(p: &str) -> bool {
    !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) && p.parse::<u32>().is_ok_and(|n| (1..=65535).contains(&n))
}

/// The reference's `validateHost`: errors keep the form from being saved, warnings only guide. `editing` is the server
/// being edited (its own alias is not taken).
pub fn check(f: &Form, hosts: &[Host], editing: Option<usize>, local: &str) -> Check {
    let mut c = Check::default();
    let (alias, hostname, port) = (f.alias.trim(), f.hostname.trim(), f.port.trim());
    let others = || hosts.iter().enumerate().filter(move |(i, _)| Some(*i) != editing).map(|(_, h)| h);
    if alias.is_empty() {
        c.errors.push(Problem::NoAlias);
    } else if !alias_ok(alias) {
        c.errors.push(Problem::BadAlias);
    } else if others().any(|h| h.alias.eq_ignore_ascii_case(alias)) {
        c.errors.push(Problem::TakenAlias(alias.to_string()));
    }
    if hostname.is_empty() {
        c.errors.push(Problem::NoAddress);
    } else if !hostname.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-')) {
        c.errors.push(Problem::BadAddress);
    }
    if !port.is_empty() && !port_ok(port) {
        c.errors.push(Problem::BadPort);
    }
    if !alias.is_empty() && c.errors.is_empty() {
        if alias.chars().count() > 16 {
            c.warnings.push(Problem::LongAlias);
        }
        let x = f.host();
        if let Some(same) = others().find(|h| h.hostname == x.hostname && h.port() == x.port() && h.target(local) == x.target(local)) {
            c.warnings.push(Problem::SameAs(same.alias.clone()));
        }
    }
    c
}

// ---------------------------------------------------------------- finding --

/// Lowercase without accents: what a search compares (`norm`).
pub fn fold(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

/// The first letters of an alias's parts (`prod-web`: `pw`).
fn initials(alias: &str) -> String {
    alias.split(['-', '_', '.']).filter_map(|p| p.chars().next()).flat_map(char::to_lowercase).collect()
}

/// The servers in the order the panel lists them: the last used first, the never used by name (`hostsByRecent`).
pub fn by_recent(hosts: &[Host]) -> Vec<usize> {
    let mut v: Vec<usize> = (0..hosts.len()).collect();
    v.sort_by(|&a, &b| hosts[b].last.cmp(&hosts[a].last).then_with(|| hosts[a].alias.to_lowercase().cmp(&hosts[b].alias.to_lowercase())));
    v
}

/// What the panel's search keeps of `order`: the aliases with `q` in them or with `q` as their initials; when none, the
/// servers whose address or user has it.
pub fn filter(hosts: &[Host], order: &[usize], q: &str) -> Vec<usize> {
    let q = fold(q.trim());
    if q.is_empty() {
        return order.to_vec();
    }
    let by_alias: Vec<usize> = order.iter().copied().filter(|&i| fold(&hosts[i].alias).contains(&q) || initials(&hosts[i].alias) == q).collect();
    if !by_alias.is_empty() {
        return by_alias;
    }
    order.iter().copied().filter(|&i| fold(&format!("{} {}", hosts[i].hostname, hosts[i].user)).contains(&q)).collect()
}

/// The saved server a session goes to: the one with that alias (ssh compares them without case), else the one with that
/// address, port and (when said) user.
pub fn find(hosts: &[Host], d: &Dest) -> Option<usize> {
    if let Some(i) = hosts.iter().position(|h| h.alias.eq_ignore_ascii_case(&d.host)) {
        return Some(i);
    }
    let port = if d.port == 0 { 22 } else { d.port };
    hosts.iter().position(|h| h.hostname.eq_ignore_ascii_case(&d.host) && h.port() == port && (d.user.is_empty() || h.user == d.user))
}

// ------------------------------------------------------------ config files --

/// The tail `RemoteCommand` carries so that the session goes on after the command (`cmd`).
const STAY: &str = "; exec $SHELL -l";

/// A value as a config line writes it: quoted when it has spaces.
fn quote(v: &str) -> String {
    if v.chars().any(char::is_whitespace) { format!("\"{v}\"") } else { v.to_string() }
}

fn unquote(v: &str) -> &str {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') { &v[1..v.len() - 1] } else { v }
}

/// axon's file: one `Host` block per server, in the order they were added (`genConfig`).
pub fn write_config(hosts: &[Host]) -> String {
    let mut s = String::from(
        "# axon's SSH servers (Alt+H). ~/.ssh/config\n\
         # includes this file: `ssh <alias>` works in\n\
         # every shell. axon writes it whole when a\n\
         # server changes: comments are not kept.\n",
    );
    for h in hosts {
        s.push_str(&format!("\nHost {}\n    HostName {}\n", h.alias, h.hostname));
        if !h.user.is_empty() {
            s.push_str(&format!("    User {}\n", quote(&h.user)));
        }
        if h.port() != 22 {
            s.push_str(&format!("    Port {}\n", h.port));
        }
        if !h.identity.is_empty() {
            s.push_str(&format!("    IdentityFile {}\n    IdentitiesOnly yes\n", quote(&h.identity)));
        }
        if !h.cmd.is_empty() {
            // `%` starts one of ssh's tokens in a `RemoteCommand`: a literal one is written twice.
            s.push_str(&format!("    RequestTTY yes\n    RemoteCommand {}{STAY}\n", h.cmd.replace('%', "%%")));
        }
    }
    s
}

/// How bad a line of a config file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Error,
    Warn,
}

/// What is wrong on a line of a config file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IssueKind {
    /// A keyword with no value.
    NoValue(String),
    /// A second `Host` block with this alias: ssh uses the first.
    Repeated(String),
    /// A keyword before any `Host` (other than the ones that belong there).
    Outside(String),
    BadPort(String),
    /// A keyword ssh does not know: misspelt?
    Unknown(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// From 1.
    pub line: usize,
    pub level: Level,
    pub kind: IssueKind,
}

/// A server found in a config file, and the line its `Host` is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub host: Host,
    pub line: usize,
    /// A line of its block is wrong (a port that is not one): it is not imported.
    pub bad: bool,
}

/// The keywords of `ssh_config(5)`, in lowercase: one not here is reported as unknown.
const KEYWORDS: &[&str] = &[
    "addkeystoagent", "addressfamily", "batchmode", "bindaddress", "bindinterface", "canonicaldomains",
    "canonicalizefallbacklocal", "canonicalizehostname", "canonicalizemaxdots", "canonicalizepermittedcnames",
    "casignaturealgorithms", "certificatefile", "challengeresponseauthentication", "channeltimeout", "checkhostip", "cipher",
    "ciphers", "clearallforwardings", "compression", "connectionattempts", "connecttimeout", "controlmaster", "controlpath",
    "controlpersist", "dynamicforward", "enableescapecommandline", "enablesshkeysign", "escapechar", "exitonforwardfailure",
    "fingerprinthash", "forkafterauthentication", "forwardagent", "forwardx11", "forwardx11timeout", "forwardx11trusted",
    "gatewayports", "globalknownhostsfile", "gssapiauthentication", "gssapiclientidentity", "gssapidelegatecredentials",
    "gssapikeyexchange", "gssapirenewalforcesrekey", "gssapiserveridentity", "gssapitrustdns", "hashknownhosts", "host",
    "hostbasedacceptedalgorithms", "hostbasedauthentication", "hostbasedkeytypes", "hostkeyalgorithms", "hostkeyalias",
    "hostname", "identitiesonly", "identityagent", "identityfile", "ignoreunknown", "include", "ipqos",
    "kbdinteractiveauthentication", "kbdinteractivedevices", "kexalgorithms", "knownhostscommand", "localcommand",
    "localforward", "loglevel", "logverbose", "macs", "match", "nohostauthenticationforlocalhost", "numberofpasswordprompts",
    "obscurekeystroketiming", "passwordauthentication", "permitlocalcommand", "permitremoteopen", "pkcs11provider", "port",
    "preferredauthentications", "protocol", "proxycommand", "proxyjump", "proxyusefdpass", "pubkeyacceptedalgorithms",
    "pubkeyacceptedkeytypes", "pubkeyauthentication", "refuseconnection", "rekeylimit", "remotecommand", "remoteforward",
    "requesttty", "requiredrsasize", "revokedhostkeys", "rsaauthentication", "securitykeyprovider", "sendenv",
    "serveralivecountmax", "serveraliveinterval", "sessiontype", "setenv", "stdinnull", "streamlocalbindmask",
    "streamlocalbindunlink", "stricthostkeychecking", "syslogfacility", "tag", "tcpkeepalive", "tunnel", "tunneldevice",
    "updatehostkeys", "usekeychain", "user", "userknownhostsfile", "verifyhostkeydns", "visualhostkey", "warnweakcrypto",
    "xauthlocation",
];

/// The servers a config file defines and what is wrong in it, with line numbers (`parseConfig`). A `Host` line with
/// several names defines each; a pattern (`*`, `?`, `!`) and a `Match` block define none. A server with no `HostName`
/// goes to its alias, as ssh does.
pub fn read_config(text: &str) -> (Vec<Found>, Vec<Issue>) {
    let mut found: Vec<Found> = Vec::new();
    let mut issues = Vec::new();
    // The servers the block being read defines (indices into `found`), or none (a pattern, a `Match`, before any `Host`).
    let mut block: Option<std::ops::Range<usize>> = None;
    let mut in_block = false;
    let mut ignore: Vec<String> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let ln = i + 1;
        let line = raw.split_once('#').map_or(raw, |(a, _)| a).trim();
        if line.is_empty() {
            continue;
        }
        let (key, value) = match line.find(|c: char| c.is_whitespace() || c == '=') {
            Some(at) => (&line[..at], line[at..].trim_start_matches(|c: char| c.is_whitespace() || c == '=').trim()),
            None => (line, ""),
        };
        let k = key.to_ascii_lowercase();
        if value.is_empty() {
            issues.push(Issue { line: ln, level: Level::Error, kind: IssueKind::NoValue(key.to_string()) });
            continue;
        }
        match k.as_str() {
            "host" => {
                in_block = true;
                let start = found.len();
                for name in value.split_whitespace().map(unquote) {
                    if name.contains(['*', '?', '!']) {
                        continue;
                    }
                    if found.iter().any(|f| f.host.alias.eq_ignore_ascii_case(name)) {
                        issues.push(Issue { line: ln, level: Level::Warn, kind: IssueKind::Repeated(name.to_string()) });
                        continue;
                    }
                    found.push(Found { host: Host { alias: name.to_string(), ..Host::default() }, line: ln, bad: false });
                }
                block = Some(start..found.len());
                continue;
            }
            "match" => {
                in_block = true;
                block = None;
                continue;
            }
            "ignoreunknown" => ignore.extend(value.split([',', ' ']).filter(|s| !s.is_empty()).map(str::to_ascii_lowercase)),
            _ => {}
        }
        if !KEYWORDS.contains(&k.as_str()) && !ignore.iter().any(|p| p == &k) {
            issues.push(Issue { line: ln, level: Level::Warn, kind: IssueKind::Unknown(key.to_string()) });
            continue;
        }
        if !in_block {
            // What comes before the first block applies to every server; `Include` and `IgnoreUnknown` belong there.
            if !matches!(k.as_str(), "include" | "ignoreunknown") {
                issues.push(Issue { line: ln, level: Level::Warn, kind: IssueKind::Outside(key.to_string()) });
            }
            continue;
        }
        let Some(range) = block.clone() else { continue };
        let v = unquote(value);
        for f in &mut found[range] {
            // ssh keeps the first value it reads for each keyword.
            let h = &mut f.host;
            match k.as_str() {
                "hostname" if h.hostname.is_empty() => h.hostname = v.to_string(),
                "user" if h.user.is_empty() => h.user = v.to_string(),
                "identityfile" if h.identity.is_empty() => h.identity = v.to_string(),
                "remotecommand" if h.cmd.is_empty() => {
                    let c = v.replace("%%", "%");
                    h.cmd = c.strip_suffix(STAY).unwrap_or(&c).to_string();
                }
                "port" if h.port == 0 => match v.parse::<u16>() {
                    Ok(p) if p > 0 => h.port = p,
                    _ => f.bad = true,
                },
                _ => {}
            }
        }
        if k == "port" && !v.parse::<u16>().is_ok_and(|p| p > 0) {
            issues.push(Issue { line: ln, level: Level::Error, kind: IssueKind::BadPort(v.to_string()) });
        }
    }
    for f in &mut found {
        if f.host.hostname.is_empty() {
            f.host.hostname = f.host.alias.clone();
        }
    }
    (found, issues)
}

/// The line that includes `file` in `~/.ssh/config`: with `~` when it is under `home`, quoted when it has spaces.
pub fn include_line(file: &Path, home: Option<&Path>) -> String {
    let path = match home.and_then(|h| file.strip_prefix(h).ok()) {
        Some(rel) => format!("~/{}", rel.to_string_lossy().replace('\\', "/")),
        None => file.to_string_lossy().replace('\\', "/"),
    };
    format!("Include {}", quote(&path))
}

/// Whether `text` (a `~/.ssh/config`) includes the file `line` names before its first block, where it applies to every
/// server (an `Include` inside a `Host` block applies to that block only).
pub fn includes(text: &str, line: &str) -> bool {
    let want = line.split_once(' ').map_or("", |(_, p)| unquote(p));
    for raw in text.lines() {
        let l = raw.split_once('#').map_or(raw, |(a, _)| a).trim();
        let Some((k, v)) = l.split_once(char::is_whitespace) else { continue };
        match k.to_ascii_lowercase().as_str() {
            "host" | "match" => return false,
            "include" if v.split_whitespace().map(unquote).any(|p| p == want) => return true,
            _ => {}
        }
    }
    false
}

/// `text` with `line` at its top, where it applies to every server.
pub fn with_include(text: &str, line: &str) -> String {
    let mut s = format!("# The SSH servers of axon's panel (Alt+H).\n{line}\n");
    if !text.is_empty() {
        s.push('\n');
        s.push_str(text);
    }
    s
}

/// The state file of when each server was used: `alias<TAB>last<TAB>uses` lines.
pub fn write_used(hosts: &[Host]) -> String {
    hosts.iter().filter(|h| h.last > 0 || h.uses > 0).map(|h| format!("{}\t{}\t{}\n", h.alias, h.last, h.uses)).collect()
}

pub fn read_used(text: &str, hosts: &mut [Host]) {
    for line in text.lines() {
        let mut f = line.split('\t');
        let (Some(alias), Some(last), Some(uses)) = (f.next(), f.next(), f.next()) else { continue };
        if let Some(h) = hosts.iter_mut().find(|h| h.alias == alias) {
            h.last = last.parse().unwrap_or(0);
            h.uses = uses.parse().unwrap_or(0);
        }
    }
}

// ------------------------------------------------------------------- files --

/// The servers and where they are kept.
#[derive(Clone, Debug, Default)]
pub struct Book {
    pub hosts: Vec<Host>,
    /// axon's file, `~/.ssh/config`, and the state file of uses; none in a book that is only in memory.
    file: Option<PathBuf>,
    ssh_config: Option<PathBuf>,
    used: Option<PathBuf>,
}

/// What a save did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Saved {
    Written,
    /// ... and `~/.ssh/config` includes it now (it did not before).
    Included,
}

/// Seconds since the epoch.
pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Two uses closer than this are one (the panel connects, and the shell's hook says so a moment later).
const SAME_USE_SECS: u64 = 10;

impl Book {
    /// The servers in memory only (the design board, the tests): saving writes nothing.
    #[cfg(test)]
    pub fn memory(hosts: Vec<Host>) -> Book {
        Book { hosts, ..Book::default() }
    }

    /// Reads axon's file and the uses. A file that is not there is an empty book.
    pub fn load() -> Book {
        let file = crate::config::path().and_then(|p| Some(p.parent()?.join("ssh_config")));
        let ssh_config = crate::context::home_dir().map(|h| h.join(".ssh").join("config"));
        let used = crate::restore::path_named("ssh-used");
        let mut hosts: Vec<Host> = file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()).map(|t| read_config(&t).0.into_iter().map(|f| f.host).collect()).unwrap_or_default();
        if let Some(t) = used.as_ref().and_then(|u| std::fs::read_to_string(u).ok()) {
            read_used(&t, &mut hosts);
        }
        Book { hosts, file, ssh_config, used }
    }

    /// axon's file, for the export view.
    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    /// `~/.ssh/config` as it is now, for the import view (empty when it is not there).
    pub fn read_ssh_config(&self) -> io::Result<String> {
        match &self.ssh_config {
            Some(p) => match std::fs::read_to_string(p) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
                r => r,
            },
            None => Ok(String::new()),
        }
    }

    /// Writes axon's file, and the `Include` into `~/.ssh/config` when it is not there yet.
    pub fn save(&self) -> io::Result<Saved> {
        let Some(file) = &self.file else { return Ok(Saved::Written) };
        write_atomic(file, write_config(&self.hosts).as_bytes(), false)?;
        let Some(cfg) = &self.ssh_config else { return Ok(Saved::Written) };
        let line = include_line(file, crate::context::home_dir().as_deref());
        let text = match std::fs::read_to_string(cfg) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        if includes(&text, &line) {
            return Ok(Saved::Written);
        }
        // A link (a dotfiles manager's) stays one: the file it points to is written.
        let target = std::fs::canonicalize(cfg).unwrap_or_else(|_| cfg.clone());
        write_atomic(&target, with_include(&text, &line).as_bytes(), true)?;
        Ok(Saved::Included)
    }

    /// A server was used now: first in the list, one more use. Written at once (a small file, a rare moment).
    pub fn touch(&mut self, i: usize, now: u64) {
        let Some(h) = self.hosts.get_mut(i) else { return };
        if now.saturating_sub(h.last) >= SAME_USE_SECS {
            h.uses += 1;
        }
        h.last = now;
        self.save_used();
    }

    pub fn save_used(&self) {
        if let Some(u) = &self.used {
            let _ = write_atomic(u, write_used(&self.hosts).as_bytes(), false);
        }
    }
}

/// Writes `path` through a temporary file beside it, so a reader never sees half of it; private to the user (ssh refuses
/// a config others can write). `ssh_dir`: the directory is `~/.ssh`, created private.
fn write_atomic(path: &Path, bytes: &[u8], ssh_dir: bool) -> io::Result<()> {
    if let Some(dir) = path.parent()
        && !dir.exists()
    {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        if ssh_dir {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let _ = ssh_dir;
    let tmp = path.with_extension("axon-tmp");
    std::fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
pub fn sample() -> Vec<Host> {
    let h = |alias: &str, hostname: &str, user: &str, port, identity: &str, last| Host {
        alias: alias.into(),
        hostname: hostname.into(),
        user: user.into(),
        port,
        identity: identity.into(),
        cmd: String::new(),
        last,
        uses: 0,
    };
    vec![
        h("web", "10.10.0.11", "deploy", 0, "~/.ssh/keys/dev.pem", 0),
        h("db", "10.10.0.12", "postgres", 0, "~/.ssh/keys/dev.pem", 0),
        h("staging", "10.20.0.11", "qa", 0, "~/.ssh/keys/qa.pem", 0),
        h("produccion", "192.168.1.100", "administrador", 2222, "~/.ssh/keys/servidor-prod.pem", 0),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_go_before_and_after_the_server_and_what_follows_it_is_a_command() {
        let a = args(&words("-i ~/.ssh/key.pem me@192.168.1.100 -p 2222"));
        assert_eq!((a.identity, a.target, a.port, a.command), (Some("~/.ssh/key.pem"), Some("me@192.168.1.100"), Some("2222"), false));
        let a = args(&words("-p2222 -luser -tA -o StrictHostKeyChecking=no web"));
        assert_eq!((a.port, a.user, a.target), (Some("2222"), Some("user"), Some("web")));
        let a = args(&words("web uptime -a"));
        assert_eq!((a.target, a.command), (Some("web"), true));
        let a = args(&words("-v -- -weird"));
        assert_eq!(a.target, Some("-weird"), "`--` ends the options");
        assert_eq!(args(&words("-p")).port, None, "a value that is not there");
    }

    #[test]
    fn a_session_is_an_ssh_to_a_server_with_nothing_to_run_there() {
        let d = |user: &str, host: &str, port| Some(Dest { user: user.into(), host: host.into(), port, identity: String::new() });
        assert_eq!(session("ssh web"), d("", "web", 0));
        assert_eq!(session("TERM=xterm /usr/bin/ssh -p 2222 me@10.0.0.1"), d("me", "10.0.0.1", 2222));
        assert_eq!(session("ssh -l root db"), d("root", "db", 0));
        assert_eq!(session("ssh ssh://me@host.example:2200"), d("me", "host.example", 2200));
        assert_eq!(session("SSH.EXE web"), d("", "web", 0));
        assert_eq!(session("ssh web uptime"), None, "runs and comes back");
        assert_eq!(session("ssh"), None);
        assert_eq!(session("ssh -v"), None);
        assert_eq!(session("sshfs web:/ x"), None);
        assert_eq!(session("ls web"), None);
    }

    #[test]
    fn an_address_has_a_user_or_dots() {
        for t in ["me@web", "10.0.0.1", "api.example.com", "a-b.c"] {
            assert!(looks_address(t), "{t}");
        }
        for t in ["web", "prod-web", "", ".", "a..b"] {
            assert!(!looks_address(t), "{t}");
        }
    }

    #[test]
    fn a_pasted_command_fills_the_form_and_proposes_a_name() {
        let f = pasted("ssh -i ~/.ssh/servidor-prod.pem administrador@192.168.1.100 -p 2222").unwrap();
        assert_eq!((f.hostname.as_str(), f.user.as_str(), f.port.as_str(), f.identity.as_str()), ("192.168.1.100", "administrador", "2222", "~/.ssh/servidor-prod.pem"));
        assert_eq!(suggest_alias(&f, &[], "server"), "servidor-prod");
        let f = pasted("  root@10.0.0.7 -p 22 ").unwrap();
        assert_eq!((f.port.as_str(), f.identity.as_str()), ("", ""), "22 is the default: nothing to write");
        assert_eq!(suggest_alias(&f, &[], "server"), "srv-7");
        assert_eq!(pasted("ssh web"), None, "an alias, not an address");
        assert_eq!(pasted("hello"), None);
        // From the address; taken names get a number.
        let f = Form { hostname: "api.example.com".into(), identity: "~/.ssh/id_rsa.pub".into(), ..Form::default() };
        assert_eq!(suggest_alias(&f, &[], "server"), "rsa", "id_ is dropped");
        let f = Form { hostname: "api.example.com".into(), ..Form::default() };
        let taken = vec![Host { alias: "api".into(), ..Host::default() }, Host { alias: "API-2".into(), ..Host::default() }];
        assert_eq!(suggest_alias(&f, &taken, "server"), "api-3");
        assert_eq!(suggest_alias(&Form::default(), &[], "server"), "server");
        let f = Form { identity: "~/.ssh/a-very-long-name-for-a-key.pem".into(), ..Form::default() };
        assert_eq!(suggest_alias(&f, &[], "server").chars().count(), 16);
    }

    #[test]
    fn the_form_says_what_keeps_it_from_being_saved_and_what_is_only_advice() {
        let hosts = sample();
        let form = |alias: &str, hostname: &str, port: &str| Form { alias: alias.into(), hostname: hostname.into(), port: port.into(), ..Form::default() };
        let e = |f: &Form| check(f, &hosts, None, "ivan").errors;
        assert_eq!(e(&form("", "", "")), vec![Problem::NoAlias, Problem::NoAddress]);
        assert_eq!(e(&form("my web", "x y", "")), vec![Problem::BadAlias, Problem::BadAddress]);
        assert_eq!(e(&form("WEB", "1.2.3.4", "")), vec![Problem::TakenAlias("WEB".into())]);
        assert_eq!(e(&form("ok", "1.2.3.4", "70000")), vec![Problem::BadPort]);
        assert_eq!(e(&form("ok", "1.2.3.4", "0")), vec![Problem::BadPort]);
        assert!(e(&form("ok", "1.2.3.4", "2222")).is_empty());
        // Its own alias is not taken while it is edited.
        assert!(check(&Form::of(&hosts[0]), &hosts, Some(0), "ivan").errors.is_empty());
        // Advice: a long alias, a connection saved under another name.
        let c = check(&form("a-very-long-alias-name", "1.2.3.4", ""), &hosts, None, "ivan");
        assert_eq!((c.errors.len(), c.warnings.clone()), (0, vec![Problem::LongAlias]));
        let f = Form { alias: "web2".into(), hostname: "10.10.0.11".into(), user: "deploy".into(), ..Form::default() };
        assert_eq!(check(&f, &hosts, None, "ivan").warnings, vec![Problem::SameAs("web".into())]);
    }

    #[test]
    fn the_list_puts_the_last_used_first_and_the_search_finds_aliases_initials_then_addresses() {
        let mut hosts = sample();
        hosts.push(Host { alias: "prod-web".into(), hostname: "203.0.113.5".into(), user: "ana".into(), ..Host::default() });
        hosts[2].last = 100;
        hosts[4].last = 50;
        let order = by_recent(&hosts);
        let names = |v: &[usize]| v.iter().map(|&i| hosts[i].alias.as_str()).collect::<Vec<_>>();
        assert_eq!(names(&order), ["staging", "prod-web", "db", "produccion", "web"]);
        assert_eq!(names(&filter(&hosts, &order, "")), names(&order));
        assert_eq!(names(&filter(&hosts, &order, "PROD")), ["prod-web", "produccion"]);
        assert_eq!(names(&filter(&hosts, &order, "pw")), ["prod-web"], "initials");
        assert_eq!(names(&filter(&hosts, &order, "Producción")), ["produccion"], "accents do not matter");
        assert_eq!(names(&filter(&hosts, &order, "10.10")), ["db", "web"], "no alias has it: the addresses");
        assert_eq!(names(&filter(&hosts, &order, "ana")), ["prod-web"], "... and the users");
        assert!(filter(&hosts, &order, "zzz").is_empty());
    }

    #[test]
    fn a_session_is_found_by_alias_or_by_its_address() {
        let hosts = sample();
        let d = |user: &str, host: &str, port| Dest { user: user.into(), host: host.into(), port, identity: String::new() };
        assert_eq!(find(&hosts, &d("", "WEB", 0)), Some(0));
        assert_eq!(find(&hosts, &d("root", "db", 0)), Some(1), "an alias with another user is still that server");
        assert_eq!(find(&hosts, &d("administrador", "192.168.1.100", 2222)), Some(3));
        assert_eq!(find(&hosts, &d("", "192.168.1.100", 2222)), Some(3));
        assert_eq!(find(&hosts, &d("", "192.168.1.100", 0)), None, "another port");
        assert_eq!(find(&hosts, &d("other", "10.10.0.11", 0)), None, "another user");
        assert_eq!(find(&hosts, &d("", "nowhere", 0)), None);
    }

    #[test]
    fn axons_file_reads_back_as_it_was_written() {
        let mut hosts = sample();
        hosts[0].cmd = "cd /var/www/app && ls -l | grep 100%".into();
        hosts[1].identity = "~/My Keys/db.pem".into();
        let text = write_config(&hosts);
        assert!(text.contains("Host produccion\n    HostName 192.168.1.100\n    User administrador\n    Port 2222\n"), "{text}");
        assert!(text.contains("    RequestTTY yes\n    RemoteCommand cd /var/www/app && ls -l | grep 100%%; exec $SHELL -l\n"), "{text}");
        assert!(text.contains("IdentityFile \"~/My Keys/db.pem\""), "{text}");
        let (found, issues) = read_config(&text);
        assert!(issues.is_empty(), "{issues:?}");
        let back: Vec<Host> = found.into_iter().map(|f| f.host).collect();
        assert_eq!(back, hosts.iter().map(|h| Host { last: 0, uses: 0, ..h.clone() }).collect::<Vec<_>>());
    }

    #[test]
    fn a_users_config_gives_its_servers_and_its_mistakes_with_their_lines() {
        let text = "\
Include ~/.config/axon/ssh_config
ServerAliveInterval 30

Host web web2   # two names, one block
    HostName 10.0.0.1
    User deploy
    HostName ignored.example
Host *.internal !x
    User nobody
Host github.com
    User git
Host broken
    HostName 10.0.0.9
    Port seventy
    Prot 22
Host web
    HostName 10.0.0.2
Match host foo
    User bar
Host empty
    User
";
        let (found, issues) = read_config(text);
        let got: Vec<(&str, &str, &str, usize, bool)> = found.iter().map(|f| (f.host.alias.as_str(), f.host.hostname.as_str(), f.host.user.as_str(), f.line, f.bad)).collect();
        assert_eq!(
            got,
            [("web", "10.0.0.1", "deploy", 4, false), ("web2", "10.0.0.1", "deploy", 4, false), ("github.com", "github.com", "git", 10, false), ("broken", "10.0.0.9", "", 12, true), ("empty", "empty", "", 20, false)]
        );
        let lines: Vec<(usize, Level)> = issues.iter().map(|i| (i.line, i.level)).collect();
        assert_eq!(lines, [(2, Level::Warn), (14, Level::Error), (15, Level::Warn), (16, Level::Warn), (21, Level::Error)], "{issues:?}");
        assert_eq!(issues[0].kind, IssueKind::Outside("ServerAliveInterval".into()));
        assert_eq!(issues[2].kind, IssueKind::Unknown("Prot".into()));
        assert_eq!(issues[3].kind, IssueKind::Repeated("web".into()));
        assert_eq!(issues[4].kind, IssueKind::NoValue("User".into()));
        // `IgnoreUnknown` keeps a keyword of another ssh quiet.
        let (_, issues) = read_config("IgnoreUnknown UseKeychain,Foo\nHost a\n    Foo yes\n");
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn the_include_goes_at_the_top_once() {
        let home = Path::new("/home/me");
        let line = include_line(Path::new("/home/me/.config/axon/ssh_config"), Some(home));
        assert_eq!(line, "Include ~/.config/axon/ssh_config");
        assert_eq!(include_line(Path::new("/opt/my cfg/ssh_config"), Some(home)), "Include \"/opt/my cfg/ssh_config\"");
        assert!(!includes("", &line));
        let text = with_include("Host a\n    HostName b\n", &line);
        assert!(text.starts_with("# The SSH servers of axon's panel (Alt+H).\nInclude ~/.config/axon/ssh_config\n\nHost a\n"), "{text}");
        assert!(includes(&text, &line));
        assert_eq!(with_include("", &line), "# The SSH servers of axon's panel (Alt+H).\nInclude ~/.config/axon/ssh_config\n");
        // Inside a block it applies to that block only: not included.
        assert!(!includes("Host a\n    Include ~/.config/axon/ssh_config\n", &line));
        assert!(includes("include   ~/.config/axon/ssh_config ~/other\nHost a\n", &line), "keywords are read without case");
    }

    #[test]
    fn uses_are_kept_by_alias_and_a_use_right_after_another_is_the_same() {
        let mut hosts = sample();
        hosts[1].last = 1000;
        hosts[1].uses = 3;
        let text = write_used(&hosts);
        assert_eq!(text, "db\t1000\t3\n");
        let mut fresh = sample();
        read_used(&format!("{text}nobody\t5\t5\nbad line\n"), &mut fresh);
        assert_eq!((fresh[1].last, fresh[1].uses, fresh[0].last), (1000, 3, 0));
        let mut b = Book::memory(fresh);
        b.touch(1, 1005);
        assert_eq!((b.hosts[1].last, b.hosts[1].uses), (1005, 3), "the panel's connect and the hook's word are one use");
        b.touch(1, 2000);
        assert_eq!((b.hosts[1].last, b.hosts[1].uses), (2000, 4));
        assert!(matches!(b.save(), Ok(Saved::Written)), "a book in memory writes nothing");
    }

    #[test]
    fn a_target_says_the_user_and_a_port_that_is_not_22() {
        let hosts = sample();
        assert_eq!(hosts[0].target("ivan"), "deploy@10.10.0.11");
        assert_eq!(hosts[3].target("ivan"), "administrador@192.168.1.100:2222");
        let h = Host { hostname: "x".into(), ..Host::default() };
        assert_eq!((h.target("ivan"), h.target("")), ("ivan@x".to_string(), "x".to_string()));
    }
}
