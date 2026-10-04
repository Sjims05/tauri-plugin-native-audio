//! The desktop player: the queue, the engine, the system media controls and the output device,
//! and what the commands and media keys do with them. The commands (commands.rs) call these.
//!
//! This module has the player's parts and shared helpers; the submodules add what it does.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use souvlaki::{MediaControlEvent, SeekDirection};
use tauri::ipc::Channel;

use super::engine::{self, Engine};
use super::media_controls::{MediaPanel, PanelConfig, TrackInfo};
use super::persist::{ListConfig, ListEntry, Persist, Progress, QueuePosition};
use super::pipeline::{NextTrack, Track};
use super::queue::{Queue, Repeat, UpdateOptions, UpdateSummary};

const STATE_EVENT: &str = "native_audio_state";
const PLAYBACK_EVENT: &str = "native_audio_playback_event";
const TRACKED_LIST_EVENT: &str = "native_audio_tracked_list";
/// While playing: how often the queue position and the progress checkpoint are saved ...
const POSITION_SAVE_EVERY: Duration = Duration::from_secs(1);
/// ... and item progress (trackProgress).
const ITEM_PROGRESS_SAVE_EVERY: Duration = Duration::from_secs(15);
/// How often the background loop looks for a chosen output device that was gone.
const DEVICE_CHECK_EVERY: Duration = Duration::from_secs(2);
/// Seek step of the media panel's fast forward / rewind buttons (without a skip interval).
const SEEK_STEP_SECS: f64 = 10.0;

pub type Result<T> = std::result::Result<T, String>;

/// A queue item, as the JavaScript API sends it.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub src: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artwork_url: Option<String>,
}

/// A running sleep timer.
#[derive(Debug, Clone, Copy)]
enum SleepTimer {
    /// Pause at `ends_at` (epoch ms `ends_at_ms` for the state), fading out over the last `fade_secs`.
    At { ends_at: Instant, ends_at_ms: i64, fade_secs: f64 },
    /// Pause when the current track ends.
    EndOfTrack,
}

/// Default fade-out of a sleep timer (as on Android).
const SLEEP_FADE_OUT_SECS: f64 = 10.0;

/// Where to pick up when a new engine is opened (output device changed or lost).
struct Resume {
    track: Option<(Track, f64)>,
    playing: bool,
}

struct Inner {
    engine: Option<Engine>,
    listeners: HashMap<String, Vec<Channel<Value>>>,
    background_started: bool,
    /// The volume as set (a slider's position), 0 to 1; what's heard follows `volume_curve`.
    volume: f32,
    volume_curve: VolumeCurve,
    /// The chosen output device; None follows the system default.
    device: Option<String>,
    resume: Option<Resume>,
    /// Above 0: the media keys' previous / next seek by this many seconds (setSkipInterval).
    skip_interval: f64,
    /// Given at setup; the panel starts with the background loop.
    panel_config: Option<PanelConfig>,
    panel: Option<MediaPanel>,
    media_controls: bool,
    /// Remember how far each item played (setOptions trackProgress).
    track_progress: bool,
    /// Previous restarts the current track when it's past this; 0: previous always goes back a track.
    previous_restarts_after: f64,
    /// Next on the last track (repeat off) goes to the first track, paused (otherwise nothing happens).
    next_at_end_first: bool,
    /// The volume changed and isn't saved yet (saved by the background loop, not on every step of
    /// a slider).
    volume_unsaved: bool,
    /// The queue revision last saved (or that shouldn't be: dispose keeps the saved queue).
    saved_queue_revision: u64,
    /// resumeLastQueue ran (at the first initialize).
    resumed: bool,
    sleep: Option<SleepTimer>,
}

/// The desktop player, managed as Tauri state. Locks, in this order only: `inner`, `queue`,
/// `persist`. The engine's feeder thread takes only `queue` (to ask what follows a track), so
/// nothing that waits for the engine may hold `queue`.
#[derive(Clone)]
pub struct DesktopAudio {
    inner: Arc<Mutex<Inner>>,
    queue: Arc<Mutex<Queue>>,
    persist: Arc<Mutex<Persist>>,
}

mod actions;
mod background;
mod media_keys;
mod saved;
mod settings;
mod sleep;

pub use saved::ProgressEntry;

