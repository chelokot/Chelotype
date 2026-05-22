use crate::backend::ScreenSize;
use portable_pty::CommandBuilder;

const FISH_CANDIDATES: [&str; 2] = ["/usr/bin/fish", "/bin/fish"];
pub const INPUT_UNDO_CAPTURE_SEQUENCE: &[u8] = b"\x1b[57344u";
pub const INPUT_UNDO_SEQUENCE: &[u8] = b"\x1b[57345u";
pub const INPUT_REDO_SEQUENCE: &[u8] = b"\x1b[57346u";
pub const INPUT_CURSOR_TARGET_SEQUENCE: &[u8] = b"\x1b[57347u";
pub const INPUT_CURSOR_BRIDGE_ENV: &str = "CHELOTYPE_INPUT_CURSOR_BRIDGE";
pub const INPUT_CURSOR_BRIDGE_FISH: &str = "fish";
pub const INPUT_CURSOR_TARGET_FILE_ENV: &str = "CHELOTYPE_CURSOR_TARGET_FILE";
pub const INPUT_CURSOR_TARGET_FILE_PLACEHOLDER: &str = "__CHELOTYPE_CURSOR_TARGET_FILE__";

const FISH_CHELOTYPE_INIT: &str = "\
functions -q fish_prompt; and functions -c fish_prompt __chelotype_user_fish_prompt
function fish_prompt
    printf '\\e]133;A\\e\\\\'
    __chelotype_user_fish_prompt
    printf '\\e]133;B\\e\\\\'
end
function __chelotype_capture_undo
    set -g __chelotype_undo_lines $__chelotype_undo_lines x(string escape --style=var -- (commandline))
    set -g __chelotype_undo_cursors $__chelotype_undo_cursors (commandline -C)
    set -e __chelotype_redo_lines
    set -e __chelotype_redo_cursors
end
function __chelotype_decode_line
    string sub -s 2 -- $argv[1] | string unescape --style=var
end
function __chelotype_undo
    set -l count (count $__chelotype_undo_lines)
    test $count -gt 0; or return
    set -g __chelotype_redo_lines $__chelotype_redo_lines x(string escape --style=var -- (commandline))
    set -g __chelotype_redo_cursors $__chelotype_redo_cursors (commandline -C)
    commandline --replace (__chelotype_decode_line $__chelotype_undo_lines[$count])
    commandline -C $__chelotype_undo_cursors[$count]
    set -e __chelotype_undo_lines[$count]
    set -e __chelotype_undo_cursors[$count]
    commandline -f repaint
end
function __chelotype_redo
    set -l count (count $__chelotype_redo_lines)
    test $count -gt 0; or return
    set -g __chelotype_undo_lines $__chelotype_undo_lines x(string escape --style=var -- (commandline))
    set -g __chelotype_undo_cursors $__chelotype_undo_cursors (commandline -C)
    commandline --replace (__chelotype_decode_line $__chelotype_redo_lines[$count])
    commandline -C $__chelotype_redo_cursors[$count]
    set -e __chelotype_redo_lines[$count]
    set -e __chelotype_redo_cursors[$count]
    commandline -f repaint
