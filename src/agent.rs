//! The AI agents that run in a terminal (Claude Code, Codex, Antigravity): a window that runs one is not busy the way a
//! build is. `claude` starts and does not end until the user quits it, so for the shell's hook it is a command that runs
//! for hours, and a spinner that turns that long says nothing (and keeps a timer going). The window says which agent it
//! is instead, with its mark, and nothing turns. Whether the agent works or waits for the user is in its title: Claude
//! Code puts a mark before it (`State::of_title`), and the mark of the window says it (ADR 0046).
//!
//! Pure: the name of the command in, the agent out; a title in, its state out. The marks are drawn by `icons::agent`;
//! the state that holds the name is `history::History::command`, the one that holds the title the pane's.

/// An agent axon knows by the command that starts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    /// Claude Code (`claude`).
    Claude,
    /// OpenAI's Codex (`codex`).
    Codex,
    /// Google's Antigravity (`agy`).
    Antigravity,
}

impl Agent {
    /// Every agent, in the order the design board lists them.
    #[cfg(test)]
    pub const ALL: [Agent; 3] = [Agent::Claude, Agent::Codex, Agent::Antigravity];

    /// The agent that runs when the command is called `word` (a command's first word, as `history::command_word` gives
    /// it): the name alone, in any case, without a directory or an `.exe`-like ending.
    pub fn of(word: &str) -> Option<Agent> {
        let name = word.rsplit(['/', '\\']).next().unwrap_or(word);
        let name = name.rsplit_once('.').filter(|(_, ext)| matches!(*ext, "exe" | "cmd" | "bat" | "ps1")).map_or(name, |(stem, _)| stem);
        match name.to_ascii_lowercase().as_str() {
            "claude" => Some(Agent::Claude),
            "codex" | "chatgpt" => Some(Agent::Codex),
            "agy" | "antigravity" => Some(Agent::Antigravity),
            _ => None,
        }
    }

    /// How the agent is called, for the tooltip-sized places that spell it out (the overview's card).
    #[allow(dead_code)]
    pub fn name(self) -> &'static str {
        match self {
            Agent::Claude => "Claude",
            Agent::Codex => "Codex",
            Agent::Antigravity => "Antigravity",
        }
    }
}

/// What an agent is doing, as its title says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum State {
    /// Its title says nothing (an agent that sets none, or not yet).
    #[default]
    Unknown,
    /// It works on what it was asked: the user can look elsewhere.
    Working,
    /// It waits for the user: at its prompt, or asking a question or a permission.
    Waiting,
}

impl State {
    /// The state a program's title says with its first character, the mark Claude Code puts before it: `◐` and `◑` in
    /// turn while it is busy, `✳` the rest of the time (at its prompt, or asking something: the user's turn). Inside
    /// tmux or screen it writes `✳` always, busy or not: there it reads as waiting.
    pub fn of_title(t: &str) -> State {
        match t.chars().next() {
            Some('\u{2733}') => State::Waiting,
            Some('\u{25d0}' | '\u{25d1}') => State::Working,
            _ => State::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_commands_that_start_each_agent_are_known() {
        assert_eq!(Agent::of("claude"), Some(Agent::Claude));
        assert_eq!(Agent::of("codex"), Some(Agent::Codex));
        assert_eq!(Agent::of("chatgpt"), Some(Agent::Codex));
        assert_eq!(Agent::of("agy"), Some(Agent::Antigravity));
        assert_eq!(Agent::of("antigravity"), Some(Agent::Antigravity));
    }

    #[test]
    fn case_directories_and_windows_endings_do_not_matter() {
        assert_eq!(Agent::of("Claude"), Some(Agent::Claude));
        assert_eq!(Agent::of("/home/ana/.local/bin/claude"), Some(Agent::Claude));
        assert_eq!(Agent::of("claude.exe"), Some(Agent::Claude));
        assert_eq!(Agent::of(r"C:\Users\ana\AppData\Roaming\npm\codex.cmd"), Some(Agent::Codex));
        assert_eq!(Agent::of("agy.ps1"), Some(Agent::Antigravity));
    }

    #[test]
    fn other_commands_are_not_agents() {
        for w in ["", "cargo", "vim", "claudette", "claude-code-router", "xclaude", "agyx", "codex.txt", "bash"] {
            assert_eq!(Agent::of(w), None, "{w:?}");
        }
    }

    #[test]
    fn claude_codes_title_mark_says_whether_it_works_or_waits() {
        assert_eq!(State::of_title("✳ Fix the icons"), State::Waiting);
        assert_eq!(State::of_title("✳ Claude Code"), State::Waiting);
        assert_eq!(State::of_title("◐ Fix the icons"), State::Working);
        assert_eq!(State::of_title("◑ Fix the icons"), State::Working);
        for t in ["", "Fix the icons", "vim main.rs", "* notes", "> task", "⠂ build"] {
            assert_eq!(State::of_title(t), State::Unknown, "{t:?}");
        }
    }

    #[test]
    fn every_agent_has_a_name() {
        for a in Agent::ALL {
            assert!(!a.name().is_empty());
        }
    }
}
