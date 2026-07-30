use crate::backend::ScreenSize;
use portable_pty::CommandBuilder;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LaunchTarget {
    Host,
    Distrobox {
        name: String,
        has_unshared_groups: bool,
    },
    Toolbox {
        name: String,
    },
    Podman {
        name: String,
    },
}

impl LaunchTarget {
    pub fn title(&self) -> String {
        match self {
            Self::Host => "My Computer".to_string(),
            Self::Distrobox { name, .. } | Self::Toolbox { name } | Self::Podman { name } => {
                name.clone()
            }
        }
    }

    pub fn id(&self) -> String {
        match self {
            Self::Host => "host".to_string(),
            Self::Distrobox { name, .. } => format!("distrobox:{name}"),
            Self::Toolbox { name } => format!("toolbox:{name}"),
            Self::Podman { name } => format!("podman:{name}"),
        }
    }

    pub fn command(&self) -> CommandBuilder {
        self.command_with_size(None)
    }

    pub fn command_with_size(&self, size: Option<ScreenSize>) -> CommandBuilder {
        match self {
            Self::Host => shell_command(size),
            Self::Distrobox {
                name,
                has_unshared_groups,
            } => distrobox_command(name, *has_unshared_groups, size),
            Self::Toolbox { name } => podman_exec_command(name, true, size),
            Self::Podman { name } => podman_exec_command(name, false, size),
        }
    }
}

pub fn startup_launch_target() -> LaunchTarget {
    let targets = available_launch_targets();
    select_startup_launch_target(&targets, crate::config::read_value("startup_launch_target"))
}

pub fn remember_startup_launch_target(target: &LaunchTarget) {
    crate::config::write_value("startup_launch_target", &target.id());
}

pub fn available_launch_targets() -> Vec<LaunchTarget> {
    let mut targets = vec![LaunchTarget::Host];
    let podman_targets = podman_containers();
    let podman_names = podman_targets
        .iter()
        .map(LaunchTarget::title)
        .collect::<HashSet<_>>();
    targets.extend(
        toolbox_containers()
            .into_iter()
            .filter(|name| !podman_names.contains(name.as_str()))
            .map(|name| LaunchTarget::Toolbox { name }),
    );
    targets.extend(podman_targets);
    targets
}

fn select_startup_launch_target(
    targets: &[LaunchTarget],
    remembered_id: Option<String>,
) -> LaunchTarget {
    if let Some(remembered) = remembered_id
        .as_deref()
        .and_then(|id| targets.iter().find(|target| target.id() == id))
    {
        return remembered.clone();
    }
    targets
        .iter()
        .find(|target| !matches!(target, LaunchTarget::Host))
        .cloned()
        .unwrap_or(LaunchTarget::Host)
}

fn shell_command(size: Option<ScreenSize>) -> CommandBuilder {
    crate::shell::default_shell_command_with_size(size)
}

fn distrobox_command(
    name: &str,
    has_unshared_groups: bool,
    size: Option<ScreenSize>,
) -> CommandBuilder {
    let mut command = crate::host::command_builder_with_size("distrobox", size);
    mark_input_cursor_bridge(&mut command);
    command.arg("enter");
    if !has_unshared_groups {
        command.arg("--no-tty");
    }
    command.arg(name);
    if !has_unshared_groups {
        command.arg("--additional-flags");
        command.arg("--tty");
    }
    command.arg("--");
    command.arg("env");
    if let Some(directory) = launch_working_directory() {
        command.arg(format!("--chdir={directory}"));
    }
    append_input_cursor_target_env(&mut command);
    command.args(crate::shell::default_shell_argv());
    command
}

fn podman_exec_command(
    name: &str,
    is_toolbox_like: bool,
    size: Option<ScreenSize>,
) -> CommandBuilder {
    let mut command = crate::host::command_builder_with_size("/bin/sh", size);
    mark_input_cursor_bridge(&mut command);
    command.arg("-lc");
    let mut exec = format!(
        "podman start {name} >/dev/null 2>&1 || true; exec podman exec --privileged --interactive --tty --detach-keys= ",
        name = shell_quote(name),
    );
    append_input_cursor_target_podman_env(&mut exec);
    if is_toolbox_like {
        if let Some(user) = crate::host::environment_value("USER") {
            exec.push_str(&format!("--user={} ", shell_quote(&user)));
        }
        if let Some(directory) = launch_working_directory() {
            exec.push_str(&format!("--workdir={} ", shell_quote(&directory)));
        }
    }
    exec.push_str(&shell_quote(name));
    exec.push(' ');
    exec.push_str(&crate::shell::default_shell_command_line());
    command.arg(exec);
    command
}

