# Fedora Packaging Notes

Chelotype uses the Rust ecosystem's common permissive license expression:
`MIT OR Apache-2.0`.

The distro package should install:

- `/usr/bin/chelotype`
- `LICENSE-APACHE`
- `LICENSE-MIT`
- `data/com.chelokot.Chelotype.desktop`
- `data/com.chelokot.Chelotype.metainfo.xml`
- `data/icons/hicolor/scalable/apps/com.chelokot.Chelotype.svg`
- `data/icons/hicolor/symbolic/apps/com.chelokot.Chelotype-symbolic.svg`

The native build needs GTK4, libadwaita, Rust, and Zig 0.15.x because
`libghostty-vt` builds its vendored native core with Zig.

Binary packages should include license files for the bundled Rust crates,
Ghostty source, and Zig package sources that are linked into `chelotype` or
`libghostty-vt`.

Suggested package checks:

```sh
cargo test --locked -- --test-threads=1
desktop-file-validate data/com.chelokot.Chelotype.desktop
appstreamcli validate --no-net data/com.chelokot.Chelotype.metainfo.xml
```
