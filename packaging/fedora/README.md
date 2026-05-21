# Fedora Packaging Notes

Chelotype uses the Rust ecosystem's common permissive license expression:
`MIT OR Apache-2.0`.

The distro package should install:

- `/usr/bin/chelotype`
- `data/com.chelotype.Terminal.desktop`
- `data/com.chelotype.Terminal.metainfo.xml`
- `data/icons/hicolor/scalable/apps/com.chelotype.Terminal.svg`
- `data/icons/hicolor/symbolic/apps/com.chelotype.Terminal-symbolic.svg`

The native build needs GTK4, libadwaita, Rust, and Zig 0.15.x because
`libghostty-vt` builds its vendored native core with Zig.

Binary packages should include license files for the bundled Rust crates,
Ghostty source, and Zig package sources that are linked into `chelotype` or
`libghostty-vt`.

Suggested package checks:

```sh
cargo test --locked -- --test-threads=1
desktop-file-validate data/com.chelotype.Terminal.desktop
appstreamcli validate --no-net data/com.chelotype.Terminal.metainfo.xml
```
