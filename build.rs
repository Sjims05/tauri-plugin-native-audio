const COMMANDS: &[&str] = &[
    "initialize",
    "register_listener",
    "remove_listener",
    "set_source",
    "set_queue",
    "next",
    "previous",
    "skip_to",
    "add_to_queue",
    "remove_from_queue",
    "move_in_queue",
    "get_queue",
    "restore_last_queue",
    "play",
    "pause",
    "seek_to",
    "set_rate",
    "set_skip_interval",
    "set_shuffle",
    "set_repeat_mode",
    "set_library",
    "set_options",
    "set_controls",
    "set_control_active",
    "get_control_presses",
    "acknowledge_control_presses",
    "get_playback_events",
    "acknowledge_playback_events",
    "set_tracked_lists",
    "get_tracked_list",
    "set_tracked_list",
    "get_tracked_list_changes",
    "acknowledge_tracked_list_changes",
    "set_sleep_timer",
    "cancel_sleep_timer",
    "get_item_progress",
    "set_item_progress",
    "get_state",
    "get_progress_checkpoint",
    "clear_progress_checkpoint",
    "dispose",
];

/// Marks the block this plugin owns in the app's AndroidManifest.xml.
const CAR_MANIFEST_BLOCK: &str = "NATIVE AUDIO CAR SUPPORT";

/// Added to the app manifest when `plugins.native-audio.carSupport` is true: lets Android Auto
/// find the app as a media app and connect to the audio service to browse and play.
const CAR_MANIFEST_ENTRIES: &str = r#"<service
    xmlns:tools="http://schemas.android.com/tools"
    android:name="app.tauri.nativeaudio.NativeAudioService"
    android:exported="true"
    tools:replace="android:exported">
    <intent-filter>
        <action android:name="androidx.media3.session.MediaLibraryService" />
        <action android:name="android.media.browse.MediaBrowserService" />
    </intent-filter>
</service>
<meta-data
    android:name="com.google.android.gms.car.application"
    android:resource="@xml/automotive_app_desc" />"#;

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Config {
    #[serde(default)]
    car_support: bool,
}

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .ios_path("ios")
        .build();

    let config = tauri_plugin::plugin_config::<Config>("native-audio").unwrap_or_default();
    update_car_manifest_entries(config.car_support);
}

fn update_car_manifest_entries(car_support: bool) {
    println!("cargo:rerun-if-env-changed=TAURI_ANDROID_PROJECT_PATH");
    if car_support {
        tauri_plugin::mobile::update_android_manifest(
            CAR_MANIFEST_BLOCK,
            "application",
            CAR_MANIFEST_ENTRIES.to_string(),
        )
        .expect("failed to add car support entries to AndroidManifest.xml");
    } else {
        // update_android_manifest always leaves its marker comments behind, so remove the whole
        // block ourselves and leave the manifest exactly as it was before car support was enabled.
        remove_car_manifest_entries();
    }
}

fn remove_car_manifest_entries() {
    let Some(project_path) = std::env::var_os("TAURI_ANDROID_PROJECT_PATH") else {
        return;
    };
    let manifest_path = std::path::Path::new(&project_path).join("app/src/main/AndroidManifest.xml");
    let Ok(manifest) = std::fs::read_to_string(&manifest_path) else {
        return;
    };
    let marker = format!("<!-- {CAR_MANIFEST_BLOCK}. AUTO-GENERATED. DO NOT REMOVE. -->");
    if !manifest.contains(&marker) {
        return;
    }

    let mut inside_block = false;
    let kept: Vec<&str> = manifest
        .split('\n')
        .filter(|line| {
            if line.contains(&marker) {
                inside_block = !inside_block;
                return false;
            }
            !inside_block
        })
        .collect();
    std::fs::write(&manifest_path, kept.join("\n"))
        .expect("failed to remove car support entries from AndroidManifest.xml");
}
