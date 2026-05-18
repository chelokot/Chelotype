# Chelotype

Chelotype is a GTK4/libadwaita terminal experiment backed by one real PTY and `libghostty-vt` as the terminal-core state. The app renders from Ghostty grid/cell snapshots through our own renderer; the old VTE bridge and hidden-terminal input path are gone.

## Run

```sh
bash scripts/with-zig.sh cargo run
```

The first run downloads Zig 0.15.2 into `.tools/` because `libghostty-vt` builds its vendored native core with Zig. `.tools/` is ignored by git.

## Check

```sh
bash scripts/with-zig.sh cargo fmt -- --check
bash scripts/with-zig.sh cargo clippy --all-targets -- -D warnings
bash scripts/with-zig.sh cargo test
bash scripts/with-zig.sh cargo bench --bench pipeline -- --sample-size 10
```

## Headless Diagnostics

```sh
CHELOTYPE_HEADLESS=1 \
CHELOTYPE_HEADLESS_EVENTS='text:printf \x27hello\n\x27\n' \
CHELOTYPE_HEADLESS_EXPECT='hello' \
bash scripts/with-zig.sh cargo run
```

Snapshots are written to `/tmp/chelotype_snapshots` by default. Set `CHELOTYPE_SNAPSHOT_DIR` to override it.

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

The same test file also runs scenarios that verify ANSI-colored cells, send real keyboard events with `xdotool`, drag-select terminal text with the mouse, resize the GTK window, and inspect the resulting structured snapshots.
