//! The player for the app's own Rust code, not only its JavaScript: what's queued, updating the queue, and
//! the Android Auto library. For work that runs while the app's UI may be asleep, e.g. a library scan
//! keeping the playing queue and Android Auto in step with the library.
//!
//! ```ignore
//! use tauri_plugin_native_audio::{NativeAudioExt, UpdateQueueOptions};
//! let queue = app.native_audio().get_queue()?;
//! if queue.source_id.as_deref() == Some("album:12") {
//!     app.native_audio().update_queue(items, UpdateQueueOptions::default())?;
//! }
//! ```
//!
//! Desktop: the Rust player itself. Android: the plugin's native code (call from a background thread:
//! it waits for the answer). iOS: not supported yet (errors).

use serde::{Deserialize, Serialize};
use tauri::{Manager, Runtime};

/// What plays on another device the app controls (its own connection to it), for the phone's media session,
/// notification and volume buttons to show and control instead of this device's player: [`NativeAudio::set_remote_playback`].
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePlayback {
    /// The other device's name ("Playing on …").
    pub device: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    /// A picture of the song (a local file, or a URL), as a queue item's.
    pub artwork_url: Option<String>,
    pub playing: bool,
    /// Seconds into the song, now (it counts on while playing).
    pub position: f64,
    pub duration: f64,
    /// Whether there's a song after this one.
    pub has_next: bool,
    /// The other device's volume in steps, 0 to `max_volume` (each press of a volume button is one step).
    pub volume: u32,
    pub max_volume: u32,
    /// Its shuffle and repeat ("off", "all", "one"): the `setControls` shuffle and repeat buttons show and change them.
    pub shuffle: bool,
    pub repeat: String,
    /// The song's id in this app, when it has it too: the `setControls` custom buttons (like) are for it.
    pub item_id: Option<i64>,
}

/// A control for the other device, from the phone's media session, notification or volume buttons
/// ([`NativeAudio::on_remote_command`]).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "cmd", rename_all = "camelCase")]
pub enum RemoteCommand {
    Play,
    Pause,
    Next,
    Previous,
    /// To this many seconds into the song.
    Seek { position: f64 },
    /// The volume to `step` of `max_volume`.
    #[serde(rename_all = "camelCase")]
    Volume { step: u32, max_volume: u32 },
    /// The `setControls` shuffle button.
    Shuffle { on: bool },
    /// The `setControls` repeat button: "off", "all" or "one".
    Repeat { mode: String },
}

/// A queue item, as in the JavaScript API.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    /// 1 or higher to count in playback events and progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    pub src: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artwork_url: Option<String>,
}

/// The queue, as `getQueue` gives it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Queue {
    pub items: Vec<QueueItem>,
    /// -1 when the queue is empty.
    pub current_index: i64,
    #[serde(default)]
    pub play_order: Vec<i64>,
    /// The playable folder (playlist, album) the queue was started from, if any.
    #[serde(default)]
    pub source_id: Option<String>,
}

/// `updateQueue`'s options (left out: the defaults), as in the JavaScript API.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateQueueOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    /// "finish" (default) or "skip".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed_current: Option<String>,
    /// "remove" (default) or "keep".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed_items: Option<String>,
    /// "inPlace" (default), "end" or "next".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_items: Option<String>,
    /// "playlist" (default) or "queue".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
}

/// The player's state, as `getState` gives it (the parts Rust code needs).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlayerState {
    /// "idle", "loading", "playing", "ended" or "error".
    pub status: String,
    /// Seconds into the current item.
    pub current_time: f64,
    /// Seconds.
    pub duration: f64,
    pub is_playing: bool,
    /// -1 when nothing is loaded.
    pub queue_index: i64,
    pub queue_length: i64,
    pub current_id: Option<i64>,
    pub shuffle: bool,
    /// "off", "all" or "one".
    pub repeat_mode: String,
    /// 0 to 1 (as set, before the volume curve).
    pub volume: f64,
}

/// What `updateQueue` changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub struct UpdateQueueResult {
    /// Tracks new to the playlist.
    pub added: u32,
    /// Tracks no longer in the playlist.
    pub removed: u32,
}

