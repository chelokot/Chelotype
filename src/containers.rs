use portable_pty::CommandBuilder;
use std::process::Command;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LaunchTarget {
    Host,
    Toolbox { name: String },
    Podman { name: String },
}

impl LaunchTarget {
    pub fn title(&self) -> String {
        match self {
            Self::Host => "My Computer".to_string(),
            Self::Toolbox { name } | Self::Podman { name } => name.clone(),
        }
    }

    pub fn id(&self) -> String {
        match self {
            Self::Host => "host".to_string(),
            Self::Toolbox { name } => format!("toolbox:{name}"),
            Self::Podman { name } => format!("podman:{name}"),
        }
    }

    pub fn command(&self) -> CommandBuilder {
        match self {
            Self::Host => shell_command(),
            Self::Toolbox { name } => {
                let mut command = CommandBuilder::new("toolbox");
                command.arg("enter");
                command.arg("--container");
                command.arg(name);
                command
            }
            Self::Podman { name } => {
                let mut command = CommandBuilder::new("/bin/sh");
                command.arg("-lc");
                command.arg(format!(
                    "podman start {name} >/dev/null 2>&1 || true; exec podman exec -it {name} {shell}",
                    name = shell_quote(name),
                    shell = shell_quote(&crate::shell::default_shell_path())
                ));
                command
            }
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
    targets.extend(
        toolbox_containers()
            .into_iter()
            .map(|name| LaunchTarget::Toolbox { name }),
    );
    let toolbox_names = targets
        .iter()
        .filter_map(|target| match target {
            LaunchTarget::Toolbox { name } => Some(name.clone()),
            _ => None,
        })
        .collect::<std::collections::HashSet<_>>();
    targets.extend(
        podman_running_containers()
            .into_iter()
            .filter(|name| !toolbox_names.contains(name.as_str()))
            .map(|name| LaunchTarget::Podman { name }),
    );
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

fn shell_command() -> CommandBuilder {
    crate::shell::default_shell_command()
}

fn toolbox_containers() -> Vec<String> {
    let Ok(output) = Command::new("toolbox")
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

fn podman_running_containers() -> Vec<String> {
    let Ok(output) = Command::new("podman")
        .args(["ps", "-a", "--format", "{{.Names}}"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_names(&String::from_utf8_lossy(&output.stdout))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn parse_names(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
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
    fn parses_plain_podman_names() {
        assert_eq!(
            parse_names("fedora-toolbox-sha-b719027\norn-clickhouse-local\n"),
            vec!["fedora-toolbox-sha-b719027", "orn-clickhouse-local"]
        );
    }

    #[test]
    fn shell_quotes_container_names() {
        assert_eq!(shell_quote("fedora-toolbox"), "'fedora-toolbox'");
        assert_eq!(shell_quote("bad'name"), "'bad'\\''name'");
    }

    #[test]
    fn launch_target_ids_are_stable() {
        assert_eq!(LaunchTarget::Host.id(), "host");
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
