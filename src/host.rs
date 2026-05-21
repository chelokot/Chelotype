use portable_pty::CommandBuilder;
use std::path::{Path, PathBuf};
use std::process::Command;

const HOST_ROOT: &str = "/var/run/host";

pub fn command_builder(program: &str) -> CommandBuilder {
    if is_flatpak() {
        let mut command = CommandBuilder::new("flatpak-spawn");
        command.arg("--host");
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

fn is_flatpak() -> bool {
    std::env::var_os("FLATPAK_ID").is_some() || Path::new("/.flatpak-info").is_file()
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
        unsafe {
            std::env::remove_var("FLATPAK_ID");
        }

        assert_eq!(
            command_builder_argv(command_builder("/bin/sh")),
            vec!["/bin/sh"]
        );
    }

    #[test]
    #[serial]
    fn command_builder_uses_flatpak_spawn_inside_flatpak() {
        unsafe {
            std::env::set_var("FLATPAK_ID", "com.chelokot.Chelotype");
        }

        assert_eq!(
            command_builder_argv(command_builder("/bin/sh")),
            vec!["flatpak-spawn", "--host", "/bin/sh"]
        );

        unsafe {
            std::env::remove_var("FLATPAK_ID");
        }
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
