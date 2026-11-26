//! The AI agents that run in a terminal (Claude Code, Codex, Antigravity): a window that runs one is not busy the way a
//! build is. `claude` starts and does not end until the user quits it, so for the shell's hook it is a command that runs
//! for hours, and a spinner that turns that long says nothing (and keeps a timer going). The window says which agent it
//! is instead, with its mark, and nothing turns.
//!
//! Pure: the name of the command in, the agent out. The marks are drawn by `icons::agent`; the state that holds the name
//! is `history::History::command`.

/// An agent axon knows by the command that starts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    /// Claude Code (`claude`).
    Claude,
    /// ChatGPT's terminal agent (`codex`).
    ChatGpt,
    /// Google's Antigravity (`agy`).
    Google,
}

impl Agent {
    /// Every agent, in the order the design board lists them.
    #[cfg(test)]
    pub const ALL: [Agent; 3] = [Agent::Claude, Agent::ChatGpt, Agent::Google];

    /// The agent that runs when the command is called `word` (a command's first word, as `history::command_word` gives
    /// it): the name alone, in any case, without a directory or an `.exe`-like ending.
    pub fn of(word: &str) -> Option<Agent> {
        let name = word.rsplit(['/', '\\']).next().unwrap_or(word);
        let name = name.rsplit_once('.').filter(|(_, ext)| matches!(*ext, "exe" | "cmd" | "bat" | "ps1")).map_or(name, |(stem, _)| stem);
        match name.to_ascii_lowercase().as_str() {
            "claude" => Some(Agent::Claude),
            "codex" | "chatgpt" => Some(Agent::ChatGpt),
            "agy" | "antigravity" => Some(Agent::Google),
            _ => None,
        }
    }

    /// How the agent is called, for the tooltip-sized places that spell it out (the overview's card).
    #[allow(dead_code)]
    pub fn name(self) -> &'static str {
        match self {
            Agent::Claude => "Claude",
            Agent::ChatGpt => "ChatGPT",
            Agent::Google => "Antigravity",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_commands_that_start_each_agent_are_known() {
        assert_eq!(Agent::of("claude"), Some(Agent::Claude));
        assert_eq!(Agent::of("codex"), Some(Agent::ChatGpt));
        assert_eq!(Agent::of("chatgpt"), Some(Agent::ChatGpt));
        assert_eq!(Agent::of("agy"), Some(Agent::Google));
        assert_eq!(Agent::of("antigravity"), Some(Agent::Google));
    }

    #[test]
    fn case_directories_and_windows_endings_do_not_matter() {
        assert_eq!(Agent::of("Claude"), Some(Agent::Claude));
        assert_eq!(Agent::of("/home/ana/.local/bin/claude"), Some(Agent::Claude));
        assert_eq!(Agent::of("claude.exe"), Some(Agent::Claude));
        assert_eq!(Agent::of(r"C:\Users\ana\AppData\Roaming\npm\codex.cmd"), Some(Agent::ChatGpt));
        assert_eq!(Agent::of("agy.ps1"), Some(Agent::Google));
    }

    #[test]
    fn other_commands_are_not_agents() {
        for w in ["", "cargo", "vim", "claudette", "claude-code-router", "xclaude", "agyx", "codex.txt", "bash"] {
            assert_eq!(Agent::of(w), None, "{w:?}");
        }
    }

    #[test]
    fn every_agent_has_a_name() {
        for a in Agent::ALL {
            assert!(!a.name().is_empty());
        }
    }
}
