use serde::Deserialize;
use tauri::{
    plugin::{Builder, TauriPlugin},
    Runtime,
};

#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "app.tauri.nativeaudio";

#[cfg(target_os = "ios")]
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
    Builder::<R, Option<Config>>::new("native-audio")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            {
                let _ = _api.register_android_plugin(PLUGIN_IDENTIFIER, "NativeAudioPlugin")?;
            }
            #[cfg(target_os = "ios")]
            {
                let _ = _api.register_ios_plugin(init_plugin_native_audio)?;
            }
            Ok(())
        })
        .build()
}