/// The browsable library for Android Auto, as `setLibrary` takes it in the JavaScript API (see
/// docs/android-auto.md).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub items: Vec<LibraryItem>,
    /// Top-level folders: Android Auto's tabs (it shows up to 4).
    pub root: Vec<LibraryFolder>,
    /// Search and "play ... on <app>" by voice (default true).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<bool>,
    /// Shown while `root` is empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub empty_message: Option<String>,
    /// Items without `artwork_url` show their file's embedded cover (default true).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedded_artwork: Option<bool>,
    /// Folders without `artwork_url`: "collage" (default), "first" or "none".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_artwork: Option<String>,
}

/// A playable item, defined once in `items` and listed by id in folders.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItem {
    pub id: i64,
    pub src: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artwork_url: Option<String>,
    /// A header above a run of entries with the same group.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// 0 to 1: not played, partly played, played.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
}

/// A folder: a tab, a playlist, an album...
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFolder {
    /// Unique across all folders; a played folder becomes the queue's `source_id`.
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artwork_url: Option<String>,
    /// Played as a whole; picking an item inside plays the folder from there.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub playable: bool,
    /// With `playable`: tapping it in Android Auto opens it instead of playing it (its items still play it
    /// from there, and voice still plays it).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub open_on_tap: bool,
    /// "list" or "grid".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<LibraryChild>,
    /// Show a tracked list instead of `children` (it updates on its own).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracked_list: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

/// A folder's child: a sub-folder, or an item's id.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum LibraryChild {
    Folder(LibraryFolder),
    Item(i64),
}

/// The player, for Rust code: `app.native_audio()` (see [`NativeAudioExt`]).
pub struct NativeAudio<R: Runtime> {
    #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
    app: tauri::AppHandle<R>,
    #[cfg(all(feature = "mobile", target_os = "android"))]
    handle: tauri::plugin::PluginHandle<R>,
    #[allow(dead_code)]
    runtime: std::marker::PhantomData<fn() -> R>,
}

type Result<T> = std::result::Result<T, String>;

impl<R: Runtime> NativeAudio<R> {
    #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
    pub(crate) fn desktop(app: tauri::AppHandle<R>) -> Self {
        Self { app, runtime: std::marker::PhantomData }
    }

    #[cfg(all(feature = "mobile", target_os = "android"))]
    pub(crate) fn mobile(handle: tauri::plugin::PluginHandle<R>) -> Self {
        Self { handle, runtime: std::marker::PhantomData }
    }

    /// The queue: its items, the current one, and the playlist or album it was started from.
    pub fn get_queue(&self) -> Result<Queue> {
        #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
        {
            let audio = self.app.state::<crate::desktop::player::DesktopAudio>();
            serde_json::from_value(audio.queue()).map_err(|e| e.to_string())
        }
        #[cfg(all(feature = "mobile", target_os = "android"))]
        {
            self.handle.run_mobile_plugin("getQueue", ()).map_err(|e| e.to_string())
        }
        #[cfg(not(any(
            all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))),
            all(feature = "mobile", target_os = "android")
        )))]
        Err("not supported on this platform".into())
    }

    /// The playlist the queue plays changed: `items` is its new content, matched by id (see the
    /// JavaScript `updateQueue`).
    pub fn update_queue(&self, items: Vec<QueueItem>, options: UpdateQueueOptions) -> Result<UpdateQueueResult> {
        #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
        {
            let audio = self.app.state::<crate::desktop::player::DesktopAudio>();
            let items = serde_json::to_value(items).and_then(serde_json::from_value).map_err(|e| e.to_string())?;
            let UpdateQueueOptions { source_id, removed_current, removed_items, new_items, order } = options;
            let (options, skip_removed_current) =
                crate::desktop::commands::update_options(removed_current, removed_items, new_items, order)?;
            let result = audio.update_queue(items, source_id, options, skip_removed_current)?;
            serde_json::from_value(result).map_err(|e| e.to_string())
        }
        #[cfg(all(feature = "mobile", target_os = "android"))]
        {
            #[derive(Serialize)]
            #[serde(rename_all = "camelCase")]
            struct Payload {
                items: Vec<QueueItem>,
                #[serde(flatten)]
                options: UpdateQueueOptions,
            }
            self.handle.run_mobile_plugin("updateQueue", Payload { items, options }).map_err(|e| e.to_string())
        }
        #[cfg(not(any(
            all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))),
            all(feature = "mobile", target_os = "android")
        )))]
        {
            let _ = (items, options);
            Err("not supported on this platform".into())
        }
    }
}

