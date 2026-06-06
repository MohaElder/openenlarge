# Releasing OpenEnlarge

Releases are built by `.github/workflows/release.yml` when a `v*` tag is pushed.

## One-time GitHub secrets (Settings → Secrets and variables → Actions)

**macOS (sign + notarize):**
- `APPLE_CERTIFICATE` — base64 of your Developer ID Application `.p12`: `base64 -i cert.p12 | pbcopy`
- `APPLE_CERTIFICATE_PASSWORD` — the `.p12` export password
- `APPLE_SIGNING_IDENTITY` — e.g. `Developer ID Application: Your Name (TEAMID)`
- `APPLE_ID` — your Apple ID email
- `APPLE_PASSWORD` — an app-specific password (appleid.apple.com → Sign-In and Security)
- `APPLE_TEAM_ID` — your 10-char Apple Team ID

**Windows (sign):**
- `WINDOWS_CERTIFICATE` — base64 of your code-signing `.pfx`
- `WINDOWS_CERTIFICATE_PASSWORD` — the `.pfx` password

**Auto-update (sign update artifacts):**
- `TAURI_SIGNING_PRIVATE_KEY` — contents of the updater private key file
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — its password (empty string if the key was generated without one)

If a platform's secrets are absent, that platform still builds — unsigned. If the
updater secrets are absent, builds still succeed but the update artifacts/`latest.json`
are not produced, so in-app auto-update stays dormant.

## Auto-update signing key

The in-app updater verifies downloads against a public key baked into
`app/src-tauri/tauri.conf.json` (`plugins.updater.pubkey`), and the release CI signs
the update artifacts with the matching private key. This key is **separate** from the
Apple/Windows code-signing certificates.

Generate the keypair once (already done for this repo; redo only if rotating):

```bash
cd app && npm run tauri signer generate -- -w ~/.tauri/openenlarge.key
# (omit the prompt with `--ci` to create a password-less key)
```

- Public key → `tauri.conf.json` `plugins.updater.pubkey` (committed).
- Private key (`~/.tauri/openenlarge.key`) → `TAURI_SIGNING_PRIVATE_KEY` secret;
  password (if any) → `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. **Keep the private key
  safe and never commit it** — losing it means you can no longer ship updates that
  existing installs will accept.

Two things to know about how updates roll out:

1. The updater endpoint is `releases/latest/download/latest.json`, which resolves
   only to a **published** release — so a draft build never prompts users (same as
   the website). Publishing the draft is what turns the update on.
2. Auto-update only works **from the first updater-enabled release forward**. Users on
   an older build download the next version manually once; after that it's automatic.

## Cutting a release

```bash
# bump version in app/src-tauri/tauri.conf.json + app/package.json first if needed
git tag v0.1.0
git push origin v0.1.0
```

The workflow creates a **draft** release with all installers attached. Review it, then publish.
