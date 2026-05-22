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

const FISH_CHELOTYPE_INIT: &str = r#"functions -q fish_prompt; and functions -c fish_prompt __chelotype_user_fish_prompt
function fish_prompt
    printf '\e]133;A\e\\'
    __chelotype_user_fish_prompt
    printf '\e]133;B\e\\'
end
function __chelotype_decode_line
    set -l encoded (string sub -s 2 -- $argv[1] | string collect)
    test -n "$encoded"; or return 0
    printf '%s' "$encoded" | string unescape --style=var | string collect --allow-empty
end
function __chelotype_encode_line
    printf 'x%s' (string escape --style=var -- $argv[1])
end
function __chelotype_slice --argument-names value start length
    test $length -gt 0; or return
    string sub -s $start -l $length -- "$value"
end
function __chelotype_common_prefix --argument-names old_line new_line
    set -l old_len (string length -- "$old_line")
    set -l new_len (string length -- "$new_line")
    set -l prefix 0
    while test $prefix -lt $old_len; and test $prefix -lt $new_len
        set -l index (math $prefix + 1)
        test (string sub -s $index -l 1 -- "$old_line") = (string sub -s $index -l 1 -- "$new_line"); or break
        set prefix (math $prefix + 1)
    end
    echo $prefix
end
function __chelotype_common_suffix --argument-names old_line new_line prefix
    set -l old_len (string length -- "$old_line")
    set -l new_len (string length -- "$new_line")
    set -l suffix 0
    while test (math $prefix + $suffix) -lt $old_len; and test (math $prefix + $suffix) -lt $new_len
        set -l old_index (math $old_len - $suffix)
        set -l new_index (math $new_len - $suffix)
        test (string sub -s $old_index -l 1 -- "$old_line") = (string sub -s $new_index -l 1 -- "$new_line"); or break
        set suffix (math $suffix + 1)
    end
    echo $suffix
end
function __chelotype_clear_redo
    set -e __chelotype_redo_prefixes
    set -e __chelotype_redo_suffixes
    set -e __chelotype_redo_old_mids
    set -e __chelotype_redo_new_mids
    set -e __chelotype_redo_old_cursors
    set -e __chelotype_redo_new_cursors
end
function __chelotype_push_encoded_patch --argument-names stack prefix suffix old_mid new_mid old_cursor new_cursor
    if test "$stack" = undo
        set -g __chelotype_undo_prefixes $__chelotype_undo_prefixes $prefix
        set -g __chelotype_undo_suffixes $__chelotype_undo_suffixes $suffix
        set -g __chelotype_undo_old_mids $__chelotype_undo_old_mids $old_mid
        set -g __chelotype_undo_new_mids $__chelotype_undo_new_mids $new_mid
        set -g __chelotype_undo_old_cursors $__chelotype_undo_old_cursors $old_cursor
        set -g __chelotype_undo_new_cursors $__chelotype_undo_new_cursors $new_cursor
    else
        set -g __chelotype_redo_prefixes $__chelotype_redo_prefixes $prefix
        set -g __chelotype_redo_suffixes $__chelotype_redo_suffixes $suffix
        set -g __chelotype_redo_old_mids $__chelotype_redo_old_mids $old_mid
        set -g __chelotype_redo_new_mids $__chelotype_redo_new_mids $new_mid
        set -g __chelotype_redo_old_cursors $__chelotype_redo_old_cursors $old_cursor
        set -g __chelotype_redo_new_cursors $__chelotype_redo_new_cursors $new_cursor
    end
end
function __chelotype_push_patch --argument-names stack prefix suffix old_mid new_mid old_cursor new_cursor
    __chelotype_push_encoded_patch $stack $prefix $suffix (__chelotype_encode_line "$old_mid") (__chelotype_encode_line "$new_mid") $old_cursor $new_cursor
end
function __chelotype_finalize_pending_undo
    set -q __chelotype_pending_line; or return 1
    set -l old_line (__chelotype_decode_line $__chelotype_pending_line)
    set -l old_cursor $__chelotype_pending_cursor
    set -l new_line (commandline)
    set -l new_cursor (commandline -C)
    set -e __chelotype_pending_line
    set -e __chelotype_pending_cursor
    test "$old_line" != "$new_line"; or test "$old_cursor" != "$new_cursor"; or return 0
    set -l prefix (__chelotype_common_prefix "$old_line" "$new_line")
    set -l suffix (__chelotype_common_suffix "$old_line" "$new_line" $prefix)
    set -l old_len (string length -- "$old_line")
    set -l new_len (string length -- "$new_line")
    set -l old_mid_len (math $old_len - $prefix - $suffix)
    set -l new_mid_len (math $new_len - $prefix - $suffix)
    set -l mid_start (math $prefix + 1)
    set -l old_mid (__chelotype_slice "$old_line" $mid_start $old_mid_len)
    set -l new_mid (__chelotype_slice "$new_line" $mid_start $new_mid_len)
    __chelotype_push_patch undo $prefix $suffix "$old_mid" "$new_mid" $old_cursor $new_cursor
    return 0
end
function __chelotype_apply_patch --argument-names prefix suffix next_mid next_cursor
    set -l line (commandline)
    set -l line_len (string length -- "$line")
    set -l before (__chelotype_slice "$line" 1 $prefix)
    set -l after_start (math $line_len - $suffix + 1)
    set -l after (__chelotype_slice "$line" $after_start $suffix)
    set -l replacement (__chelotype_decode_line $next_mid)
    commandline --replace "$before$replacement$after"
    commandline -C $next_cursor
    commandline -f repaint
end
function __chelotype_capture_undo
    __chelotype_finalize_pending_undo
    set -g __chelotype_pending_line (__chelotype_encode_line (commandline))
    set -g __chelotype_pending_cursor (commandline -C)
    __chelotype_clear_redo
