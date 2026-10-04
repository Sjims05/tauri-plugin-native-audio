# Troubleshooting

[← README](../README.MD)

## All platforms

**`unknown command plugin:native-audio|…`**

- Register the plugin in Rust: `.plugin(tauri_plugin_native_audio::init())`.
- Allow its commands: `native-audio:default` in a capability file (or the explicit permissions, see
  [Permissions](api.md#permissions)).
- Desktop: the `desktop` Cargo feature must be on (it is by default).

**A track is skipped / `error` in the state**

- The file is missing or unreadable, or the format isn't supported on that platform (see
  [Desktop](desktop.md#what-it-plays) for desktop's formats).

## Android

**Notification controls not shown**

- Call `initialize()` before playback, and start playback with `play()`.
- Grant the notification permission on Android 13+ (`initialize()` asks for it).

**Playback starts by itself when Android Auto connects**

- That's Android Auto's own "auto-resume media" setting; the plugin's `resumeLastQueue: "paused"` can't hold it back.
  Turn it off in Android Auto's settings to let `resumeLastQueue` decide.

**The app doesn't show up in Android Auto**

- Enable `carSupport` and rebuild, and for a sideloaded build turn on **Unknown sources** in Android Auto's developer
  settings. See [Android Auto](android-auto.md#setting-up).

## Desktop

**`tauri dev` stutters or plays with gaps**

- Debug builds decode slowly. Optimize the dependencies in the app's `src-tauri/Cargo.toml`:
  `[profile.dev.package."*"]` with `opt-level = 3`. Release builds are fine either way.

**Linux build fails with `alsa` / `pkg-config` errors**

- Install the ALSA development package (`libasound2-dev` on Debian / Ubuntu, `alsa-lib-devel` on Fedora) and
  `pkg-config`.

**The player isn't in the system's media controls (Windows media panel, MPRIS, Now Playing)**

- They appear once something is loaded, and need `mediaControls` on (the default).
- Windows: the controls belong to the app's first window, so there has to be one.

**Sound stays on the old device after changing outputs**

- With `setOutputDevice(null)` (the default) the player follows the system default and per-app routing. With a chosen
  device it stays on that one until it's unplugged. See [Output devices](desktop.md#output-devices).

## iOS

iOS is unmaintained: the code is kept from the original plugin so iOS apps that used it keep working, but it isn't
compiled or tested in this fork and doesn't get new features. Notes from the original plugin:

- Requires iOS 14.0+. For background playback, enable Background Modes → Audio for the app target.
- Plain `http://` URLs can be blocked by App Transport Security: use HTTPS or configure ATS exceptions.
- App closes right after launch on iOS 14: make sure the target's `LD_RUNPATH_SEARCH_PATHS` includes both
  `@executable_path/Frameworks` and `/usr/lib/swift` (usually not needed for iOS 16+), then clean and reinstall.
- Sources: remote URLs, `file://` URLs and paths; `asset://localhost/…` and `http://asset.localhost/…` resolve to local
  files, and local files without a known extension are inspected and remapped (mp3, wav, m4a, ogg).
- Playback runs through a single actor-owned runtime; `setSource` and `seekTo` use revision fencing to ignore stale
  callbacks.
