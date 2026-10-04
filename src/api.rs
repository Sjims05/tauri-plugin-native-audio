//! The player for the app's own Rust code, not only its JavaScript: what's queued, and updating the queue.
//! For work that runs while the app's UI may be asleep, e.g. a library scan keeping the playing queue in
//! step with the library.
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

/// What `updateQueue` changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub struct UpdateQueueResult {
    /// Tracks new to the playlist.
    pub added: u32,
    /// Tracks no longer in the playlist.
    pub removed: u32,
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

/// `app.native_audio()` on anything that has the app (the app handle, a window, ...).
pub trait NativeAudioExt<R: Runtime> {
    fn native_audio(&self) -> &NativeAudio<R>;
}

impl<R: Runtime, T: Manager<R>> NativeAudioExt<R> for T {
    fn native_audio(&self) -> &NativeAudio<R> {
        self.state::<NativeAudio<R>>().inner()
    }
}