impl<R: Runtime> NativeAudio<R> {
    /// The browsable library for Android Auto, saved on the device (Auto uses it with the app closed).
    /// Replaces the previous one. Elsewhere it's accepted and not used.
    pub fn set_library(&self, library: &Library) -> Result<()> {
        #[cfg(all(feature = "mobile", target_os = "android"))]
        {
            self.handle.run_mobile_plugin::<serde_json::Value>("setLibrary", library).map(|_| ()).map_err(|e| e.to_string())
        }
        #[cfg(not(all(feature = "mobile", target_os = "android")))]
        {
            let _ = library;
            Ok(())
        }
    }
}

impl<R: Runtime> NativeAudio<R> {
    /// The player's state: what's playing, where, playing or not, shuffle and repeat.
    pub fn get_state(&self) -> Result<PlayerState> {
        #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
        {
            let audio = self.app.state::<crate::desktop::player::DesktopAudio>();
            serde_json::from_value(audio.state()).map_err(|e| e.to_string())
        }
        #[cfg(all(feature = "mobile", target_os = "android"))]
        {
            self.handle.run_mobile_plugin("getState", ()).map_err(|e| e.to_string())
        }
        #[cfg(not(any(
            all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))),
            all(feature = "mobile", target_os = "android")
        )))]
        Err("not supported on this platform".into())
    }

    /// Loads a queue, paused, at `start_index` and `start_position` seconds into it (like `setQueue`).
    pub fn set_queue(&self, items: Vec<QueueItem>, start_index: usize, start_position: f64, source_id: Option<String>) -> Result<()> {
        #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
        {
            let audio = self.app.state::<crate::desktop::player::DesktopAudio>();
            let items = serde_json::to_value(items).and_then(serde_json::from_value).map_err(|e| e.to_string())?;
            audio.load(items, start_index, start_position, source_id).map(|_| ())
        }
        #[cfg(all(feature = "mobile", target_os = "android"))]
        {
            #[derive(Serialize)]
            #[serde(rename_all = "camelCase")]
            struct Payload {
                items: Vec<QueueItem>,
                start_index: usize,
                start_position: f64,
                #[serde(skip_serializing_if = "Option::is_none")]
                source_id: Option<String>,
            }
            self.handle
                .run_mobile_plugin::<serde_json::Value>("setQueue", Payload { items, start_index, start_position, source_id })
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        #[cfg(not(any(
            all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))),
            all(feature = "mobile", target_os = "android")
        )))]
        {
            let _ = (items, start_index, start_position, source_id);
            Err("not supported on this platform".into())
        }
    }

    /// Plays (what's loaded, from where it is).
    pub fn play(&self) -> Result<()> {
        #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
        return self.desktop_audio().play().map(|_| ());
        #[allow(unreachable_code)]
        self.mobile_control("play")
    }

    pub fn pause(&self) -> Result<()> {
        #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
        return self.desktop_audio().pause().map(|_| ());
        #[allow(unreachable_code)]
        self.mobile_control("pause")
    }

    /// The next track (as the next button does: `nextAtEnd` at the end of the queue).
    pub fn next(&self) -> Result<()> {
        #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
        return self.desktop_audio().next().map(|_| ());
        #[allow(unreachable_code)]
        self.mobile_control("next")
    }

    /// The previous track, or back to the start of this one (`previousRestartsAfterSeconds`).
    pub fn previous(&self) -> Result<()> {
        #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
        return self.desktop_audio().previous().map(|_| ());
        #[allow(unreachable_code)]
        self.mobile_control("previous")
    }

    /// Jumps to `position` seconds in the track.
    pub fn seek_to(&self, position: f64) -> Result<()> {
        #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
        return self.desktop_audio().seek_to(position).map(|_| ());
        #[allow(unreachable_code)]
        self.mobile_call("seekTo", serde_json::json!({ "position": position }))
    }

    /// The volume, 0 to 1 (through the volume curve, as the app's slider).
    pub fn set_volume(&self, volume: f64) -> Result<()> {
        #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
        {
            self.desktop_audio().set_volume(volume);
            return Ok(());
        }
        #[allow(unreachable_code)]
        self.mobile_call("setVolume", serde_json::json!({ "volume": volume }))
    }

    /// The app controls another device (`Some`: what plays there, updated as it changes), or not anymore (`None`).
    /// Meanwhile this device's player is paused and its media controls show and control that device (their presses
    /// come to [`Self::on_remote_command`]): on Android the media session, the notification, the lock screen and the
    /// volume buttons; on a computer the system's media controls (the Windows media panel, the media keys).
    #[allow(unused_variables)]
    pub fn set_remote_playback(&self, playback: Option<RemotePlayback>) -> Result<()> {
        #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
        {
            self.desktop_audio().set_remote_playback(playback);
            return Ok(());
        }
        #[cfg(all(feature = "mobile", target_os = "android"))]
        return self.mobile_call(
            "setRemotePlayback",
            match playback {
                Some(playback) => serde_json::to_value(playback).map_err(|e| e.to_string())?,
                // No device: back to this device.
                None => serde_json::json!({}),
            },
        );
        #[allow(unreachable_code)]
        Ok(())
    }

    /// Calls `handler` with each control for the other device ([`Self::set_remote_playback`]); register it once.
    #[allow(unused_variables)]
    pub fn on_remote_command(&self, handler: impl Fn(RemoteCommand) + Send + Sync + 'static) -> Result<()> {
        #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
        {
            self.desktop_audio().set_remote_commands(std::sync::Arc::new(handler));
            return Ok(());
        }
        #[cfg(all(feature = "mobile", target_os = "android"))]
        {
            #[derive(Serialize)]
            struct Listen {
                event: &'static str,
                handler: tauri::ipc::Channel<serde_json::Value>,
            }
            let channel = tauri::ipc::Channel::new(move |body| {
                if let tauri::ipc::InvokeResponseBody::Json(json) = body {
                    if let Ok(command) = serde_json::from_str::<RemoteCommand>(&json) {
                        handler(command);
                    }
                }
                Ok(())
            });
            return self.mobile_call("registerListener", serde_json::to_value(Listen { event: "native_audio_remote_command", handler: channel }).map_err(|e| e.to_string())?);
        }
        #[allow(unreachable_code)]
        Ok(())
    }

    /// Plays the queue's track at `index` (its place in the queue's own order).
    pub fn skip_to(&self, index: usize) -> Result<()> {
        #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
        return self.desktop_audio().skip_to(index).map(|_| ());
        #[allow(unreachable_code)]
        self.mobile_call("skipTo", serde_json::json!({ "index": index }))
    }

    /// Adds songs to the queue: after the playing one (`play_next`), or at the end. With nothing loaded, they become
    /// the queue (paused).
    pub fn add_to_queue(&self, items: Vec<QueueItem>, play_next: bool) -> Result<()> {
        #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
        {
            let items = serde_json::to_value(items).and_then(serde_json::from_value).map_err(|e| e.to_string())?;
            return self.desktop_audio().add_to_queue(items, play_next).map(|_| ());
        }
        #[allow(unreachable_code)]
        self.mobile_call("addToQueue", serde_json::json!({ "items": items, "playNext": play_next }))
    }

    /// Takes the queue's item at `index` (its place in the queue's own order) out of it.
    pub fn remove_from_queue(&self, index: usize) -> Result<()> {
        #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
        return self.desktop_audio().remove_from_queue(index).map(|_| ());
        #[allow(unreachable_code)]
        self.mobile_call("removeFromQueue", serde_json::json!({ "index": index }))
    }

    /// Moves an item in the order the queue plays in (shuffled or not): from place `from` to place `to` of that order.
    pub fn move_in_play_order(&self, from: usize, to: usize) -> Result<()> {
        #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
        return self.desktop_audio().move_in_play_order(from, to).map(|_| ());
        #[allow(unreachable_code)]
        self.mobile_call("moveInPlayOrder", serde_json::json!({ "from": from, "to": to }))
    }

    /// A command with arguments on Android; elsewhere not there.
    #[allow(dead_code, unused_variables)]
    fn mobile_call(&self, command: &str, payload: serde_json::Value) -> Result<()> {
        #[cfg(all(feature = "mobile", target_os = "android"))]
        return self.handle.run_mobile_plugin::<serde_json::Value>(command, payload).map(|_| ()).map_err(|e| e.to_string());
        #[allow(unreachable_code)]
        Err("not supported on this platform".into())
    }

    #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
    fn desktop_audio(&self) -> tauri::State<'_, crate::desktop::player::DesktopAudio> {
        self.app.state::<crate::desktop::player::DesktopAudio>()
    }

    /// A control without arguments on Android (`command`); elsewhere not there.
    #[allow(dead_code, unused_variables)]
    fn mobile_control(&self, command: &str) -> Result<()> {
        #[cfg(all(feature = "mobile", target_os = "android"))]
        return self.handle.run_mobile_plugin::<serde_json::Value>(command, ()).map(|_| ()).map_err(|e| e.to_string());
        #[allow(unreachable_code)]
        Err("not supported on this platform".into())
    }

    pub fn set_shuffle(&self, enabled: bool) -> Result<()> {
        #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
        {
            self.app.state::<crate::desktop::player::DesktopAudio>().set_shuffle(enabled).map(|_| ())
        }
        #[cfg(all(feature = "mobile", target_os = "android"))]
        {
            #[derive(Serialize)]
            struct Payload {
                enabled: bool,
            }
            self.handle.run_mobile_plugin::<serde_json::Value>("setShuffle", Payload { enabled }).map(|_| ()).map_err(|e| e.to_string())
        }
        #[cfg(not(any(
            all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))),
            all(feature = "mobile", target_os = "android")
        )))]
        {
            let _ = enabled;
            Err("not supported on this platform".into())
        }
    }

    /// "off", "all" or "one".
    pub fn set_repeat_mode(&self, mode: &str) -> Result<()> {
        #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
        {
            self.app.state::<crate::desktop::player::DesktopAudio>().set_repeat_mode(mode).map(|_| ())
        }
        #[cfg(all(feature = "mobile", target_os = "android"))]
        {
            #[derive(Serialize)]
            struct Payload<'a> {
                mode: &'a str,
            }
            self.handle.run_mobile_plugin::<serde_json::Value>("setRepeatMode", Payload { mode }).map(|_| ()).map_err(|e| e.to_string())
        }
        #[cfg(not(any(
            all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))),
            all(feature = "mobile", target_os = "android")
        )))]
        {
            let _ = mode;
            Err("not supported on this platform".into())
        }
    }
}

