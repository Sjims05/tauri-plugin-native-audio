# tauri-plugin-native-audio

Native audio playback for Tauri apps: one JavaScript API over Media3 ExoPlayer on Android and a gapless Rust
player on desktop (Windows, macOS, Linux). Built for real music and podcast apps: the queue and everything around
it run natively, so playback keeps going with the screen locked, in the background, in Android Auto, and while
the app's WebView isn't running.

It started as a fork of [tauri-plugin-native-audio](https://github.com/uvarov-frontend/tauri-plugin-native-audio)
by uvarov-frontend and is still a drop-in replacement for it: same package names, commands and permissions.

## Platforms

| Feature | Android | Android Auto | Desktop | iOS¹ |
| --- | :---: | :---: | :---: | :---: |
| Background playback, system media controls | ✓ | ✓ | ✓ | ✓ |
| Native queue, shuffle, repeat, queue editing, `updateQueue` | ✓ | ✓ | ✓ | partly² |
| Gapless playback | ✓ | ✓ | ✓ | |
| Resume the last queue, saved settings | ✓ | ✓ | ✓ | partly² |
| Sleep timer | ✓ | ✓ | ✓ | ✓ |
| Volume (with a volume curve) | ✓ | ✓ | ✓ | |
| Output device choice | system | system | ✓ | system |
| Playback speed (`setRate`) | ✓ | ✓ | | ✓ |
| Browsable library, search and voice | | ✓ | | |
| Extra player buttons (like, seek, speed, sleep timer, …) | ✓³ | ✓ | | |
| Tracked lists, playback log, listening progress | ✓ | ✓ | ✓ | ✓ |

¹ **iOS is unmaintained:** the iOS code is kept from the original plugin so iOS apps that used it keep working,
but it isn't compiled or tested in this fork and doesn't get new features.
² Without the newer options (`updateQueue`, auto-resume modes, previous / next rules).
³ In the Android 13+ media controls, with `carSupport` enabled.

Every behavior can be configured or turned off: see [Options](docs/options.md).

## Guides

- [Playback and the queue](docs/playback.md): queue, editing, `updateQueue`, shuffle and repeat, previous / next,
  sleep timer, volume, resuming the last queue
- [Options](docs/options.md): every `setOptions` option in one table
- [Desktop](docs/desktop.md): the Rust player, formats, gapless, media keys, output devices, what's saved where
- [Android Auto](docs/android-auto.md): the library, search, artwork, designing for Auto, extra buttons
- [Syncing with your app](docs/syncing.md): tracked lists, listening progress, the playback log, the checkpoint
- [JavaScript API](docs/api.md): every type and command, validation, state, permissions
- [Troubleshooting](docs/troubleshooting.md)

## Install

Requires Rust **1.85** or newer.

This fork isn't published to crates.io or npm, so install it from GitHub. If you're switching from the original
plugin, remove it first; your imports and permission identifiers keep working.

`src-tauri/Cargo.toml`:

```toml
[dependencies]
tauri-plugin-native-audio = { git = "https://github.com/Sjims05/tauri-plugin-native-audio", tag = "v1.8.0" }
```

The JavaScript bindings:

```sh
npm install github:Sjims05/tauri-plugin-native-audio#v1.8.0
# or: pnpm add / yarn add github:Sjims05/tauri-plugin-native-audio#v1.8.0
```

Use the same tag for both, so the Rust and JavaScript sides match.

### Platforms to build

Two Cargo features choose what's built, both on by default: `mobile` (Android and iOS) and `desktop` (the Rust
player). An app for one side only can leave the other out, for example desktop only:

```toml
tauri-plugin-native-audio = { git = "…", tag = "v1.8.0", default-features = false, features = ["desktop"] }
```

`carSupport` (Android Auto) needs `mobile`. Linux builds need the ALSA development package (`libasound2-dev` on
Debian / Ubuntu, `alsa-lib-devel` on Fedora) and `pkg-config`.

### Local development

To work on the plugin itself, clone it next to your app and use path dependencies:

```toml
tauri-plugin-native-audio = { path = "../../tauri-plugin-native-audio" }
```

```sh
npm install ../tauri-plugin-native-audio
```

Tell Vite to resolve `@tauri-apps/api` from the app, so there's one copy (`vite.config.ts`):
`resolve: { dedupe: ["@tauri-apps/api"] }`.

The JavaScript bindings are written in `guest-js/index.ts`; after changing them, run `npm install` (once) and
`npm run build` in the plugin, which updates the committed `guest-js/dist-js`.

## Setting up

1. Register the plugin (`src-tauri/src/lib.rs`):

   ```rust
   tauri::Builder::default()
       .plugin(tauri_plugin_native_audio::init())
   ```

2. Allow its commands in a capability file (for example `src-tauri/capabilities/default.json`):

   ```json
   { "permissions": ["core:default", "native-audio:default"] }
   ```

   `native-audio:default` allows every command; see [Permissions](docs/api.md#permissions) for a stricter setup.

3. Optional, in `tauri.conf.json`: Android Auto support, applied at build time (see [Android Auto](docs/android-auto.md)):

   ```json
   { "plugins": { "native-audio": { "carSupport": true } } }
   ```

Platform requirements:

- **Android** 8.0+ (API 26). `play()` starts a foreground service for the media notification; on Android 13+ the
  notification permission is requested in `initialize()`. The plugin's manifest declares `INTERNET`,
  `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_MEDIA_PLAYBACK`, `POST_NOTIFICATIONS` and `WAKE_LOCK`.
- **Desktop**: plays local files (paths or `file://` URLs). See [Desktop](docs/desktop.md).
- **iOS** (unmaintained, see above) 14.0+, with Background Modes → Audio enabled for background playback.

## Quick start

```ts
import { initialize, setQueue, play, addStateListener, setOptions } from "tauri-plugin-native-audio-api";

const stop = await addStateListener((state) => {
  console.log(state.status, state.queueIndex, state.currentTime, state.duration);
});

await initialize(); // once the UI is ready (desktop resumes the last queue here)
await setOptions({ resumeLastQueue: "paused" });

await setQueue({
  items: [
    { src: "/music/1.flac", id: 1, title: "Song 1", artist: "Artist" },
    { src: "/music/2.flac", id: 2, title: "Song 2", artist: "Artist" },
  ],
  sourceId: "playlist-7", // which playlist this is, for "recently played" lists
});
await play();
```

From here: [the queue and playback](docs/playback.md), then [the options](docs/options.md).

## Notice

This plugin was fully developed with the help of AI.

## License

MIT or Apache-2.0, where applicable.