impl DesktopAudio {
    /// `panel`: how the system media controls find the app (None: no media controls).
    /// `data_dir`: where to save settings, the last queue, ... (None: nothing is saved).
    pub fn new(panel: Option<PanelConfig>, data_dir: Option<PathBuf>) -> Self {
        let persist = Persist::open(data_dir);
        let settings = persist.settings().clone();
        let mut queue = Queue::default();
        queue.repeat_added = settings.repeat_added_tracks.unwrap_or(false);
        Self {
            inner: Arc::new(Mutex::new(Inner {
                engine: None,
                listeners: HashMap::new(),
                background_started: false,
                volume: settings.volume.filter(|v| v.is_finite()).map_or(1.0, |v| v.clamp(0.0, 1.0)),
                volume_curve: settings.volume_curve.as_deref().and_then(VolumeCurve::parse).unwrap_or(VolumeCurve::Quadratic),
                device: settings.output_device,
                resume: None,
                skip_interval: settings.skip_interval.unwrap_or(0.0),
                panel_config: panel,
                panel: None,
                media_controls: settings.media_controls.unwrap_or(true),
                track_progress: settings.track_progress.unwrap_or(false),
                previous_restarts_after: settings.previous_restarts_after_seconds.unwrap_or(3.0),
                next_at_end_first: settings.next_at_end.as_deref() == Some("first"),
                volume_unsaved: false,
                saved_queue_revision: queue.revision(),
                resumed: false,
                sleep: None,
            })),
            queue: Arc::new(Mutex::new(queue)),
            persist: Arc::new(Mutex::new(persist)),
        }
    }

    // ---- the engine

    /// The engine and the queue, opening the output device the first time.
    fn with_engine<T>(&self, f: impl FnOnce(&Engine, &mut Queue) -> T) -> Result<T> {
        let mut inner = self.inner.lock().unwrap();
        self.ensure_engine(&mut inner)?;
        self.start_background(&mut inner)?;
        let mut queue = self.queue.lock().unwrap();
        Ok(f(inner.engine.as_ref().unwrap(), &mut queue))
    }

    fn ensure_engine(&self, inner: &mut Inner) -> Result<()> {
        if inner.engine.is_some() {
            return Ok(());
        }
        let engine = Engine::new(self.next_track(), inner.device.as_deref())?;
        engine.set_volume(inner.volume_curve.gain(inner.volume));
        engine.pause_after_current(matches!(inner.sleep, Some(SleepTimer::EndOfTrack)));
        if let Some(resume) = inner.resume.take() {
            if let Some((track, position)) = resume.track {
                engine.start(track, position);
            }
            if resume.playing {
                engine.play();
            }
        }
        inner.engine = Some(engine);
        Ok(())
    }

    /// Opens a new engine (on the wanted output device), playing on from where the old one was.
    fn reopen_engine(&self, inner: &mut Inner) {
        if let Some(old) = inner.engine.take() {
            let status = old.status();
            let track = if status.ended { None } else { old.current_track().map(|t| (t, status.position_secs)) };
            inner.resume = Some(Resume { track, playing: status.playing });
            // Waits for the feeder thread, which only ever needs `queue`: not held here.
            drop(old);
        }
        // Without a device, the next device check tries again.
        if let Err(e) = self.ensure_engine(inner) {
            eprintln!("native-audio: can't open the audio output: {e}");
        }
    }

    /// What the engine plays after each track: the queue's next entry (repeat one: the same again).
    fn next_track(&self) -> NextTrack {
        let queue = self.queue.clone();
        Arc::new(move |key| {
            let queue = queue.lock().unwrap();
            queue.peek_next(key, true).and_then(|next| track_of(&queue, next))
        })
    }

    /// The state once the engine has opened what a command asked for (on its own thread), so it
    /// shows the new track. Waits without holding the locks: opening the track may need the queue.
    fn settled(&self) -> Value {
        let waiter = self.inner.lock().unwrap().engine.as_ref().map(Engine::waiter);
        if let Some(waiter) = waiter {
            waiter.wait(Duration::from_millis(500));
        }
        self.state()
    }

    // ---- state and the background loop

    /// The state in the shape of the JavaScript API's NativeAudioState.
    pub fn state(&self) -> Value {
        let inner = self.inner.lock().unwrap();
        let queue = self.queue.lock().unwrap();
        let mut state = json!({
            "status": "idle",
            "currentTime": 0.0,
            "duration": 0.0,
            "isPlaying": false,
            "buffering": false,
            "rate": 1.0,
            "queueIndex": -1,
            "queueLength": queue.len(),
            "shuffle": queue.shuffle,
            "repeatMode": queue.repeat.name(),
            "sleepTimerEndsAtMs": match inner.sleep {
                Some(SleepTimer::At { ends_at_ms, .. }) => json!(ends_at_ms),
                _ => Value::Null,
            },
            "sleepTimerEndOfTrack": matches!(inner.sleep, Some(SleepTimer::EndOfTrack)),
            "volume": inner.volume,
        });
        let Some(engine) = inner.engine.as_ref() else { return state };
        let status = engine.status();
        let index = status.key.and_then(|key| queue.index_of(key));
        let loaded = !queue.is_empty();
        // A track that can't be played is skipped; "error" when that left nothing to play.
        state["status"] = json!(if status.ended && status.error.is_some() {
            "error"
        } else if status.ended && loaded {
            "ended"
        } else if status.loading && loaded {
            "loading"
        } else if status.playing && index.is_some() {
            "playing"
        } else {
            "idle"
        });
        state["currentTime"] = json!(status.position_secs);
        state["duration"] = json!(status.duration_secs.unwrap_or(0.0));
        state["isPlaying"] = json!(status.playing && !status.ended && index.is_some());
        state["queueIndex"] = json!(index.map_or(-1, |i| i as i64));
        if let Some(id) = index.and_then(|i| queue.entries()[i].item.id) {
            state["currentId"] = json!(id);
        }
        if let Some(error) = status.error {
            state["error"] = json!(error);
        }
        state
    }

