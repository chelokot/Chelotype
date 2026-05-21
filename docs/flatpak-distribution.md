# Flatpak Distribution

Chelotype publishes a signed Flatpak repository to a static website.

Install directly from the Flatpak ref:

```sh
flatpak install --user https://chelokot.com/flatpak/com.chelokot.Chelotype.flatpakref
```

The `.flatpakref` file points Flatpak at Flathub for the GNOME runtime and at
the Chelotype repository for the app itself.

To add the Chelotype remote explicitly:

```sh
flatpak remote-add --user --if-not-exists chelotype https://chelokot.com/flatpak/chelotype.flatpakrepo
flatpak install --user chelotype com.chelokot.Chelotype
```

The publishing workflow is `.github/workflows/publish-flatpak.yml`. It builds
`com.chelokot.Chelotype.yml` on the `stable` Flatpak branch, signs the
repository with the dedicated Flatpak GPG key stored in GitHub Actions secrets,
writes the install descriptors, and deploys the `public/` directory to GitHub
Pages.
