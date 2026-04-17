//! Bundled resources and process spawning (testable pieces).

/// Markdown source for the in-app help window (rendered as wrapped text for now).
pub fn bundled_help_markdown() -> &'static str {
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/HELP.md"))
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
    fn help_bundle_non_empty() {
        assert!(bundled_help_markdown().contains("Reel"));
    }

    #[test]
    fn new_window_command_points_at_current_exe() {
        let exe = std::env::current_exe().expect("current_exe");
        let cmd = new_window_command();
        assert_eq!(cmd.get_program(), exe.as_os_str());
    }
}
