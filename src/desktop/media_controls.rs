//! The system's media controls (souvlaki): the Windows media panel and media keys, MPRIS on Linux,
//! Now Playing on macOS. Runs on its own thread; the player sends it what's playing, and button
//! presses come back through the handler given to `start`.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig};

use super::player::to_path;
use super::tags;

/// How the controls find the app.
pub struct PanelConfig {
    /// Shown as the player's name (Linux).
    pub display_name: String,
    /// The player's D-Bus name (Linux): org.mpris.MediaPlayer2.<dbus_name>.
    pub dbus_name: String,
    /// Windows: the app window's raw HWND, once there is one (the controls belong to a window).
    pub window: Arc<dyn Fn() -> Option<isize> + Send + Sync>,
}

/// What's playing.
#[derive(Clone)]
pub struct TrackInfo {
    pub key: u64,
    pub path: PathBuf,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub artwork_url: Option<String>,
    pub duration_secs: Option<f64>,
}

enum Update {
    Enabled(bool),
    Track(TrackInfo),
    Playback { playing: bool, position_secs: f64 },
    Stopped,
}

pub struct MediaPanel {
    updates: Sender<Update>,
}

impl MediaPanel {
    pub fn start(config: PanelConfig, on_event: impl Fn(MediaControlEvent) + Send + 'static) -> Self {
        let (updates, rx) = mpsc::channel();
        let _ = thread::Builder::new()
            .name("native-audio-media-controls".into())
            .spawn(move || run(config, rx, on_event));
        Self { updates }
    }

    pub fn set_track(&self, track: TrackInfo) {
        let _ = self.updates.send(Update::Track(track));
    }

    pub fn set_playback(&self, playing: bool, position_secs: f64) {
        let _ = self.updates.send(Update::Playback { playing, position_secs });
    }

    pub fn set_stopped(&self) {
        let _ = self.updates.send(Update::Stopped);
    }

    /// Off: the player leaves the system's media controls (and the media keys stop reaching it).
    pub fn set_enabled(&self, on: bool) {
        let _ = self.updates.send(Update::Enabled(on));
    }
}

