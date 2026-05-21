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

## Domain

GitHub Pages is configured with `chelokot.com` as the custom domain. The apex
DNS records for `chelokot.com` must point at GitHub Pages:

```txt
185.199.108.153
185.199.109.153
185.199.110.153
185.199.111.153
```

Optional IPv6 records:

```txt
2606:50c0:8000::153
2606:50c0:8001::153
2606:50c0:8002::153
2606:50c0:8003::153
```

After DNS propagation, verify the public repo descriptor:

```sh
curl -I https://chelokot.com/flatpak/chelotype.flatpakrepo
```