fn mark_input_cursor_bridge(command: &mut CommandBuilder) {
    if crate::shell::default_shell_has_input_edit_bridge() {
        command.env(
            crate::shell::INPUT_CURSOR_BRIDGE_ENV,
            crate::shell::INPUT_CURSOR_BRIDGE_FISH,
        );
    }
}

fn append_input_cursor_target_env(command: &mut CommandBuilder) {
    if crate::shell::default_shell_has_input_edit_bridge() {
        command.arg(format!(
            "{}={}",
            crate::shell::INPUT_CURSOR_TARGET_FILE_ENV,
            crate::shell::INPUT_CURSOR_TARGET_FILE_PLACEHOLDER
        ));
    }
}

fn append_input_cursor_target_podman_env(exec: &mut String) {
    if crate::shell::default_shell_has_input_edit_bridge() {
        exec.push_str(&format!(
            "--env={}={} ",
            crate::shell::INPUT_CURSOR_TARGET_FILE_ENV,
            crate::shell::INPUT_CURSOR_TARGET_FILE_PLACEHOLDER
        ));
    }
}

fn launch_working_directory() -> Option<String> {
    crate::host::environment_value("PWD")
        .filter(|path| !path.trim().is_empty())
        .or_else(|| crate::host::environment_value("HOME"))
}

