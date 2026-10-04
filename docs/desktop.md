# Desktop

[← README](../README.MD) · [Playback](playback.md) · [Options](options.md)

On Windows, macOS and Linux the plugin plays through its own player, written in Rust, with the same commands,
state and rules as on Android: the app code doesn't change between platforms.

- [What it plays](#what-it-plays)
- [Gapless playback](#gapless-playback)
- [Media keys and the system media controls](#media-keys-and-the-system-media-controls)
- [Output devices](#output-devices)
- [What's saved, and where](#whats-saved-and-where)
- [Not on desktop](#not-on-desktop)
- [Building](#building)

## What it plays

- **Sources:** local files, as a path or a `file://` URL. Streaming URLs (`https://…`) aren't supported on desktop.
- **Formats:** MP3, FLAC, AAC and ALAC in MP4 / M4A, Ogg Vorbis, WAV / PCM (decoded with
  [Symphonia](https://github.com/pdeljanov/Symphonia)).
- **Output:** every track is converted to the output device's own sample rate and channel count, so files of any
  rate play without the device switching modes.
- **Errors:** a track that can't be played (missing, unsupported, broken) is skipped and reported in the state's
  `error`, which clears with the next play / skip / load. The status is `error` only when nothing in the queue
  could be played.
- **Status:** `loading` while a track is being opened; commands that change track resolve once it's open, so the
  state they return already shows it.

## Gapless playback

The next track is opened and decoded a couple of seconds before the current one ends, and its first sample follows
the current one's last with nothing in between:

- the silence encoders add to the start and end of files is removed: MP3 (LAME / Xing info) and AAC in MP4
  (iTunSMPB, edit lists);
- tracks at the same sample rate share one continuous sample-rate converter, so there's no seam at the switch;
- repeat `one` loops the track gaplessly;
- seeking is sample-accurate.

Queue edits that change what comes next are picked up even when the next track was already prepared: playback
continues from the same spot with the right track after it.

## Media keys and the system media controls

The player shows up in the system's media controls and the media keys control it:

| | Windows | macOS | Linux |
| --- | --- | --- | --- |
| Where | The media panel (volume flyout, lock screen) | Now Playing | MPRIS (desktop media widgets, `playerctl`) |
| Tested | ✓ | not yet | not yet |

- They show the title, artist, album and cover, and the position. The app's `title`, `artist` and `artworkUrl` come
  first; what's missing comes from the file's own tags and embedded cover.
- Play, pause, previous, next, seeking and the position bar work. Stop pauses (the queue stays). Fast forward /
  rewind seek by the [skip interval](playback.md#previous-and-next) (10 s without one). On Linux the volume works
  too.
- `setSkipInterval` and the `previousRestartsAfterSeconds` / `nextAtEnd` options apply to these buttons too.
- `setOptions({ mediaControls: false })` takes the player out of the system's controls.
- Windows: the controls belong to the app's first window. Embedded covers are written to a temporary file for the
  media panel and removed again.
- Linux: uses the pure-Rust D-Bus implementation (no `libdbus` needed). The player's D-Bus name comes from the app's
  identifier.

## Output devices

```ts
import { getOutputDevices, setOutputDevice } from "tauri-plugin-native-audio-api";

const { devices, selected, active } = await getOutputDevices();
// devices: [{ id, name, isDefault }]; selected: the chosen id or null; active: the id playing now
await setOutputDevice(devices[1].id); // play on this one
await setOutputDevice(null);          // follow the system default (the default)
```

- **Following the system default:** the player plays on the system's default output and moves along when it
  changes. On Windows it uses the default endpoint itself, so per-app routing (Windows' app volume settings,
  SteelSeries Sonar and the like) applies, and a change of default is picked up right away.
- **A chosen device:** `id`s are stable across runs. When the chosen device goes away (unplugged), playback moves to
  the default; when it's back, playback moves back.
- **Device lost:** when the output stops working, the player reopens it and plays on from the same spot.
- The choice is saved, so the next run plays on the same device. On Android, `getOutputDevices` returns an empty
  list and `setOutputDevice` does nothing: Android picks the output itself.

## What's saved, and where

In the app's data folder (Tauri's `app_data_dir`), under `native-audio/`, as JSON written atomically:

| File | What |
| --- | --- |
| `settings.json` | Volume, volume curve, output device, skip interval and the [options](options.md) |
| `queue.json` | The last queue: items, play order, shuffle, repeat, `sourceId` |
| `queue-position.json` | Where it was: track and position (saved about once a second while playing) |
| `playback-events.json` | The [playback log](syncing.md#playback-log) until the app picks it up (newest 500) |
| `tracked-lists.json` | [Tracked lists](syncing.md#tracked-lists), their definitions and unacknowledged changes |
| `item-progress.json` | [Listening progress](syncing.md#listening-progress) |
| `progress-checkpoint.json` | The [progress checkpoint](syncing.md#progress-checkpoint) |

`dispose()` stops playback and empties the queue, but keeps the saved queue for `restoreLastQueue()`.

## Not on desktop

- `setRate` (playback speed) rejects with "isn't available on desktop yet". Changing speed without changing pitch
  needs time-stretching, which the player doesn't do.
- Android Auto and its buttons: `setLibrary`, `setControls`, `setControlActive` and the control press log are
  accepted and do nothing, so the same app code runs everywhere.
- `pausedKeepAliveMinutes`, `keepAliveWhileCarConnected` and `keepQueueOnStop` are Android-only.

## Building

- The player is the `desktop` Cargo feature (on by default). A desktop-only app can drop the Android code with
  `default-features = false, features = ["desktop"]`.
- Rust 1.85 or newer.
- Linux needs the ALSA development package (`libasound2-dev` on Debian / Ubuntu, `alsa-lib-devel` on Fedora) and
  `pkg-config`.
- Decoding is heavy in unoptimized builds. If `tauri dev` stutters, optimize dependencies in the app's
  `src-tauri/Cargo.toml`; your own code stays quick to compile:

  ```toml
  [profile.dev.package."*"]
  opt-level = 3
  ```

- `examples/desktop_play.rs` plays, checks or renders files through the player from the command line, for testing:
  `cargo run --release --example desktop_play -- play a.flac b.flac`.
