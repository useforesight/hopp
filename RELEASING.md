# Releasing Hopp (Foresight fork)

Releases are published at <https://github.com/useforesight/hopp/releases>. Every installed copy checks the latest published release and updates itself, so publishing reaches everyone.

## One-time setup

1. In the repo's Actions tab, if GitHub says workflows aren't being run on this fork, enable them.
2. Add these secrets to the `Release` environment (Settings > Environments):

| Secret                       | Value                                                                     |
| ---------------------------- | ------------------------------------------------------------------------- |
| `TAURI_SIGNING_PRIVATE_KEY`  | Private half of the updater key whose public half is in `tauri.conf.json` |
| `KEYCHAIN_PASSWORD`          | Any random string. It only protects a keychain that lasts for one build   |
| `APPLE_CERTIFICATE`          | Base64 of the "Developer ID Application" certificate, exported as `.p12`  |
| `APPLE_CERTIFICATE_PASSWORD` | Password chosen when exporting the `.p12`                                 |
| `APPLE_ID`                   | Apple ID of a member of the Apple Developer team                          |
| `APPLE_PASSWORD`             | App-specific password for that Apple ID, from <https://account.apple.com> |
| `APPLE_TEAM_ID`              | Team ID of the Apple Developer team                                       |

Only the team's Account Holder can create a Developer ID certificate. The Sentry and PostHog secrets in `release.yml` are optional: leave them unset and telemetry stays off.

**Keep a copy of the updater private key somewhere safe.** GitHub can't show it again. Without it, installed apps can't receive updates, and everyone would have to reinstall by hand.

## Cutting a release

1. Set `version` in `tauri/src-tauri/tauri.conf.json` and merge to `main`.
2. Write the notes in `release-notes/<version>.md`.
3. Run the **Build Hopp Release** workflow from `main`, with `version` set to the new tag, such as `v1.1.0`. It builds macOS (Apple Silicon and Intel) and Windows, and creates a **draft** release.
   - The fork had no tags before its first release, so `v1.1.0` needs `from_ref` set to `4163a3e0bbe182f25ff19c901c03605d16019731` (upstream v1.0.33). Later releases can leave it empty.
4. Replace the generated notes: `gh release edit <version> --repo useforesight/hopp --notes-file release-notes/<version>.md`
5. Install the draft's DMG on a Mac and make a real call: ring, answer, share a screen, draw.
6. Publish the draft. This creates the tag and makes `latest.json` live, and installed apps start updating themselves. A bad release can only be replaced by a newer one.

## Moving from stock Hopp

Stock Hopp updates from upstream, so it never receives these releases. The first install is by hand: download the DMG from the release page and drag Hopp into Applications, replacing the old one. Sign-in and settings carry over.

macOS sees a new signature, so it asks again for each permission. If screen sharing still fails after granting it, run `tccutil reset All com.hopp.app` and reopen Hopp.

Stock Hopp and this build can call each other. Someone on stock sees drawings in the older style.
