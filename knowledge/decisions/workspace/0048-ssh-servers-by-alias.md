---
type: decision
status: accepted
updated: 2026-11-27
related: [0026-docked-settings-panel.md, 0027-command-palette.md, 0034-a-group-is-the-split.md, 0044-an-agent-has-its-mark-not-a-spinner.md, ../interface/0036-the-reference-cleaner.md]
---
# 0048 · SSH servers by alias: a docked panel over an OpenSSH file that `~/.ssh/config` includes

**Decision:** the reference's servers manager (`Alt+H`, `#btn-hosts`) is a panel docked where the settings go (one of the
two at a time), over a file of axon's own in OpenSSH syntax, `ssh_config` beside `config.toml`. The first save puts one line,
`Include ~/.config/axon/ssh_config`, at the top of `~/.ssh/config` (made private if missing; a symlink is written through), so
`ssh web` works in every shell and program, inside axon or not, and the shell's completion offers the aliases. Nothing else in
`~/.ssh/config` is ever rewritten. When each server was used, and how often, is state: `ssh-used` in the state directory.

- **Model** (`ssh.rs`, pure): ssh's options as ssh reads them (`-p2222`, `-l`, options after the server, `--`, `ssh://`), a
  pasted command filling the form, the proposed alias (key file, `srv-<octet>`, the name's first label, `-2`), the form's
  errors and advice, the search (alias, initials, then address and user), `read_config` with line numbers (every `ssh_config(5)`
  keyword known, `IgnoreUnknown` honoured, several names per `Host`, no `HostName` meaning the alias, as ssh does).
- **Panel** (`hosts.rs`): list, edit, import, export, as the reference lays them out; Import reads `~/.ssh/config` itself (the
  reference has it pasted: a browser cannot read files). Effects go to the app (`wlapp/hosts_ui.rs`), which writes.
- **Sessions:** the hook already sends the command line (`L`); `ssh::session` turns `ssh web` into where it goes, kept in
  `History::remote` (only the server and user, never the rest of the line). Such a window is labelled with the alias, shows a
  cyan dot (`side::Status::Remote`) instead of a spinner, and lights its server green in the panel. Connecting brings that window
  on screen, else types `ssh <alias>` into the focused pane when its prompt is empty (`view::input_empty`), else opens a window
  in the group *Servers* (cyan; a group is the split, ADR 0034). An ssh to an address with no alias offers *Save as …* in a
  toast; one that has an alias says so (the reference's block footers).
- **On arrival** is `RequestTTY yes` and `RemoteCommand <cmd>; exec $SHELL -l`, so the session goes on after it (`%` doubled).
- The palette lists *SSH servers* after the overview and `ssh <alias>` for each server under *Servers*.

**Why:** the reference's aliases only help if the shell knows them; an `Include` is how OrbStack and others add hosts without
owning the user's file. Cost (`scripts/bench.sh`, 2026-11-27): binary +195.7 KB (8,394,712 to 8,590,384 B; the panel 80.7 KB,
`ssh.rs` 30.6, the app's side 26.9, generics and strings the rest; the settings panel is 86.6 KB); panel open 22.6 MB as the
settings, 0 ticks idle; nothing on the PTY or paint path when it is closed.

**Rejected:**
- Editing `~/.ssh/config` in place: rewriting hand-written blocks and comments.
- A file only axon reads (`ssh -F`): `ssh web` typed in a shell would not work.
- Short aliases when typed (`ssh pw` for `prod-web`): needs an `ssh` function in the hook; the user chose not to (2026-11-27).
  The panel's search and the palette find by prefix and initials.
- The reference's 94 % floating panel on narrow windows: axon docks; narrow windows get its two-by-two footer only.
- The remote prompt's chips: the remote shell has no hook to mark its prompt.

**Revisit if:** the hook wraps `ssh` (short aliases), or a remote shell can carry axon's hook.
