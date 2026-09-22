# Chelotype

<!-- chelotype-media-start -->
<p align="center">
  <a href="docs/media/chelotype-preferences.webm">
    <img src="docs/media/chelotype-preferences.webp" alt="Chelotype preferences preview" width="960">
  </a>
</p>
<!-- chelotype-media-end -->

Chelotype is an opinionated GTK terminal for Linux desktops. It is built in Rust, uses `libghostty-vt` as its terminal core, and focuses on container-first shell work, mouse-driven interaction, smooth scrolling, cursor animation, and practical customization.

It supports direct launching into containers, customization, and more.

## Workspace controls

| Action | Shortcut or gesture |
| --- | --- |
| New tab | `Ctrl+Shift+T` |
| Split the active tab | `Ctrl+Shift+E` |
| Switch tabs | `Ctrl+PageUp` / `Ctrl+PageDown` |
| Rename a tab | Double-click its title, or use its context menu |
| Resize split panes | Drag the divider |
| Preferences | `Ctrl+,` |

## Terminal context

The header changes color when the active pane's foreground PTY process or same-session ancestry is running with effective UID 0 or is an SSH client. The detector reads process identity from the Linux process namespace and never infers context from prompt or command text; changing tabs, panes, or processes refreshes the color. Root describes the local process. An SSH connection does not expose the remote account's privilege level, so remote root access is not inferred. Flatpak and other container process namespaces can restrict `/proc` visibility; when the context cannot be observed, the header keeps its normal color.

## Highlights

- Launch tabs directly into the host, Toolbox, Distrobox, or Podman containers.
- Keep Flatpak installs useful with host/container commands launched through `flatpak-spawn --host`.
- Move around terminal input with the mouse, drag selections, copy primary/clipboard text, and resize split panes.
- Tune cursor shape, cursor animation, animation speed, font, palette, and smooth scrolling from the GTK preferences UI.
- Render terminal state from Ghostty grid snapshots with Chelotype's own GTK4/libadwaita renderer.

## Status

Chelotype is early, Linux-only, and actively changing. The app is usable for development workflows, but package repository and Flathub submission work is still in progress.

## Run From Source

```sh
bash scripts/with-zig.sh cargo run
```

The first build downloads Zig into `.tools/` because the vendored Ghostty terminal core is built with Zig.

Chelotype opens `fish` when it is available. To force a different shell:

```sh
CHELOTYPE_SHELL=/path/to/shell bash scripts/with-zig.sh cargo run
```

## Checks

```sh
bash scripts/with-zig.sh cargo fmt -- --check
bash scripts/with-zig.sh cargo clippy --all-targets -- -D warnings
bash scripts/with-zig.sh cargo test -- --test-threads=1
```

The GTK tests need Xvfb, xauth, xdotool, ImageMagick, Python 3, Fish, Zsh,
zsh-autosuggestions, and zsh-syntax-highlighting. Application compatibility
tests also exercise Neovim and tmux through real PTYs. Set
`CHELOTYPE_REQUIRE_E2E=1` to fail when optional E2E dependencies are missing;
CI uses this mode. Tests launch their own X11 displays.
The GUI harness replaces container commands with test doubles, so it never
starts or inspects a developer's running Toolbox.

## Packaging

Desktop/AppStream metadata lives in `data/`.

```sh
PREFIX=/usr DESTDIR=/tmp/chelotype-root scripts/install-desktop-metadata.sh
```

```sh
flatpak-builder --user --disable-rofiles-fuse --install-deps-from=flathub --force-clean --default-branch=stable build-dir com.chelokot.Chelotype.yml
```

Fedora packaging notes and the draft spec live in `packaging/fedora/`.

## Install

```sh
flatpak remote-add --user --if-not-exists chelotype https://chelokot.com/flatpak/chelotype.flatpakrepo
flatpak install --user chelotype com.chelokot.Chelotype
```

After that, Chelotype updates through Flatpak:

```sh
flatpak update --user com.chelokot.Chelotype
```

## Media

Regenerate the README video and Flathub screenshot with:

```sh
scripts/capture-flathub-media.sh
```

## License

Chelotype is licensed under `MIT OR Apache-2.0`.
