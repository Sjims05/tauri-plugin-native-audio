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
    "set_volume",
    "get_output_devices",
    "set_output_device",
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

/// The drawable an app can add (res/drawable*/native_audio_car_icon.png or .xml) to give Android Auto a
/// one-color icon for its player and media card, tinted by Auto. Without it, Auto tints the app icon.
const CAR_ICON_NAME: &str = "native_audio_car_icon";

/// Added with the car entries when the app has the CAR_ICON_NAME drawable.
const CAR_ICON_ENTRY: &str = r#"
<meta-data
    android:name="androidx.car.app.TintableAttributionIcon"
    android:resource="@drawable/native_audio_car_icon" />"#;

#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Config {
    #[serde(default)]
    car_support: bool,
}

fn main() {
    // The platforms the app chose (Cargo features, see Cargo.toml).
    let mobile = std::env::var_os("CARGO_FEATURE_MOBILE").is_some();
    let desktop = std::env::var_os("CARGO_FEATURE_DESKTOP").is_some();
    if !mobile && !desktop {
        panic!(
            "tauri-plugin-native-audio: no platform enabled. Enable the `mobile` feature, the `desktop` feature, or both (the default)."
        );
    }

    let mut builder = tauri_plugin::Builder::new(COMMANDS);
    if mobile {
        builder = builder.android_path("android").ios_path("ios");
    }
    builder.build();

    let config = tauri_plugin::plugin_config::<Config>("native-audio").unwrap_or_default();
    if config.car_support && !mobile {
        panic!(
            "tauri-plugin-native-audio: `carSupport` is on in tauri.conf.json, but the `mobile` feature is off. Android Auto needs the Android player: enable `mobile`, or turn carSupport off."
        );
    }
    update_car_manifest_entries(config.car_support && mobile);
}

fn update_car_manifest_entries(car_support: bool) {
    println!("cargo:rerun-if-env-changed=TAURI_ANDROID_PROJECT_PATH");
    if car_support {
        let mut entries = CAR_MANIFEST_ENTRIES.to_string();
        if app_has_car_icon() {
            entries.push_str(CAR_ICON_ENTRY);
        }
        tauri_plugin::mobile::update_android_manifest(CAR_MANIFEST_BLOCK, "application", entries)
        .expect("failed to add car support entries to AndroidManifest.xml");
    } else {
        // update_android_manifest always leaves its marker comments behind, so remove the whole
        // block ourselves and leave the manifest exactly as it was before car support was enabled.
        remove_car_manifest_entries();
    }
}

/// Whether the app's Android project has the `native_audio_car_icon` drawable (in any density folder).
fn app_has_car_icon() -> bool {
    let Some(project_path) = std::env::var_os("TAURI_ANDROID_PROJECT_PATH") else {
        return false;
    };
    let res = std::path::Path::new(&project_path).join("app/src/main/res");
    // Check again when the app's resources change (an icon added or removed).
    println!("cargo:rerun-if-changed={}", res.display());
    let Ok(folders) = std::fs::read_dir(&res) else {
        return false;
    };
    folders.flatten().any(|folder| {
        folder.file_name().to_string_lossy().starts_with("drawable")
            && std::fs::read_dir(folder.path()).is_ok_and(|files| {
                files.flatten().any(|file| {
                    let name = file.file_name();
                    let name = name.to_string_lossy();
                    name.strip_prefix(CAR_ICON_NAME).is_some_and(|ext| ext == ".png" || ext == ".xml" || ext == ".webp")
                })
            })
    })
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