/// `app.native_audio()` on anything that has the app (the app handle, a window, ...).
pub trait NativeAudioExt<R: Runtime> {
    fn native_audio(&self) -> &NativeAudio<R>;
}

impl<R: Runtime, T: Manager<R>> NativeAudioExt<R> for T {
    fn native_audio(&self) -> &NativeAudio<R> {
        self.state::<NativeAudio<R>>().inner()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_library_is_sent_in_the_javascript_apis_shape() {
        let library = Library {
            items: vec![LibraryItem { id: 1, src: "/m/a.mp3".into(), title: Some("A".into()), group: Some("X".into()), ..Default::default() }],
            root: vec![LibraryFolder {
                id: "albums".into(),
                title: "Albums".into(),
                style: Some("grid".into()),
                children: vec![
                    LibraryChild::Folder(LibraryFolder {
                        id: "album:1".into(),
                        title: "One".into(),
                        playable: true,
                        children: vec![LibraryChild::Item(1)],
                        ..Default::default()
                    }),
                    LibraryChild::Item(1),
                ],
                ..Default::default()
            }],
            empty_message: Some("Nothing yet".into()),
            ..Default::default()
        };
        let json = serde_json::to_value(&library).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "items": [{ "id": 1, "src": "/m/a.mp3", "title": "A", "group": "X" }],
                "root": [{
                    "id": "albums",
                    "title": "Albums",
                    "style": "grid",
                    "children": [{ "id": "album:1", "title": "One", "playable": true, "children": [1] }, 1],
                }],
                "emptyMessage": "Nothing yet",
            })
        );
    }
}
