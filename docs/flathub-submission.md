# Flathub Submission

The Flathub manifest is `com.chelokot.Chelotype.yml` in the repository root.
It builds the tagged `v0.1.0` source release and installs the desktop,
AppStream, icon, font, and bundled third-party license metadata.

Before opening the Flathub pull request, verify the tagged release locally:

```sh
flatpak-builder --user --disable-rofiles-fuse --install-deps-from=flathub --force-clean --repo=repo build-dir com.chelokot.Chelotype.yml
flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest com.chelokot.Chelotype.yml
flatpak run --command=flatpak-builder-lint org.flatpak.Builder repo repo
```

The app intentionally requests `org.freedesktop.Flatpak` access because its
Flatpak build launches host shells and host container tools through
`flatpak-spawn --host`. This needs a Flathub linter exception with a reason
matching that behavior.

The `com.chelokot.Chelotype` app ID also requires `https://chelokot.com` to be
reachable with a certificate valid for `chelokot.com`.

New app submissions must be opened against `flathub/flathub` branch `new-pr`
by a human maintainer. Copy `com.chelokot.Chelotype.yml`,
`cargo-sources.json`, and `zig-sources.json` into the submission branch.