end
function __chelotype_move_cursor_to_target
    set -l target_file $CHELOTYPE_CURSOR_TARGET_FILE
    test -n \"$target_file\"; or return
    test -f \"$target_file\"; or return
    set -l target (string trim < \"$target_file\")
    string match -qr '^[0-9]+$' -- $target; or return
    commandline -C $target
    commandline -f repaint
end
bind \\e\\[57344u __chelotype_capture_undo
bind -M insert \\e\\[57344u __chelotype_capture_undo
bind \\e\\[57345u __chelotype_undo
bind -M insert \\e\\[57345u __chelotype_undo
bind \\e\\[57346u __chelotype_redo
bind -M insert \\e\\[57346u __chelotype_redo
bind \\e\\[57347u __chelotype_move_cursor_to_target
bind -M insert \\e\\[57347u __chelotype_move_cursor_to_target";

pub fn default_shell_command() -> CommandBuilder {
    default_shell_command_with_size(None)
}

pub fn default_shell_command_with_size(size: Option<ScreenSize>) -> CommandBuilder {
    shell_command_for_path_with_size(&default_shell_path(), size)
}

pub fn default_shell_argv() -> Vec<String> {
    shell_argv_for_path(&default_shell_path())
}

pub fn default_shell_command_line() -> String {
    default_shell_argv()
        .iter()
        .map(|argument| shell_quote(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn default_shell_has_input_edit_bridge() -> bool {
    is_fish_path(&default_shell_path())
}

pub fn default_shell_path() -> String {
    resolve_default_shell(
        std::env::var("CHELOTYPE_SHELL").ok().as_deref(),
        crate::host::environment_value("SHELL").as_deref(),
        crate::host::path_is_file,
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

fn shell_command_for_path_with_size(path: &str, size: Option<ScreenSize>) -> CommandBuilder {
    let argv = shell_argv_for_path(path);
    let mut command = crate::host::command_builder_with_size(&argv[0], size);
    command.args(&argv[1..]);
    if is_fish_path(path) {
        command.env(INPUT_CURSOR_BRIDGE_ENV, INPUT_CURSOR_BRIDGE_FISH);
    }
    command
}

fn shell_argv_for_path(path: &str) -> Vec<String> {
    if is_fish_path(path) {
        return vec![
            path.to_string(),
            "--init-command".to_string(),
            FISH_CHELOTYPE_INIT.to_string(),
        ];
    }
    vec![path.to_string()]
}

fn is_fish_path(path: &str) -> bool {
    std::path::Path::new(path)
        .file_name()
        .is_some_and(|name| name == "fish")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

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

    #[test]
    fn fish_command_installs_chelotype_undo_redo_bindings() {
        let command = shell_command_for_path_with_size("/usr/bin/fish", None);
        let argv = command
            .get_argv()
            .iter()
            .map(|arg| arg.as_os_str())
            .collect::<Vec<_>>();

        assert_eq!(
            argv,
            vec![
                OsStr::new("/usr/bin/fish"),
                OsStr::new("--init-command"),
                OsStr::new(FISH_CHELOTYPE_INIT),
            ]
        );
        assert_eq!(
            command.get_env(INPUT_CURSOR_BRIDGE_ENV),
            Some(OsStr::new("fish"))
        );
    }

    #[test]
    fn non_fish_command_is_not_modified() {
        let command = shell_command_for_path_with_size("/bin/bash", None);
        let argv = command
            .get_argv()
            .iter()
            .map(|arg| arg.as_os_str())
            .collect::<Vec<_>>();

        assert_eq!(argv, vec![OsStr::new("/bin/bash")]);
        assert_eq!(command.get_env(INPUT_CURSOR_BRIDGE_ENV), None);
    }

    #[test]
    fn shell_command_line_quotes_fish_init_for_container_exec() {
        let line = shell_argv_for_path("/usr/bin/fish")
            .iter()
            .map(|argument| shell_quote(argument))
            .collect::<Vec<_>>()
            .join(" ");

        assert!(line.starts_with("'/usr/bin/fish' '--init-command' 'functions -q fish_prompt"));
        assert!(line.contains("__chelotype_user_fish_prompt"));
        assert!(line.contains("__chelotype_redo"));
        assert!(line.contains("__chelotype_move_cursor_to_target"));
    }

    #[test]
    #[serial_test::serial]
    fn flatpak_shell_command_runs_host_shell_through_flatpak_spawn() {
        crate::host::set_flatpak_test_override(Some(true));
        unsafe {
            std::env::set_var("CHELOTYPE_SHELL", "/bin/sh");
        }

        let command = default_shell_command();
        let argv = command
            .get_argv()
            .iter()
            .map(|arg| arg.as_os_str())
            .collect::<Vec<_>>();

        assert_eq!(
            argv,
            vec![
                OsStr::new("flatpak-spawn"),
                OsStr::new("--host"),
                OsStr::new("/bin/sh"),
            ]
        );

        unsafe {
            std::env::remove_var("CHELOTYPE_SHELL");
        }
        crate::host::set_flatpak_test_override(None);
    }
}
