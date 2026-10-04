# Options

[← README](../README.MD) · [Playback](playback.md) · [API](api.md)

```ts
import { setOptions } from "tauri-plugin-native-audio-api";

await setOptions({ resumeLastQueue: "play", nextAtEnd: "first" });
```

Options are saved on the device (on desktop too), so they apply from the next start on and, on Android, also when
Android Auto starts the app in the background. Only the options passed are changed.

| Option | Values (default first) | Android | Desktop | What it does |
| --- | --- | :---: | :---: | --- |
| `resumeLastQueue` | `"paused"`, `"play"`, `"off"` (or `true` / `false`) | ✓ | ✓ | Load the last queue automatically, see [below](#resumelastqueue) |
| `previousRestartsAfterSeconds` | `3`, any seconds ≥ 0 | ✓ | ✓ | Past this, previous restarts the track; `0`: always the previous track |
| `nextAtEnd` | `"nothing"`, `"first"` | ✓ | ✓ | Next on the last track (repeat off): nothing, or the first track, paused |
| `repeatAddedTracks` | `false`, `true` | ✓ | ✓ | Keep tracks added with `addToQueue` when the queue repeats |
| `volumeCurve` | `"quadratic"`, `"cubic"`, `"linear"` | ✓ | ✓ | How a `setVolume` value becomes loudness |
| `trackProgress` | `false`, `true` | ✓ | ✓ | Remember how far each item played, see [Syncing](syncing.md#listening-progress) |
| `mediaControls` | `true`, `false` | | ✓ | Show the player in the system's media controls, with the media keys |
| `pausedKeepAliveMinutes` | `30`, any minutes ≥ 0 | ✓ | | How long the controls stay after a pause |
| `keepAliveWhileCarConnected` | `true`, `false` | ✓ (`carSupport`) | | No time limit while Android Auto is connected |
| `keepQueueOnStop` | `true`, `false` | ✓ | | A stop from outside the app pauses instead |

Other settings that are saved the same way have their own calls:

| Call | Android | Desktop | See |
| --- | :---: | :---: | --- |
| `setSkipInterval(seconds)` | ✓ | ✓ | [Previous and next](playback.md#previous-and-next) |
| `setVolume(volume)` | ✓ | ✓ | [Volume](playback.md#volume) |
| `setOutputDevice(id)` | | ✓ | [Output devices](desktop.md#output-devices) |
| `setTrackedLists(lists)` | ✓ | ✓ | [Tracked lists](syncing.md#tracked-lists) |
| `setLibrary(library)` | ✓ | | [Android Auto](android-auto.md#the-library) |
| `setControls(controls)` | ✓ | | [Extra buttons](android-auto.md#extra-buttons) |

`updateQueue`'s options are per call: see [Updating the queue](playback.md#updating-the-queue-from-a-changed-playlist).

Options a platform doesn't use are accepted and ignored, so the same app code runs everywhere.

## Details

### resumeLastQueue

When to load the last queue by itself: on Android when the audio service starts with nothing loaded (Android Auto
connecting, a headset button with the app closed), on desktop at the app's first `initialize()`. `"paused"` loads
it where it was, `"play"` also plays it, `"off"` leaves it to the app's `restoreLastQueue()`. Android Auto may
start playback itself when it connects (its own "auto-resume media" setting), whatever this is set to. More in
[Resuming the last queue](playback.md#resuming-the-last-queue).

### previousRestartsAfterSeconds and nextAtEnd

They apply to the app's `previous()` / `next()` and to the system buttons alike (notification, lock screen, headset,
Android Auto, desktop media keys). With `nextAtEnd: "first"` the next button stays enabled on the last track. See
[Previous and next](playback.md#previous-and-next).

### repeatAddedTracks

Whether tracks added with `addToQueue` stay in the queue when it repeats (repeat `all`). By default they play once
and are dropped when the queue starts over.

### volumeCurve

Hearing is roughly logarithmic, so with `linear` 50% sounds only a little quieter. `quadratic` makes 50% about half
as loud (-12 dB); `cubic` gives finer control at low volumes (50% = -18 dB). The sleep timer's fade-out follows it.

### mediaControls (desktop)

Shows the player in the Windows media panel, macOS Now Playing or MPRIS on Linux, and lets the media keys control
it. `false` takes it out. See [Desktop](desktop.md#media-keys-and-the-system-media-controls).

### pausedKeepAliveMinutes (Android)

How long the playback controls stay after a pause, in the notification, on the lock screen and in Android Auto's
player, so playback is easy to pick up again (for example after a conversation in the car). Meanwhile the
notification can't be swiped away. Afterwards the audio service leaves the foreground and Android may remove the
controls. `0` lets them go right away.

### keepAliveWhileCarConnected (Android, with carSupport)

While the phone is connected to Android Auto, the controls stay after a pause with no time limit, so Android Auto's
player stays too; `pausedKeepAliveMinutes` starts counting when Android Auto disconnects. The connection is read
from Android Auto itself (the Android for Cars library), since media sessions can't tell a paused car connection
from a closed one.

### keepQueueOnStop (Android)

When something outside the app stops playback, the player pauses instead and keeps the queue, so the notification
and Android Auto's player stay and playback continues where it was. Android's media panel sends a stop to players
that were paused for about 10 minutes, which would otherwise empty Android Auto's player even while
`keepAliveWhileCarConnected` keeps the app running. Bluetooth devices and Android Auto can send a stop too. `false`
lets a stop unload the queue.
