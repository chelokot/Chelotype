use portable_pty::CommandBuilder;

const FISH_CANDIDATES: [&str; 2] = ["/usr/bin/fish", "/bin/fish"];

pub fn default_shell_command() -> CommandBuilder {
    CommandBuilder::new(default_shell_path())
}

pub fn default_shell_path() -> String {
    resolve_default_shell(
        std::env::var("CHELOTYPE_SHELL").ok().as_deref(),
        std::env::var("SHELL").ok().as_deref(),
        |path| std::path::Path::new(path).is_file(),
    )
}

fn resolve_default_shell(
    configured: Option<&str>,
    environment_shell: Option<&str>,
    exists: impl Fn(&str) -> bool,
) -> String {
    if let Some(shell) = configured.filter(|shell| !shell.trim().is_empty()) {
        return shell.to_string();
    }
    if let Some(shell) = FISH_CANDIDATES.into_iter().find(|path| exists(path)) {
        return shell.to_string();
    }
    environment_shell
        .filter(|shell| !shell.trim().is_empty())
        .unwrap_or("/bin/bash")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_shell_overrides_product_default() {
        assert_eq!(
            resolve_default_shell(Some("/usr/bin/zsh"), Some("/bin/bash"), |_| true),
            "/usr/bin/zsh"
        );
    }

    #[test]
    fn fish_is_default_even_when_environment_shell_is_zsh() {
        assert_eq!(
            resolve_default_shell(None, Some("/usr/bin/zsh"), |path| path == "/usr/bin/fish"),
            "/usr/bin/fish"
        );
    }

    #[test]
    fn environment_shell_is_fallback_when_fish_is_unavailable() {
        assert_eq!(
            resolve_default_shell(None, Some("/usr/bin/zsh"), |_| false),
            "/usr/bin/zsh"
        );
    }

    #[test]
    fn bash_is_last_resort() {
        assert_eq!(resolve_default_shell(None, None, |_| false), "/bin/bash");
    }
}