fn toolbox_containers() -> Vec<String> {
    let Ok(output) = crate::host::command("toolbox")
        .args(["list", "--containers"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_toolbox_list(&String::from_utf8_lossy(&output.stdout))
}

fn podman_containers() -> Vec<LaunchTarget> {
    let Ok(output) = crate::host::command("podman")
        .args(["ps", "-a", "--format=json"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_podman_targets(&String::from_utf8_lossy(&output.stdout))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn parse_toolbox_list(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty()
                || trimmed.starts_with("CONTAINER ID")
                || trimmed.starts_with("ID ")
            {
                return None;
            }
            let columns = trimmed.split_whitespace().collect::<Vec<_>>();
            columns.get(1).copied().or_else(|| columns.first().copied())
        })
        .map(ToOwned::to_owned)
        .collect()
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PodmanContainerJson {
    is_infra: Option<bool>,
    labels: Option<HashMap<String, String>>,
    names: Vec<String>,
}

fn parse_podman_targets(output: &str) -> Vec<LaunchTarget> {
    let Ok(containers) = serde_json::from_str::<Vec<PodmanContainerJson>>(output) else {
        return Vec::new();
    };
    containers
        .into_iter()
        .filter(|container| container.is_infra != Some(true))
        .filter_map(|container| {
            let name = container.names.into_iter().next()?;
            let labels = container.labels.unwrap_or_default();
            if labels
                .get("manager")
                .is_some_and(|value| value == "distrobox")
                || labels.get("manager").is_some_and(|value| value == "apx")
            {
                return Some(LaunchTarget::Distrobox {
                    name,
                    has_unshared_groups: labels
                        .get("distrobox.unshare_groups")
                        .is_some_and(|value| value == "1"),
                });
            }
            if labels.contains_key("com.github.containers.toolbox")
                || labels.contains_key("org.containers.toolbox")
            {
                return Some(LaunchTarget::Toolbox { name });
            }
            Some(LaunchTarget::Podman { name })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_toolbox_table_names() {
        let output = "\
CONTAINER ID  CONTAINER NAME              CREATED       STATUS   IMAGE NAME
ca296c735b0d  fedora-toolbox-latest       5 months ago  exited   image
e861f5c4e141  fedora-toolbox-sha-b719027  7 months ago  running  image
";
        assert_eq!(
            parse_toolbox_list(output),
            vec!["fedora-toolbox-latest", "fedora-toolbox-sha-b719027"]
        );
    }

    #[test]
    fn shell_quotes_container_names() {
        assert_eq!(shell_quote("fedora-toolbox"), "'fedora-toolbox'");
        assert_eq!(shell_quote("bad'name"), "'bad'\\''name'");
    }

    #[test]
    fn parses_podman_json_with_ptyxis_label_priority() {
        let output = r#"[
          {
            "IsInfra": false,
            "Names": ["fedora-toolbox"],
            "Labels": {
              "manager": "distrobox",
              "com.github.containers.toolbox": "true",
              "distrobox.unshare_groups": "0"
            }
          },
          {
            "IsInfra": false,
            "Names": ["fedora-toolbox-raw"],
            "Labels": {
              "com.github.containers.toolbox": "true"
            }
          },
          {
            "IsInfra": false,
            "Names": ["postgres"],
            "Labels": {}
          },
          {
            "IsInfra": true,
            "Names": ["infra"],
            "Labels": {}
          }
        ]"#;

        assert_eq!(
            parse_podman_targets(output),
            vec![
                LaunchTarget::Distrobox {
                    name: "fedora-toolbox".to_string(),
                    has_unshared_groups: false,
                },
                LaunchTarget::Toolbox {
                    name: "fedora-toolbox-raw".to_string(),
                },
                LaunchTarget::Podman {
                    name: "postgres".to_string(),
                },
            ]
        );
    }

    #[test]
    #[serial_test::serial]
    fn distrobox_launch_uses_distrobox_enter_without_root_podman_exec() {
        let old_pwd = std::env::var_os("PWD");
        unsafe {
            std::env::set_var("CHELOTYPE_SHELL", "/bin/sh");
            std::env::set_var("PWD", "/var/home/chelokot");
        }

        let command = LaunchTarget::Distrobox {
            name: "fedora-toolbox".to_string(),
            has_unshared_groups: false,
        }
        .command();
        let argv = command
            .get_argv()
            .iter()
            .map(|argument| argument.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert_eq!(argv[0], "distrobox");
        assert_eq!(argv[1], "enter");
        assert!(argv.contains(&"--no-tty".to_string()));
        assert!(argv.contains(&"fedora-toolbox".to_string()));
        assert!(argv.contains(&"--additional-flags".to_string()));
        assert!(argv.contains(&"--tty".to_string()));
        assert!(argv.contains(&"--".to_string()));
        assert!(argv.contains(&"env".to_string()));
        assert!(argv.contains(&"--chdir=/var/home/chelokot".to_string()));
        assert_eq!(argv.last().map(String::as_str), Some("/bin/sh"));

        unsafe {
            std::env::remove_var("CHELOTYPE_SHELL");
            if let Some(old_pwd) = old_pwd {
                std::env::set_var("PWD", old_pwd);
            } else {
                std::env::remove_var("PWD");
            }
        }
    }

    #[test]
    #[serial_test::serial]
    fn distrobox_fish_launch_exports_cursor_target_placeholder() {
        let old_shell = std::env::var_os("CHELOTYPE_SHELL");
        unsafe {
            std::env::set_var("CHELOTYPE_SHELL", "/usr/bin/fish");
        }

        let command = LaunchTarget::Distrobox {
            name: "fedora-toolbox".to_string(),
            has_unshared_groups: false,
        }
        .command();
        let argv = command
            .get_argv()
            .iter()
            .map(|argument| argument.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert_eq!(
            command.get_env(crate::shell::INPUT_CURSOR_BRIDGE_ENV),
            Some(std::ffi::OsStr::new(crate::shell::INPUT_CURSOR_BRIDGE_FISH))
        );
        assert!(argv.contains(&format!(
            "{}={}",
            crate::shell::INPUT_CURSOR_TARGET_FILE_ENV,
            crate::shell::INPUT_CURSOR_TARGET_FILE_PLACEHOLDER
        )));

        unsafe {
            if let Some(old_shell) = old_shell {
                std::env::set_var("CHELOTYPE_SHELL", old_shell);
            } else {
                std::env::remove_var("CHELOTYPE_SHELL");
            }
        }
    }

    #[test]
    fn toolbox_launch_runs_as_user_in_working_directory() {
        let command = LaunchTarget::Toolbox {
            name: "fedora-toolbox-latest".to_string(),
        }
        .command();
        let argv = command
            .get_argv()
            .iter()
            .map(|argument| argument.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert_eq!(argv[0], "/bin/sh");
        assert_eq!(argv[1], "-lc");
        assert!(argv[2].contains("podman start 'fedora-toolbox-latest'"));
        assert!(argv[2].contains("podman exec --privileged --interactive --tty"));
        if crate::shell::default_shell_has_input_edit_bridge() {
            assert!(argv[2].contains(&format!(
                "--env={}={}",
                crate::shell::INPUT_CURSOR_TARGET_FILE_ENV,
                crate::shell::INPUT_CURSOR_TARGET_FILE_PLACEHOLDER
            )));
        }
        assert!(argv[2].contains("--user="));
        assert!(argv[2].contains("--workdir="));
    }

    #[test]
    fn podman_launch_runs_configured_shell_inside_container() {
        let command = LaunchTarget::Podman {
            name: "fedora-toolbox".to_string(),
        }
        .command();
        let argv = command
            .get_argv()
            .iter()
            .map(|argument| argument.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert_eq!(argv[0], "/bin/sh");
        assert_eq!(argv[1], "-lc");
        assert!(argv[2].contains("podman exec --privileged --interactive --tty"));
        if crate::shell::default_shell_has_input_edit_bridge() {
            assert!(argv[2].contains(&format!(
                "--env={}={}",
                crate::shell::INPUT_CURSOR_TARGET_FILE_ENV,
                crate::shell::INPUT_CURSOR_TARGET_FILE_PLACEHOLDER
            )));
        }
        assert!(!argv[2].contains("--user="));
        assert!(!argv[2].contains("--workdir="));
        if argv[2].contains("fish") {
            assert!(argv[2].contains("'--init-command'"));
        }
    }

    #[test]
    #[serial_test::serial]
    fn toolbox_launch_runs_through_host_when_flatpaked() {
        crate::host::set_flatpak_test_override(Some(true));
        crate::host::set_environment_value_test_override(Some(("USER", "chelotype-test")));
        unsafe {
            std::env::set_var("CHELOTYPE_SHELL", "/bin/sh");
        }

        let command = LaunchTarget::Toolbox {
            name: "fedora-toolbox-latest".to_string(),
        }
        .command();
        let argv = command
            .get_argv()
            .iter()
            .map(|argument| argument.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert_eq!(argv[0], "flatpak-spawn");
        assert_eq!(argv[1], "--host");
        assert_eq!(argv[2], "/bin/sh");
        assert_eq!(argv[3], "-lc");
        assert!(argv[4].contains("podman start 'fedora-toolbox-latest'"));
        assert!(argv[4].contains("--user='chelotype-test'"));

        unsafe {
            std::env::remove_var("CHELOTYPE_SHELL");
        }
        crate::host::set_environment_value_test_override(None);
        crate::host::set_flatpak_test_override(None);
    }

    #[test]
    #[serial_test::serial]
    fn podman_launch_runs_through_host_shell_when_flatpaked() {
        crate::host::set_flatpak_test_override(Some(true));
        unsafe {
            std::env::set_var("CHELOTYPE_SHELL", "/bin/sh");
        }

        let command = LaunchTarget::Podman {
            name: "fedora-toolbox".to_string(),
        }
        .command();
        let argv = command
            .get_argv()
            .iter()
            .map(|argument| argument.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        assert_eq!(argv[0], "flatpak-spawn");
        assert_eq!(argv[1], "--host");
        assert_eq!(argv[2], "/bin/sh");
        assert_eq!(argv[3], "-lc");
        assert!(argv[4].contains("podman exec --privileged --interactive --tty"));

        unsafe {
            std::env::remove_var("CHELOTYPE_SHELL");
        }
        crate::host::set_flatpak_test_override(None);
    }

    #[test]
    fn launch_target_ids_are_stable() {
        assert_eq!(LaunchTarget::Host.id(), "host");
        assert_eq!(
            LaunchTarget::Distrobox {
                name: "fedora-toolbox".to_string(),
                has_unshared_groups: false
            }
            .id(),
            "distrobox:fedora-toolbox"
        );
        assert_eq!(
            LaunchTarget::Toolbox {
                name: "fedora-toolbox-latest".to_string()
            }
            .id(),
            "toolbox:fedora-toolbox-latest"
        );
        assert_eq!(
            LaunchTarget::Podman {
                name: "postgres".to_string()
            }
            .id(),
            "podman:postgres"
        );
    }

    #[test]
    fn startup_target_prefers_existing_remembered_container() {
        let targets = vec![
            LaunchTarget::Host,
            LaunchTarget::Toolbox {
                name: "fedora-toolbox-latest".to_string(),
            },
            LaunchTarget::Podman {
                name: "postgres".to_string(),
            },
        ];

        assert_eq!(
            select_startup_launch_target(&targets, Some("podman:postgres".to_string())),
            LaunchTarget::Podman {
                name: "postgres".to_string()
            }
        );
    }

    #[test]
    fn startup_target_falls_back_to_first_container_before_host() {
        let targets = vec![
            LaunchTarget::Host,
            LaunchTarget::Toolbox {
                name: "fedora-toolbox-latest".to_string(),
            },
        ];

        assert_eq!(
            select_startup_launch_target(&targets, None),
            LaunchTarget::Toolbox {
                name: "fedora-toolbox-latest".to_string()
            }
        );
        assert_eq!(
            select_startup_launch_target(&targets, Some("podman:missing".to_string())),
            LaunchTarget::Toolbox {
                name: "fedora-toolbox-latest".to_string()
            }
        );
    }

    #[test]
    fn startup_target_uses_host_when_no_container_exists() {
        assert_eq!(
            select_startup_launch_target(&[LaunchTarget::Host], None),
            LaunchTarget::Host
        );
    }
}
