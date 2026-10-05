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
