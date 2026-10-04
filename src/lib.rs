use serde::Deserialize;
use tauri::{
    plugin::{Builder, TauriPlugin},
    Runtime,
};

mod api;
pub use api::{
    Library, LibraryChild, LibraryFolder, LibraryItem, NativeAudio, NativeAudioExt, Queue, QueueItem, UpdateQueueOptions,
    UpdateQueueResult,
};

/// The desktop player. Public so its test program (examples/desktop_play.rs) can drive it directly.
#[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
#[doc(hidden)]
pub mod desktop;

#[cfg(all(feature = "mobile", target_os = "android"))]
const PLUGIN_IDENTIFIER: &str = "app.tauri.nativeaudio";

#[cfg(all(feature = "mobile", target_os = "ios"))]
tauri::ios_plugin_binding!(init_plugin_native_audio);

/// `plugins.native-audio` in `tauri.conf.json`.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// Android Auto support. Applied at build time: build.rs adds the manifest entries Android
    /// Auto needs when this is true, and removes them when it isn't.
    #[serde(default)]
    pub car_support: bool,
}

pub fn init<R: Runtime>() -> TauriPlugin<R, Option<Config>> {
    let builder = Builder::<R, Option<Config>>::new("native-audio");
    // Desktop: the commands are handled here in Rust (on Android and iOS they go to the native code).
    #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
    let builder = builder.invoke_handler(desktop::commands::handler());
    builder
        .setup(|_app, _api| {
            #[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
            {
                use tauri::Manager;
                // Saved settings, the last queue, ...: in the app's data folder.
                let data_dir = _app.path().app_data_dir().ok().map(|dir| dir.join("native-audio"));
                // resumeLastQueue runs at the app's first initialize(), once its UI is ready.
                _app.manage(desktop::player::DesktopAudio::new(Some(desktop::panel_config(_app.app_handle())), data_dir));
                _app.manage(api::NativeAudio::desktop(_app.app_handle().clone()));
            }
            #[cfg(all(feature = "mobile", target_os = "android"))]
            {
                use tauri::Manager;
                let handle = _api.register_android_plugin(PLUGIN_IDENTIFIER, "NativeAudioPlugin")?;
                _app.manage(api::NativeAudio::mobile(handle));
            }
            #[cfg(all(feature = "mobile", target_os = "ios"))]
            {
                let _ = _api.register_ios_plugin(init_plugin_native_audio)?;
            }
            Ok(())
        })
        .build()
}
