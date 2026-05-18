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
