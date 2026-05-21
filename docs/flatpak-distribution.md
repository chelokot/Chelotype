# Flatpak Distribution

Chelotype publishes a signed Flatpak repository to a static website.

Add the repository once and install the app from it:

```sh
flatpak remote-add --user --if-not-exists chelotype https://chelokot.com/flatpak/chelotype.flatpakrepo
flatpak install --user chelotype com.chelokot.Chelotype
```

Updates use the same Flatpak remote:

```sh
flatpak update --user com.chelokot.Chelotype
```

The publishing workflow is `.github/workflows/publish-flatpak.yml`. It builds
`com.chelokot.Chelotype.yml` on the `stable` Flatpak branch, signs the
repository with the dedicated Flatpak GPG key stored in GitHub Actions secrets,
writes the install descriptors, and deploys the `public/` directory to GitHub
Pages.
