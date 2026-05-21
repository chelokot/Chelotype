# Chelotype

<!-- chelotype-media-start -->
<p align="center">
  <video src="docs/media/chelotype-preferences.webm" autoplay loop muted playsinline controls width="960"></video>
</p>
<!-- chelotype-media-end -->

Chelotype is a GTK4/libadwaita terminal experiment backed by one real PTY and `libghostty-vt` as the terminal-core state. The app renders from Ghostty grid/cell snapshots through our own renderer; the old VTE bridge and hidden-terminal input path are gone.

## Run

```sh
bash scripts/with-zig.sh cargo run
```

The first run downloads Zig 0.15.2 into `.tools/` because `libghostty-vt` builds its vendored native core with Zig. `.tools/` is ignored by git.
Chelotype opens `fish` by default when it is installed. Use `CHELOTYPE_SHELL=/path/to/shell` to force another shell for a run.
When no previous container is remembered, Chelotype prefers the first available toolbox/podman container over the host. Closing a single-container window stores that container as the next startup target.

## Check

```sh
bash scripts/with-zig.sh cargo fmt -- --check
bash scripts/with-zig.sh cargo clippy --all-targets -- -D warnings
bash scripts/with-zig.sh cargo test -- --test-threads=1
bash scripts/with-zig.sh cargo bench --bench pipeline -- --sample-size 10
```

## Packaging

Desktop metadata lives under `data/` and can be installed with:

```sh
PREFIX=/usr DESTDIR=/tmp/chelotype-root scripts/install-desktop-metadata.sh
```

The Flatpak manifest is `packaging/flatpak/com.chelokot.Chelotype.yml`. It
follows the same terminal-oriented sandbox shape as Ptyxis: the UI runs inside
the Flatpak, while host shells and container commands are launched through
`flatpak-spawn --host`.

```sh
flatpak-builder --user --disable-rofiles-fuse --install-deps-from=flathub --force-clean build-dir packaging/flatpak/com.chelokot.Chelotype.yml
```

Fedora packaging notes and a draft spec template are in `packaging/fedora/`.
Chelotype is licensed as `MIT OR Apache-2.0`; the license files are installed
for Flatpak builds and referenced by Fedora packaging metadata.

## 240 Hz Runtime Profile

```sh
scripts/profile-240hz.sh --scenario held-key --duration 10
scripts/profile-240hz.sh --scenario scroll --duration 6
scripts/profile-240hz.sh --display-backend weston-headless --scenario frame-baseline --duration 1 --release
scripts/profile-240hz.sh --display-backend weston-headless --scenario timer-baseline --duration 1 --release
```

Use `--strict` to fail when `gtk_frame_interval`, real wall-clock tick cadence, smooth-scroll frame cadence, or `gtk_paint` p50 exceeds the 4.166 ms 240 Hz budget on the current compositor. Use `frame-baseline` to separate compositor/GDK pacing from terminal render work, and `timer-baseline` to compare GLib timeout cadence against GDK frame callbacks.

## Headless Diagnostics

```sh
CHELOTYPE_HEADLESS=1 \
CHELOTYPE_HEADLESS_EVENTS='text:printf \x27hello\n\x27\n' \
CHELOTYPE_HEADLESS_EXPECT='hello' \
bash scripts/with-zig.sh cargo run
```

Snapshots are written to `/tmp/chelotype_snapshots` by default. Set `CHELOTYPE_SNAPSHOT_DIR` to override it.
Headless diagnostics use a clean interactive `zsh -f` fixture with an empty prompt so editable shell input and mouse-click cursor movement are deterministic.

## GTK E2E

Real window smoke tests run under Xvfb:

```sh
GSETTINGS_BACKEND=memory NO_AT_BRIDGE=1 \
CHELOTYPE_UI_E2E=1 \
CHELOTYPE_UI_E2E_INPUT="printf 'GTK_E2E_OK\n'\n" \
CHELOTYPE_UI_E2E_EXPECT='GTK_E2E_OK' \
xvfb-run -a target/debug/chelotype
```

Build first with `bash scripts/with-zig.sh cargo build`, or just run the covered test:

```sh
bash scripts/with-zig.sh cargo test --test gtk_e2e_tests
```

The same test file also runs scenarios that verify ANSI-colored cells, capture a nonblank real-window screenshot, check truecolor terminal output, narrow cursor shape, cursor blink/reset, cursor animation/style/shape settings, and command-block rails at the pixel level, send real keyboard events, render IM preedit state before commit, commit IM-composed text with `xdotool`, click the current input row to move the shell cursor, forward terminal mouse-reporting bytes to the PTY, drag-select terminal text with the mouse, keep released drag selections stable for copy, export PRIMARY/CLIPBOARD selection text, resize split panes by dragging the divider, resize the GTK window, and inspect the resulting structured snapshots.

## Flathub Media

```sh
scripts/capture-flathub-media.sh
```
