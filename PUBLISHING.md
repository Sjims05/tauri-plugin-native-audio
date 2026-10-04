# Releasing

This fork is installed from GitHub tags (see the README), not from crates.io or npm: the package names are the
original plugin's.

## 1) Checks

From the repo root:

```bash
cargo clippy --features desktop --all-targets   # no warnings
cargo test --lib                                # the desktop player's queue and storage tests
cargo check --no-default-features --features mobile
npm install && npm run build                    # rebuild guest-js/dist-js from guest-js/index.ts
git status guest-js/dist-js                     # the built files are committed: no changes left over
npm pack --dry-run                              # the JavaScript package: guest-js/dist-js and the licenses
```

The Android tests run from an app that uses the plugin (its generated Android project includes the plugin as
`:tauri-plugin-native-audio`):

```bash
./gradlew :tauri-plugin-native-audio:testDebugUnitTest
```

## 2) Version

- Bump `version` in `Cargo.toml` and in `package.json` (the same number).
- Update the tag in the README's install instructions.
- Commit and tag:

```bash
git commit -am "release: vX.Y.Z"
git tag vX.Y.Z
git push origin main --tags
```