    fn emit(&self, event: &str, payload: &Value) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(channels) = inner.listeners.get_mut(event) {
            // A channel whose page is gone fails to send: drop it.
            channels.retain(|channel| channel.send(payload.clone()).is_ok());
        }
    }

    pub fn add_listener(&self, event: String, channel: Channel<Value>) {
        self.inner.lock().unwrap().listeners.entry(event).or_default().push(channel);
    }

    pub fn remove_listener(&self, event: &str, channel_id: u32) {
        if let Some(channels) = self.inner.lock().unwrap().listeners.get_mut(event) {
            channels.retain(|channel| channel.id() != channel_id);
        }
    }
}

/// How a volume setting becomes loudness. Hearing is roughly logarithmic, so with `Linear` half
/// the setting only sounds a little quieter; `Quadratic` (the default) makes 50% about half as loud
/// (-12 dB), `Cubic` leans further toward fine control at low volumes (50% = -18 dB).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum VolumeCurve {
    Linear,
    Quadratic,
    Cubic,
}

impl VolumeCurve {
    fn name(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Quadratic => "quadratic",
            Self::Cubic => "cubic",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        match name {
            "linear" => Some(Self::Linear),
            "quadratic" => Some(Self::Quadratic),
            "cubic" => Some(Self::Cubic),
            _ => None,
        }
    }

    /// The amplitude factor for a volume setting.
    fn gain(self, volume: f32) -> f32 {
        match self {
            Self::Linear => volume,
            Self::Quadratic => volume * volume,
            Self::Cubic => volume * volume * volume,
        }
    }
}

/// resumeLastQueue as "off", "paused" or "play".
fn resume_mode(value: &Value) -> Option<&'static str> {
    match value {
        Value::Bool(true) => Some("paused"),
        Value::Bool(false) => Some("off"),
        Value::String(s) => ["off", "paused", "play"].into_iter().find(|m| m == s),
        _ => None,
    }
}

/// Seeks in the current track; past the end goes to the end, from where it moves on as usual.
fn seek(engine: &Engine, position: f64) {
    let last = engine.status().duration_secs.filter(|d| *d > 0.0).map_or(f64::MAX, |d| (d - 0.01).max(0.0));
    engine.seek(position.clamp(0.0, last));
}

/// The queue entry `key` as a track for the engine.
fn track_of(queue: &Queue, key: u64) -> Option<Track> {
    queue.entry(key).map(|entry| Track { key, path: to_path(&entry.item.src) })
}

/// Starts the queue entry `key` from the beginning.
fn start_key(engine: &Engine, queue: &Queue, key: Option<u64>) {
    if let Some(track) = key.and_then(|key| track_of(queue, key)) {
        engine.start(track, 0.0);
    }
}

/// After the queue changed: if the engine already prepared a different track to follow the playing
/// one (it does so a couple of seconds before the end), plays on from the same spot so the right
/// one follows. Otherwise there's nothing to do: the engine asks the queue when it gets there.
fn resync(engine: &Engine, queue: &Queue) {
    let status = engine.status();
    let Some(playing) = status.key else { return };
    if status.ended {
        return;
    }
    let prepared = match (status.upcoming, status.end_prepared) {
        (Some(upcoming), _) => Some(upcoming),
        (None, true) => None,
        (None, false) => return,
    };
    if queue.peek_next(playing, true) != prepared {
        if let Some(track) = track_of(queue, playing) {
            engine.start(track, status.position_secs);
        }
    }
}

/// A local path from what the app passed: a plain path, or a file:// URL.
pub fn to_path(src: &str) -> PathBuf {
    let Some(rest) = src.strip_prefix("file://") else { return PathBuf::from(src) };
    // file:///C:/Music/x.mp3 -> C:/Music/x.mp3; also undo %-escapes like %20.
    let rest = if rest.len() > 2 && rest.starts_with('/') && rest.as_bytes()[2] == b':' { &rest[1..] } else { rest };
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&rest[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    PathBuf::from(String::from_utf8_lossy(&out).into_owned())
}