end
function __chelotype_undo
    __chelotype_finalize_pending_undo
    set -l count (count $__chelotype_undo_prefixes)
    test $count -gt 0; or return
    set -l prefix $__chelotype_undo_prefixes[$count]
    set -l suffix $__chelotype_undo_suffixes[$count]
    set -l old_mid $__chelotype_undo_old_mids[$count]
    set -l new_mid $__chelotype_undo_new_mids[$count]
    set -l old_cursor $__chelotype_undo_old_cursors[$count]
    set -l new_cursor $__chelotype_undo_new_cursors[$count]
    set -e __chelotype_undo_prefixes[$count]
    set -e __chelotype_undo_suffixes[$count]
    set -e __chelotype_undo_old_mids[$count]
    set -e __chelotype_undo_new_mids[$count]
    set -e __chelotype_undo_old_cursors[$count]
    set -e __chelotype_undo_new_cursors[$count]
    __chelotype_push_encoded_patch redo $prefix $suffix $old_mid $new_mid $old_cursor $new_cursor
    __chelotype_apply_patch $prefix $suffix $old_mid $old_cursor
end
function __chelotype_redo
    if set -q __chelotype_pending_line
        __chelotype_finalize_pending_undo
        __chelotype_clear_redo
        return
    end
    set -l count (count $__chelotype_redo_prefixes)
    test $count -gt 0; or return
    set -l prefix $__chelotype_redo_prefixes[$count]
    set -l suffix $__chelotype_redo_suffixes[$count]
    set -l old_mid $__chelotype_redo_old_mids[$count]
    set -l new_mid $__chelotype_redo_new_mids[$count]
    set -l old_cursor $__chelotype_redo_old_cursors[$count]
    set -l new_cursor $__chelotype_redo_new_cursors[$count]
    set -e __chelotype_redo_prefixes[$count]
    set -e __chelotype_redo_suffixes[$count]
    set -e __chelotype_redo_old_mids[$count]
    set -e __chelotype_redo_new_mids[$count]
    set -e __chelotype_redo_old_cursors[$count]
    set -e __chelotype_redo_new_cursors[$count]
    __chelotype_push_encoded_patch undo $prefix $suffix $old_mid $new_mid $old_cursor $new_cursor
    __chelotype_apply_patch $prefix $suffix $new_mid $new_cursor
end
function __chelotype_discard_pending_undo --on-event fish_preexec
    set -e __chelotype_pending_line
    set -e __chelotype_pending_cursor
end
function __chelotype_move_cursor_to_target
    set -l target_file $CHELOTYPE_CURSOR_TARGET_FILE
    test -n "$target_file"; or return
    test -f "$target_file"; or return
    set -l target (string trim < "$target_file")
    string match -qr '^[0-9]+$' -- $target; or return
    commandline -C $target
end
bind \e\[57344u __chelotype_capture_undo
bind -M insert \e\[57344u __chelotype_capture_undo
bind \e\[57345u __chelotype_undo
bind -M insert \e\[57345u __chelotype_undo
bind \e\[57346u __chelotype_redo
bind -M insert \e\[57346u __chelotype_redo
bind \e\[57347u __chelotype_move_cursor_to_target
bind -M insert \e\[57347u __chelotype_move_cursor_to_target"#;

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
    use std::process::Command;

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
    fn fish_undo_stack_stores_compact_insert_patch() {
        let Some(fish_path) = FISH_CANDIDATES
            .into_iter()
            .find(|path| std::path::Path::new(path).is_file())
        else {
            eprintln!("skipping fish undo patch test because fish is not installed");
            return;
        };

        let output = Command::new(fish_path)
            .args([
                "--init-command",
                FISH_CHELOTYPE_INIT,
                "-ic",
                r#"
set -l base (string repeat -n 1024 a)
set -l paste (string repeat -n 64 b)
commandline --replace $base
commandline -C (string length -- $base)
__chelotype_capture_undo
commandline --insert $paste
__chelotype_capture_undo
set -l old_mid (__chelotype_decode_line $__chelotype_undo_old_mids[1])
set -l new_mid (__chelotype_decode_line $__chelotype_undo_new_mids[1])
printf '%s %s %s %s %s\n' (count $__chelotype_undo_prefixes) $__chelotype_undo_prefixes[1] $__chelotype_undo_suffixes[1] (string length -- "$old_mid") (string length -- "$new_mid")
__chelotype_undo
printf '%s %s\n' (string length -- (commandline)) (commandline -C)
__chelotype_redo
printf '%s %s\n' (string length -- (commandline)) (commandline -C)
"#,
            ])
            .output()
            .expect("run fish undo patch check");

        assert!(
            output.status.success(),
            "fish undo patch check failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "1 1024 0 0 64\n1024 1024\n1088 1088\n"
        );
    }

    #[test]
    fn fish_undo_pending_edit_does_not_cross_command_execution() {
        let Some(fish_path) = FISH_CANDIDATES
            .into_iter()
            .find(|path| std::path::Path::new(path).is_file())
        else {
            eprintln!("skipping fish undo preexec test because fish is not installed");
            return;
        };

        let output = Command::new(fish_path)
            .args([
                "--init-command",
                FISH_CHELOTYPE_INIT,
                "-ic",
                r#"
commandline --replace abc
commandline -C 3
__chelotype_capture_undo
commandline --insert def
emit fish_preexec
commandline --replace ''
__chelotype_capture_undo
printf '%s\n' (count $__chelotype_undo_prefixes)
"#,
            ])
            .output()
            .expect("run fish undo preexec check");

        assert!(
            output.status.success(),
            "fish undo preexec check failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout), "0\n");
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
