use crate::backend::ScreenSize;
use portable_pty::CommandBuilder;
use std::path::{Path, PathBuf};
use std::process::Command;

const HOST_ROOT: &str = "/var/run/host";

#[cfg(test)]
thread_local! {
    static FLATPAK_TEST_OVERRIDE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    static ENVIRONMENT_VALUE_TEST_OVERRIDE: std::cell::RefCell<Option<(String, String)>> = const { std::cell::RefCell::new(None) };
}

pub fn command_builder(program: &str) -> CommandBuilder {
    command_builder_with_size(program, None)
}

pub fn command_builder_with_size(program: &str, size: Option<ScreenSize>) -> CommandBuilder {
    if is_flatpak() {
        let mut command = CommandBuilder::new("flatpak-spawn");
        command.arg("--host");
        if let Some(size) = size {
            command.arg(format!("--env=COLUMNS={}", size.cols));
            command.arg(format!("--env=LINES={}", size.rows));
        }
        command.arg(program);
        return command;
    }
    CommandBuilder::new(program)
}

pub fn command(program: &str) -> Command {
    if is_flatpak() {
        let mut command = Command::new("flatpak-spawn");
        command.arg("--host");
        command.arg(program);
        return command;
    }
    Command::new(program)
}

pub fn environment_value(key: &str) -> Option<String> {
    #[cfg(test)]
    if let Some(value) = ENVIRONMENT_VALUE_TEST_OVERRIDE.with(|override_value| {
        override_value
            .borrow()
            .as_ref()
            .filter(|(override_key, _)| override_key == key)
            .map(|(_, value)| value.clone())
    }) {
        return Some(value);
    }
    if is_flatpak() {
        let output = Command::new("flatpak-spawn")
            .args(["--host", "printenv", key])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let value = String::from_utf8(output.stdout).ok()?;
        return trimmed_value(value);
    }
    std::env::var(key).ok().and_then(trimmed_value)
}

pub fn path_is_file(path: &str) -> bool {
    if is_flatpak() {
        return host_path(path).is_some_and(|path| path.is_file());
    }
    Path::new(path).is_file()
}

fn host_path(path: &str) -> Option<PathBuf> {
    let relative = path.strip_prefix('/')?;
    Some(PathBuf::from(HOST_ROOT).join(relative))
}

pub fn is_flatpak() -> bool {
    #[cfg(test)]
    if let Some(value) = FLATPAK_TEST_OVERRIDE.with(|override_value| override_value.get()) {
        return value;
    }
    std::env::var_os("FLATPAK_ID").is_some() || Path::new("/.flatpak-info").is_file()
}

#[cfg(test)]
pub(crate) fn set_flatpak_test_override(value: Option<bool>) {
    FLATPAK_TEST_OVERRIDE.with(|override_value| override_value.set(value));
}

#[cfg(test)]
pub(crate) fn set_environment_value_test_override(value: Option<(&str, &str)>) {
    ENVIRONMENT_VALUE_TEST_OVERRIDE.with(|override_value| {
        *override_value.borrow_mut() =
            value.map(|(key, value)| (key.to_string(), value.to_string()));
    });
}

fn trimmed_value(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn command_builder_argv(command: CommandBuilder) -> Vec<String> {
        command
            .get_argv()
            .iter()
            .map(|argument| argument.to_string_lossy().to_string())
            .collect()
    }

    #[test]
    #[serial]
    fn command_builder_targets_program_directly_outside_flatpak() {
        set_flatpak_test_override(Some(false));

        assert_eq!(
            command_builder_argv(command_builder("/bin/sh")),
            vec!["/bin/sh"]
        );

        set_flatpak_test_override(None);
    }

    #[test]
    #[serial]
    fn command_builder_uses_flatpak_spawn_inside_flatpak() {
        set_flatpak_test_override(Some(true));

        assert_eq!(
            command_builder_argv(command_builder("/bin/sh")),
            vec!["flatpak-spawn", "--host", "/bin/sh"]
        );

        set_flatpak_test_override(None);
    }

    #[test]
    #[serial]
    fn command_builder_exports_terminal_size_through_flatpak_spawn() {
        set_flatpak_test_override(Some(true));

        assert_eq!(
            command_builder_argv(command_builder_with_size(
                "/bin/sh",
                Some(ScreenSize::new(240, 50).expect("valid size"))
            )),
            vec![
                "flatpak-spawn",
                "--host",
                "--env=COLUMNS=240",
                "--env=LINES=50",
                "/bin/sh"
            ]
        );

        set_flatpak_test_override(None);
    }

    #[test]
    fn absolute_host_paths_are_mapped_under_flatpak_host_root() {
        assert_eq!(
            host_path("/usr/bin/fish").as_deref(),
            Some(Path::new("/var/run/host/usr/bin/fish"))
        );
        assert_eq!(host_path("fish"), None);
    }
}
