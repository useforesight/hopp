# Releasing Hopp (Foresight fork)

Releases are published at <https://github.com/useforesight/hopp/releases>. Every installed copy checks the latest published release and updates itself, so publishing reaches everyone.

## Signing

Hopp isn't notarized, as this repo has no Apple Developer account, and macOS blocks apps downloaded in a browser unless they are. There are two ways past that:

- **`install.sh` (recommended):** files fetched with curl aren't quarantined, so macOS doesn't block the app. The script checks the signature itself.
- **DMG:** macOS blocks it on first open. Go to System Settings > Privacy & Security and choose Open Anyway.

Updates aren't quarantined either, so they install without a prompt.

Everything is signed with a self-signed "Foresight Hopp" certificate. It keeps the signature stable, so macOS permissions carry over between updates. If you replace the certificate, update the hash in `install.sh`, and everyone has to grant permissions again.

## Secrets

These are in the `Release` environment (Settings > Environments):

| Secret                               | Value                                                                     |
| ------------------------------------ | ------------------------------------------------------------------------- |
| `TAURI_SIGNING_PRIVATE_KEY`          | Private half of the updater key whose public half is in `tauri.conf.json` |
| `MACOS_SIGNING_CERTIFICATE`          | Base64 of the signing certificate's `.p12`                                |
| `MACOS_SIGNING_CERTIFICATE_PASSWORD` | Password of the `.p12`                                                    |
| `KEYCHAIN_PASSWORD`                  | Any random string. It only protects a keychain that lasts for one build   |

The Sentry and PostHog secrets in `release.yml` are optional: leave them unset and telemetry stays off.

**Keep a copy of the updater key and the certificate somewhere safe.** GitHub can't show them again. Without the updater key, installed apps can't receive updates and everyone has to reinstall by hand.

## Cutting a release

1. Set `version` in `tauri/src-tauri/tauri.conf.json` and merge to `main`.
2. Write the notes in `release-notes/<version>.md`.
3. Run the **Build Hopp Release** workflow from `main`, with `version` set to the new tag, such as `v1.1.0`. It builds macOS (Apple Silicon and Intel) and Windows, and creates a **draft** release.
   - The fork had no tags before its first release, so `v1.1.0` needs `from_ref` set to `4163a3e0bbe182f25ff19c901c03605d16019731` (upstream v1.0.33). Later releases can leave it empty.
4. Replace the generated notes: `gh release edit <version> --repo useforesight/hopp --notes-file release-notes/<version>.md`
5. Download the draft's build with `gh release download <version> --repo useforesight/hopp --pattern '*.dmg'`, install it on a Mac and make a real call: ring, answer, share a screen, draw.
6. Publish the draft. This creates the tag and makes `latest.json` live, and installed apps start updating themselves. A bad release can only be replaced by a newer one.

## Moving from stock Hopp

Stock Hopp updates from upstream, so it never receives these releases. Everyone installs once by hand:

```
curl -fsSL https://raw.githubusercontent.com/useforesight/hopp/main/install.sh | bash
```

Sign-in and settings carry over. macOS asks for each permission again, because the signature is different.

Stock Hopp and this build can call each other. Someone on stock sees drawings in the older style.
