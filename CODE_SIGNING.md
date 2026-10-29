# Code signing policy

The Windows installer is not code-signed, so Windows may show "Unknown publisher" when you run it. The Linux packages are not signed by a distribution key either.

macOS apps and disk images are signed with an Apple Developer ID and notarized by Apple. Every download that the in-app updater installs carries its own signature, which the app checks before installing. Each release also lists every download in `SHA256SUMS`, so you can check a file by hand.

## Build and approval

The Windows installer and the Linux packages are built from this repository's source code by GitHub Actions on GitHub-hosted runners. The macOS app is built from the same tagged commit on the maintainer's Mac, where it is signed and notarized. Every release is approved by hand before it is published.

## Privacy

See the [Privacy](README.md#privacy) section of the README. The app never sends your usage data anywhere. It connects only to check for updates, which you can turn off under **Settings → Updates**.