fn run(config: PanelConfig, updates: Receiver<Update>, on_event: impl Fn(MediaControlEvent) + Send + 'static) {
    // Button presses go to their own thread, so handling one never waits on this thread.
    let (events_tx, events_rx) = mpsc::channel::<MediaControlEvent>();
    let _ = thread::Builder::new().name("native-audio-media-keys".into()).spawn(move || {
        for event in events_rx {
            on_event(event);
        }
    });

    let mut controls: Option<MediaControls> = None;
    let mut track: Option<TrackInfo> = None;
    let mut playback = MediaPlayback::Stopped;
    let mut covers = CoverFiles::default();
    let mut reported_error = false;
    let mut enabled = true;
    loop {
        // Without a window yet (Windows), check again now and then.
        let update = match updates.recv_timeout(Duration::from_millis(500)) {
            Ok(update) => Some(update),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        let mut track_changed = false;
        match update {
            Some(Update::Enabled(on)) => {
                enabled = on;
                if !on {
                    // Dropping them takes the player out of the system's controls.
                    controls = None;
                }
            }
            Some(Update::Track(info)) => {
                track = Some(info);
                track_changed = true;
            }
            Some(Update::Playback { playing, position_secs }) => {
                let progress = Some(MediaPosition(Duration::from_secs_f64(position_secs.max(0.0))));
                playback = if playing { MediaPlayback::Playing { progress } } else { MediaPlayback::Paused { progress } };
            }
            Some(Update::Stopped) => {
                track = None;
                playback = MediaPlayback::Stopped;
            }
            None if controls.is_some() => continue,
            None => {}
        }

        if !enabled {
            continue;
        }
        if controls.is_none() {
            // Nothing to show yet: no empty entry in the system's media panel.
            if track.is_none() {
                continue;
            }
            match create(&config, events_tx.clone()) {
                Ok(Some(created)) => {
                    controls = Some(created);
                    track_changed = true;
                }
                Ok(None) => continue,
                Err(e) => {
                    if !reported_error {
                        eprintln!("native-audio: system media controls unavailable: {e}");
                        reported_error = true;
                    }
                    continue;
                }
            }
        }
        let Some(controls) = controls.as_mut() else { continue };
        if track_changed {
            let result = match &track {
                Some(info) => set_metadata(controls, info, &mut covers),
                None => controls.set_metadata(MediaMetadata::default()).map_err(|e| format!("{e:?}")),
            };
            if let Err(e) = result {
                eprintln!("native-audio: media controls: {e}");
            }
        }
        if let Err(e) = controls.set_playback(playback.clone()) {
            eprintln!("native-audio: media controls: {e:?}");
        }
    }
}

/// None while there's no window to attach to yet (Windows).
fn create(config: &PanelConfig, events: Sender<MediaControlEvent>) -> Result<Option<MediaControls>, String> {
    let hwnd = (config.window)();
    if cfg!(target_os = "windows") && hwnd.is_none() {
        return Ok(None);
    }
    let platform = PlatformConfig {
        display_name: &config.display_name,
        dbus_name: &config.dbus_name,
        hwnd: hwnd.map(|h| h as *mut std::ffi::c_void),
    };
    let mut controls = MediaControls::new(platform).map_err(|e| format!("{e:?}"))?;
    controls
        .attach(move |event| {
            let _ = events.send(event);
        })
        .map_err(|e| format!("{e:?}"))?;
    Ok(Some(controls))
}

fn set_metadata(controls: &mut MediaControls, info: &TrackInfo, covers: &mut CoverFiles) -> Result<(), String> {
    // What the app passed first, then the file's own tags.
    let tags = tags::read(&info.path);
    let file_name = info.path.file_stem().map(|s| s.to_string_lossy().into_owned());
    let title = info.title.clone().or(tags.title).or(file_name).unwrap_or_default();
    let artist = info.artist.clone().or(tags.artist).unwrap_or_default();
    let album = tags.album.unwrap_or_default();
    let cover = match &info.artwork_url {
        Some(url) if url.starts_with("http://") || url.starts_with("https://") => Some(url.clone()),
        Some(local) => Some(file_url(&to_path(local))),
        None => tags.cover.and_then(|(data, media_type)| covers.write(info.key, &data, &media_type)).map(|p| file_url(&p)),
    };
    controls
        .set_metadata(MediaMetadata {
            // Empty rather than missing, so the previous track's text doesn't stay.
            title: Some(&title),
            artist: Some(&artist),
            album: Some(&album),
            cover_url: cover.as_deref(),
            duration: info.duration_secs.map(Duration::from_secs_f64),
        })
        .map_err(|e| format!("{e:?}"))
}

/// A file:// URL in the form each platform's controls read: Windows takes the plain path after it.
fn file_url(path: &Path) -> String {
    if cfg!(target_os = "windows") {
        format!("file://{}", path.display().to_string().replace('/', "\\"))
    } else {
        format!("file://{}", path.display())
    }
}

/// Embedded covers written out for the controls (they take a file, not bytes). One at a time.
#[derive(Default)]
struct CoverFiles {
    current: Option<PathBuf>,
}

impl CoverFiles {
    fn write(&mut self, key: u64, data: &[u8], media_type: &str) -> Option<PathBuf> {
        let ext = if media_type.contains("png") { "png" } else { "jpg" };
        let dir = std::env::temp_dir().join("tauri-plugin-native-audio");
        std::fs::create_dir_all(&dir).ok()?;
        // A new name per track: the controls may cache by path.
        let path = dir.join(format!("cover-{}-{key}.{ext}", std::process::id()));
        std::fs::write(&path, data).ok()?;
        if let Some(old) = self.current.replace(path.clone()) {
            if old != path {
                let _ = std::fs::remove_file(old);
            }
        }
        Some(path)
    }
}

impl Drop for CoverFiles {
    fn drop(&mut self) {
        if let Some(path) = self.current.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}
