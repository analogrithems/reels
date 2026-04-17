//! Bundled resources and process spawning (testable pieces).

/// Which bundled markdown topic to show in [`HelpWindow`](crate::HelpWindow).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HelpDoc {
    Overview,
    Features,
    Keyboard,
    MediaFormats,
    Cli,
    ExternalAi,
    Developers,
    Agents,
    PhasesUi,
}

impl HelpDoc {
    pub fn title(self) -> &'static str {
        match self {
            HelpDoc::Overview => "Reel — Overview",
            HelpDoc::Features => "Reel — Features & roadmap",
            HelpDoc::Keyboard => "Reel — Keyboard shortcuts",
            HelpDoc::MediaFormats => "Reel — Media formats & tracks",
            HelpDoc::Cli => "Reel — CLI (reel-cli)",
            HelpDoc::ExternalAi => "Reel — External AI & tools",
            HelpDoc::Developers => "Reel — Developers",
            HelpDoc::Agents => "Reel — Agent guide (Cursor / Claude)",
            HelpDoc::PhasesUi => "Reel — UI phases roadmap",
        }
    }

    pub fn markdown(self) -> &'static str {
        match self {
            HelpDoc::Overview => {
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/HELP.md"))
            }
            HelpDoc::Features => {
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../docs/FEATURES.md"
                ))
            }
            HelpDoc::Keyboard => {
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../docs/KEYBOARD.md"
                ))
            }
            HelpDoc::MediaFormats => {
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../docs/MEDIA_FORMATS.md"
                ))
            }
            HelpDoc::Cli => {
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/CLI.md"))
            }
            HelpDoc::ExternalAi => {
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../docs/EXTERNAL_AI.md"
                ))
            }
            HelpDoc::Developers => {
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../docs/DEVELOPERS.md"
                ))
            }
            HelpDoc::Agents => {
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/AGENTS.md"))
            }
            HelpDoc::PhasesUi => {
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../docs/phases-ui.md"
                ))
            }
        }
    }
}

/// Window title and body for a help topic.
pub fn help_bundle(doc: HelpDoc) -> (&'static str, &'static str) {
    (doc.title(), doc.markdown())
}

/// Build a command that re-executes the current binary (for **New Window**).
pub fn new_window_command() -> std::process::Command {
    let exe = std::env::current_exe().expect("current_exe");
    std::process::Command::new(exe)
}

pub fn spawn_new_window() -> std::io::Result<std::process::Child> {
    new_window_command().spawn()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_bundle_all_topics_non_empty() {
        for doc in [
            HelpDoc::Overview,
            HelpDoc::Features,
            HelpDoc::Keyboard,
            HelpDoc::MediaFormats,
            HelpDoc::Cli,
            HelpDoc::ExternalAi,
            HelpDoc::Developers,
            HelpDoc::Agents,
            HelpDoc::PhasesUi,
        ] {
            let (title, body) = help_bundle(doc);
            assert!(title.len() > 6, "{doc:?}");
            assert!(body.len() > 64, "{doc:?}");
        }
    }

    #[test]
    fn overview_contains_reel() {
        assert!(HelpDoc::Overview.markdown().contains("Reel"));
    }

    #[test]
    fn agents_doc_mentions_cursor_and_claude() {
        let t = HelpDoc::Agents.markdown().to_lowercase();
        assert!(t.contains("cursor"));
        assert!(t.contains("claude"));
    }

    #[test]
    fn new_window_command_points_at_current_exe() {
        let exe = std::env::current_exe().expect("current_exe");
        let cmd = new_window_command();
        assert_eq!(cmd.get_program(), exe.as_os_str());
    }
}
