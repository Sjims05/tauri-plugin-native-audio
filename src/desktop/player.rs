//! The desktop player: the queue, the engine, the system media controls and the output device,
//! and what the commands and media keys do with them. The commands (commands.rs) call these.

use std::collections::HashMap;
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
use super::pipeline::{NextTrack, Track};
use super::queue::{Queue, Repeat};

const STATE_EVENT: &str = "native_audio_state";
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

/// Where to pick up when a new engine is opened (output device changed or lost).
struct Resume {
    track: Option<(Track, f64)>,
    playing: bool,
}

struct Inner {
    engine: Option<Engine>,
    listeners: HashMap<String, Vec<Channel<Value>>>,
    background_started: bool,
    /// The playing entry when the background loop last looked, to notice track changes.
    last_key: Option<u64>,
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
}

/// The desktop player, managed as Tauri state. Locks: `inner` before `queue`, never the other way
/// round. The engine's feeder thread takes only `queue` (to ask what follows a track), so nothing
/// that waits for the engine may hold `queue`.
#[derive(Clone)]
pub struct DesktopAudio {
    inner: Arc<Mutex<Inner>>,
    queue: Arc<Mutex<Queue>>,
}

impl DesktopAudio {
    /// `panel`: how the system media controls find the app (None: no media controls).
    pub fn new(panel: Option<PanelConfig>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                engine: None,
                listeners: HashMap::new(),
                background_started: false,
                last_key: None,
                volume: 1.0,
                volume_curve: VolumeCurve::Quadratic,
                device: None,
                resume: None,
                skip_interval: 0.0,
                panel_config: panel,
                panel: None,
                media_controls: true,
            })),
            queue: Arc::new(Mutex::new(Queue::default())),
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
            "sleepTimerEndsAtMs": null,
            "sleepTimerEndOfTrack": false,
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

    /// The background loop (state events, track changes, the media controls, the output device),
    /// and the media controls, the first time the engine is used.
    fn start_background(&self, inner: &mut Inner) -> Result<()> {
        if inner.background_started {
            return Ok(());
        }
        inner.background_started = true;
        if let Some(config) = inner.panel_config.take() {
            let me = self.clone();
            inner.panel = Some(MediaPanel::start(config, move |event| me.on_media_event(event)));
        }
        let me = self.clone();
        thread::Builder::new()
            .name("native-audio-state".into())
            .spawn(move || me.background_loop())
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn background_loop(self) {
        let mut last_state = String::new();
        let mut last_sent = Instant::now();
        let mut last_device_check = Instant::now();
        let mut panel = PanelSync::default();
        loop {
            thread::sleep(Duration::from_millis(100));
            self.check_transition();
            let poll = last_device_check.elapsed() >= DEVICE_CHECK_EVERY;
            if poll {
                last_device_check = Instant::now();
            }
            self.check_device(poll);
            let state = self.state();
            self.sync_panel(&mut panel, &state);
            // Sent when something changes, and every 250 ms while playing (the position).
            let summary = format!(
                "{}|{}|{}|{}|{}|{}|{}|{}",
                state["status"],
                state["queueIndex"],
                state["isPlaying"],
                state["queueLength"],
                state["shuffle"],
                state["repeatMode"],
                state["volume"],
                state["error"]
            );
            let playing = state["isPlaying"].as_bool() == Some(true);
            if summary != last_state || (playing && last_sent.elapsed() >= Duration::from_millis(250)) {
                self.emit(STATE_EVENT, &state);
                last_state = summary;
                last_sent = Instant::now();
            }
        }
    }

    /// A new track started playing (on its own, or skipped to): the queue may start over (repeat all).
    fn check_transition(&self) {
        let mut inner = self.inner.lock().unwrap();
        let Some(engine) = inner.engine.as_ref() else { return };
        let key = engine.status().key;
        if key == inner.last_key {
            return;
        }
        if let (Some(from), Some(to)) = (inner.last_key, key) {
            let mut queue = self.queue.lock().unwrap();
            queue.on_transition(from, to);
            resync(engine, &queue);
        }
        inner.last_key = key;
    }

    /// Reopens the output when the stream stopped working (device gone, the system default changed:
    /// reported right away), and with `poll`, when a chosen device that was gone is back.
    fn check_device(&self, poll: bool) {
        let (chosen, lost, missing) = {
            let inner = self.inner.lock().unwrap();
            match inner.engine.as_ref() {
                Some(engine) => (inner.device.clone(), engine.device_lost(), inner.device.is_some() && engine.device_id != inner.device),
                // The last try found no device at all.
                None => (inner.device.clone(), false, inner.resume.is_some()),
            }
        };
        let mut reopen = lost;
        if !reopen && missing && poll {
            // Asking the system about devices can take a moment: not while holding the lock.
            let (devices, _) = engine::output_devices();
            reopen = match &chosen {
                Some(id) => devices.iter().any(|d| &d.id == id),
                None => !devices.is_empty(),
            };
        }
        if reopen {
            let mut inner = self.inner.lock().unwrap();
            if inner.device == chosen {
                self.reopen_engine(&mut inner);
            }
        }
    }

    // ---- the system media controls

    fn sync_panel(&self, sync: &mut PanelSync, state: &Value) {
        let inner = self.inner.lock().unwrap();
        let Some(panel) = inner.panel.as_ref() else { return };
        if !inner.media_controls {
            return;
        }
        let key = inner.engine.as_ref().and_then(|e| e.status().key);
        let playing = state["isPlaying"].as_bool() == Some(true);
        let position = state["currentTime"].as_f64().unwrap_or(0.0);
        let duration = state["duration"].as_f64().filter(|d| *d > 0.0);
        let track_changed = key != sync.key || duration != sync.duration;
        if track_changed {
            sync.key = key;
            sync.duration = duration;
            let queue = self.queue.lock().unwrap();
            match key.and_then(|key| queue.entry(key)) {
                Some(entry) => panel.set_track(TrackInfo {
                    key: entry.key,
                    path: to_path(&entry.item.src),
                    title: entry.item.title.clone(),
                    artist: entry.item.artist.clone(),
                    artwork_url: entry.item.artwork_url.clone(),
                    duration_secs: duration,
                }),
                None => {
                    panel.set_stopped();
                    sync.reported = None;
                    return;
                }
            }
        }
        // The panel moves its position on by itself while playing: only report jumps (seek) and play / pause.
        let expected = sync.reported.map(|(was_playing, at, when)| at + if was_playing { when.elapsed().as_secs_f64() } else { 0.0 });
        let jumped = expected.map_or(true, |e| (e - position).abs() > 1.0);
        if track_changed || jumped || sync.reported.map(|r| r.0) != Some(playing) {
            panel.set_playback(playing, position);
            sync.reported = Some((playing, position, Instant::now()));
        }
    }

    fn on_media_event(&self, event: MediaControlEvent) {
        let skip = self.inner.lock().unwrap().skip_interval;
        let result = match event {
            MediaControlEvent::Play => self.play(),
            MediaControlEvent::Pause | MediaControlEvent::Stop => self.pause(),
            MediaControlEvent::Toggle => {
                if self.state()["isPlaying"].as_bool() == Some(true) { self.pause() } else { self.play() }
            }
            MediaControlEvent::Next if skip > 0.0 => self.seek_by(skip),
            MediaControlEvent::Next => self.next(),
            MediaControlEvent::Previous if skip > 0.0 => self.seek_by(-skip),
            MediaControlEvent::Previous => self.previous(),
            MediaControlEvent::Seek(direction) => {
                self.seek_by(signed(direction, if skip > 0.0 { skip } else { SEEK_STEP_SECS }))
            }
            MediaControlEvent::SeekBy(direction, by) => self.seek_by(signed(direction, by.as_secs_f64())),
            MediaControlEvent::SetPosition(position) => self.seek_to(position.0.as_secs_f64()),
            MediaControlEvent::SetVolume(volume) => Ok(self.set_volume(volume)),
            _ => return,
        };
        if let Err(e) = result {
            eprintln!("native-audio: media key: {e}");
        }
    }

    // ---- actions

    pub fn initialize(&self) -> Result<Value> {
        self.with_engine(|_, _| ())?;
        Ok(self.state())
    }

    /// Replaces the queue and gets `start_index` ready at `start_position` seconds.
    pub fn load(&self, items: Vec<Item>, start_index: usize, start_position: f64) -> Result<Value> {
        self.with_engine(|engine, queue| match queue.set(items, start_index).and_then(|key| track_of(queue, key)) {
            Some(track) => engine.start(track, start_position.max(0.0)),
            None => engine.stop(),
        })?;
        Ok(self.settled())
    }

    pub fn play(&self) -> Result<Value> {
        self.with_engine(|engine, queue| {
            if queue.is_empty() {
                return;
            }
            // Finished: play the queue again from the start of the play order.
            if engine.status().ended {
                start_key(engine, queue, queue.order().first().and_then(|&i| queue.key_at(i)));
            }
            engine.play();
        })?;
        Ok(self.settled())
    }

    pub fn pause(&self) -> Result<Value> {
        self.with_engine(|engine, _| engine.pause())?;
        Ok(self.state())
    }

    pub fn seek_to(&self, position: f64) -> Result<Value> {
        self.with_engine(|engine, _| seek(engine, position))?;
        Ok(self.settled())
    }

    /// Seeks `seconds` from the current position (back when negative).
    pub fn seek_by(&self, seconds: f64) -> Result<Value> {
        self.with_engine(|engine, _| seek(engine, engine.status().position_secs + seconds))?;
        Ok(self.settled())
    }

    pub fn next(&self) -> Result<Value> {
        self.with_engine(|engine, queue| {
            if let Some(key) = engine.status().key {
                start_key(engine, queue, queue.peek_next(key, false));
            }
        })?;
        Ok(self.settled())
    }

    pub fn previous(&self) -> Result<Value> {
        self.with_engine(|engine, queue| {
            let status = engine.status();
            let Some(key) = status.key else { return };
            // Like on Android: past the first 3 seconds (or at the start of the queue), previous restarts the track.
            match queue.previous(key) {
                Some(previous) if status.position_secs <= 3.0 => start_key(engine, queue, Some(previous)),
                _ => engine.seek(0.0),
            }
        })?;
        Ok(self.settled())
    }

    pub fn skip_to(&self, index: usize) -> Result<Value> {
        self.with_engine(|engine, queue| start_key(engine, queue, queue.key_at(index)))?;
        Ok(self.settled())
    }

    pub fn set_shuffle(&self, enabled: bool) -> Result<Value> {
        self.with_engine(|engine, queue| {
            queue.set_shuffle(enabled, engine.status().key);
            resync(engine, queue);
        })?;
        Ok(self.state())
    }

    pub fn set_repeat_mode(&self, mode: &str) -> Result<Value> {
        let repeat = Repeat::parse(mode).ok_or_else(|| format!("unknown repeat mode \"{mode}\" (off, all or one)"))?;
        self.with_engine(|engine, queue| {
            queue.repeat = repeat;
            resync(engine, queue);
        })?;
        Ok(self.state())
    }

    pub fn add_to_queue(&self, items: Vec<Item>, play_next: bool) -> Result<Value> {
        self.with_engine(|engine, queue| {
            let was_empty = queue.is_empty();
            let first = queue.add(items, play_next, engine.status().key);
            if was_empty {
                // It's the queue now: ready to play, like after setQueue.
                match first.and_then(|key| track_of(queue, key)) {
                    Some(track) => engine.start(track, 0.0),
                    None => engine.stop(),
                }
            } else {
                resync(engine, queue);
            }
        })?;
        Ok(self.settled())
    }

    pub fn remove_from_queue(&self, index: usize) -> Result<Value> {
        self.with_engine(|engine, queue| {
            let key = queue.key_at(index).ok_or_else(|| format!("no queue item at index {index}"))?;
            if engine.status().key == Some(key) {
                // Removing the playing track moves on to what would have come next.
                let next = queue.peek_next(key, false).filter(|&next| next != key);
                queue.remove(index);
                match next.and_then(|next| track_of(queue, next)) {
                    Some(track) => engine.start(track, 0.0),
                    None => engine.stop(),
                }
            } else {
                queue.remove(index);
                resync(engine, queue);
            }
            Ok::<_, String>(())
        })??;
        Ok(self.settled())
    }

    pub fn move_in_queue(&self, from: usize, to: usize) -> Result<Value> {
        self.with_engine(|engine, queue| {
            if !queue.move_entry(from, to) {
                return Err(format!("can't move queue item {from} to {to}"));
            }
            // Without shuffle the play order changed with the list.
            resync(engine, queue);
            Ok(())
        })??;
        Ok(self.settled())
    }

    pub fn queue(&self) -> Value {
        let state = self.state();
        let queue = self.queue.lock().unwrap();
        let items: Vec<&Item> = queue.entries().iter().map(|entry| &entry.item).collect();
        json!({
            "items": items,
            "currentIndex": state["queueIndex"],
            "playOrder": queue.order(),
        })
    }

    pub fn dispose(&self) {
        let engine = {
            let mut inner = self.inner.lock().unwrap();
            inner.last_key = None;
            inner.resume = None;
            inner.engine.take()
        };
        // Stopping the engine waits for its feeder thread, which may need the queue: no locks held here.
        drop(engine);
        self.queue.lock().unwrap().set(Vec::new(), 0);
    }

    /// 0 to 1.
    pub fn set_volume(&self, volume: f64) -> Value {
        let volume = if volume.is_finite() { volume.clamp(0.0, 1.0) as f32 } else { 1.0 };
        {
            let mut inner = self.inner.lock().unwrap();
            inner.volume = volume;
            if let Some(engine) = inner.engine.as_ref() {
                engine.set_volume(inner.volume_curve.gain(volume));
            }
        }
        self.state()
    }

    pub fn set_volume_curve(&self, curve: &str) -> Result<()> {
        let curve = VolumeCurve::parse(curve).ok_or_else(|| format!("unknown volume curve \"{curve}\" (linear, quadratic or cubic)"))?;
        let mut inner = self.inner.lock().unwrap();
        inner.volume_curve = curve;
        if let Some(engine) = inner.engine.as_ref() {
            engine.set_volume(curve.gain(inner.volume));
        }
        Ok(())
    }

    pub fn set_skip_interval(&self, seconds: f64) -> Value {
        self.inner.lock().unwrap().skip_interval = if seconds.is_finite() { seconds.max(0.0) } else { 0.0 };
        self.state()
    }

    pub fn set_repeat_added(&self, on: bool) {
        self.queue.lock().unwrap().repeat_added = on;
    }

    /// Shows or hides the player in the system's media controls (and the media keys with it).
    pub fn set_media_controls(&self, on: bool) {
        let mut inner = self.inner.lock().unwrap();
        if inner.media_controls == on {
            return;
        }
        inner.media_controls = on;
        if let Some(panel) = inner.panel.as_ref() {
            panel.set_enabled(on);
        }
    }

    /// The output devices, the chosen one (null: the system default) and the one in use.
    pub fn output_devices(&self) -> Value {
        // Asking the system can take a moment: before taking the lock.
        let (devices, default) = engine::output_devices();
        let inner = self.inner.lock().unwrap();
        let list: Vec<Value> = devices
            .iter()
            .map(|d| json!({ "id": d.id, "name": d.name, "isDefault": Some(&d.id) == default.as_ref() }))
            .collect();
        // Following the default: the default plays.
        let active = inner.engine.as_ref().map(|e| e.device_id.clone().or_else(|| default.clone()));
        json!({
            "devices": list,
            "selected": inner.device,
            "active": active.flatten(),
        })
    }

    /// Plays on the output device `id` (from outputDevices), or follows the system default with
    /// None. A chosen device that goes away falls back to the default until it's back.
    pub fn set_output_device(&self, id: Option<String>) -> Result<Value> {
        if let Some(id) = &id {
            if !engine::output_devices().0.iter().any(|d| &d.id == id) {
                return Err(format!("no output device with id \"{id}\""));
            }
        }
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.device != id {
                inner.device = id;
                if inner.engine.is_some() {
                    self.reopen_engine(&mut inner);
                }
            }
        }
        Ok(self.output_devices())
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

/// What the background loop last told the media controls.
#[derive(Default)]
struct PanelSync {
    key: Option<u64>,
    duration: Option<f64>,
    /// Playing, position, and when.
    reported: Option<(bool, f64, Instant)>,
}

fn signed(direction: SeekDirection, seconds: f64) -> f64 {
    match direction {
        SeekDirection::Forward => seconds,
        SeekDirection::Backward => -seconds,
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
