//! The desktop player: the queue, the engine, the system media controls and the output device,
//! and what the commands and media keys do with them. The commands (commands.rs) call these.

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

    /// The background loop (state events, track changes, the media controls, the output device),
    /// and the media controls, the first time the engine is used.
    fn start_background(&self, inner: &mut Inner) -> Result<()> {
        if inner.background_started {
            return Ok(());
        }
        inner.background_started = true;
        if let Some(config) = inner.panel_config.take() {
            let me = self.clone();
            let panel = MediaPanel::start(config, move |event| me.on_media_event(event));
            if !inner.media_controls {
                panel.set_enabled(false);
            }
            inner.panel = Some(panel);
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
        let mut tracking = Tracking::default();
        loop {
            thread::sleep(Duration::from_millis(100));
            for (event, payload) in self.track(&mut tracking) {
                self.emit(event, &payload);
            }
            let poll = last_device_check.elapsed() >= DEVICE_CHECK_EVERY;
            if poll {
                last_device_check = Instant::now();
            }
            self.check_device(poll);
            let state = self.state();
            self.sync_panel(&mut panel, &state);
            // Sent when something changes, and every 250 ms while playing (the position).
            self.check_sleep_timer();
            let summary = format!(
                "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
                state["status"],
                state["queueIndex"],
                state["isPlaying"],
                state["queueLength"],
                state["shuffle"],
                state["repeatMode"],
                state["volume"],
                state["sleepTimerEndsAtMs"],
                state["sleepTimerEndOfTrack"],
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

    /// Follows playback for what's saved: playback events (start / complete / skip), tracked lists,
    /// item progress, the progress checkpoint and the queue (with where it is). A new play that
    /// nobody asked for is the queue moving on by itself: the previous track completed. Also lets
    /// the queue start over (repeat all) when a new track starts. Returns the events to send.
    fn track(&self, t: &mut Tracking) -> Vec<(&'static str, Value)> {
        let mut out = Vec::new();
        let mut inner = self.inner.lock().unwrap();
        let mut queue = self.queue.lock().unwrap();
        let mut persist = self.persist.lock().unwrap();
        if inner.volume_unsaved {
            inner.volume_unsaved = false;
            let volume = inner.volume;
            persist.update_settings(|s| s.volume = Some(volume));
        }
        if queue.revision() != inner.saved_queue_revision {
            inner.saved_queue_revision = queue.revision();
            persist.save_queue(queue.snapshot().as_ref());
            t.saved_position = None;
        }
        let Some(engine) = inner.engine.as_ref() else { return out };
        let status = engine.status();
        let track_progress = inner.track_progress;

        if status.play != t.play {
            let asked = status.requests != t.requests;
            // A seek (or playing on after a queue edit) starts the same track again: still the same play.
            let same_play = asked && status.key.is_some() && status.key == t.key;
            if !same_play {
                if t.play.is_some() && !t.finished {
                    // Nobody asked: it played to the end. Asked: it was left (if it had started).
                    if !asked {
                        self.finish_play(t, &queue, &mut persist, true, track_progress, &mut out);
                    } else if t.started {
                        self.finish_play(t, &queue, &mut persist, false, track_progress, &mut out);
                    }
                }
                if let (Some(from), Some(to)) = (t.key, status.key) {
                    if from != to {
                        queue.on_transition(from, to);
                        resync(engine, &queue);
                    }
                }
                let item_id = status.key.and_then(|k| queue.entry(k)).and_then(|e| e.item.id);
                *t = Tracking { key: status.key, item_id, saved_position: t.saved_position, ..Tracking::default() };
            }
            t.play = status.play;
        }
        t.requests = status.requests;
        if t.play.is_none() {
            return out;
        }
        t.position = status.position_secs;
        t.duration = status.duration_secs.unwrap_or(t.duration);
        let playing = status.playing && !status.ended && !status.loading;

        // The last track of the queue played to the end.
        if status.ended && !t.finished {
            self.finish_play(t, &queue, &mut persist, true, track_progress, &mut out);
        }
        if !t.finished {
            if playing && !t.started {
                t.started = true;
                if let Some(id) = t.item_id {
                    out.push((PLAYBACK_EVENT, persist.record_event("start", id, (t.position * 1000.0) as i64)));
                }
            }
            if t.started {
                count_lists(t, &queue, &mut persist, false, &mut out);
            }
            if track_progress && t.started {
                let paused_now = t.playing && !playing;
                let due = playing && t.progress_saved.is_none_or(|at| at.elapsed() >= ITEM_PROGRESS_SAVE_EVERY);
                if paused_now || due {
                    t.progress_saved = Some(Instant::now());
                    if let Some(id) = t.item_id {
                        persist.record_progress(id, (t.position * 1000.0) as i64, (t.duration * 1000.0) as i64, false);
                    }
                }
            }
        }

        // Where the queue is, and the progress checkpoint: every second while playing, and on pause.
        let changed_playing = t.playing != playing;
        let due = t.position_saved.is_none_or(|at| at.elapsed() >= POSITION_SAVE_EVERY);
        if changed_playing || (playing && due) {
            t.position_saved = Some(Instant::now());
            if let Some(index) = t.key.and_then(|k| queue.index_of(k)) {
                let position = QueuePosition { index, position_secs: t.position };
                if t.saved_position != Some((index, t.position)) {
                    t.saved_position = Some((index, t.position));
                    persist.save_position(position);
                }
            }
            if let Some(id) = t.item_id.filter(|id| *id > 0) {
                if t.position > 0.25 {
                    let state = if status.ended { "ended" } else if playing { "playing" } else { "idle" };
                    persist.save_checkpoint(id, t.position, state);
                }
            }
        }
        t.playing = playing;
        out
    }

    /// The current play is over: `completed` (played to the end) or skipped.
    fn finish_play(&self, t: &mut Tracking, queue: &Queue, persist: &mut Persist, completed: bool, track_progress: bool, out: &mut Vec<(&'static str, Value)>) {
        t.finished = true;
        let position = if completed { t.duration.max(t.position) } else { t.position };
        let position_ms = (position * 1000.0) as i64;
        if let Some(id) = t.item_id {
            out.push((PLAYBACK_EVENT, persist.record_event(if completed { "complete" } else { "skip" }, id, position_ms)));
            if track_progress {
                persist.record_progress(id, position_ms, (t.duration * 1000.0) as i64, completed);
            }
        }
        if completed {
            count_lists(t, queue, persist, true, out);
        }
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
        let jumped = expected.is_none_or(|e| (e - position).abs() > 1.0);
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

    /// The first call also resumes the last queue (resumeLastQueue): the app calls this once its
    /// UI is ready, so a resumed queue doesn't start before the window shows anything.
    pub fn initialize(&self) -> Result<Value> {
        self.with_engine(|_, _| ())?;
        let first = !std::mem::replace(&mut self.inner.lock().unwrap().resumed, true);
        if first {
            self.resume_at_start();
            return Ok(self.settled());
        }
        Ok(self.state())
    }

    /// Replaces the queue and gets `start_index` ready at `start_position` seconds. `source_id`:
    /// the playable folder it plays (for `folder` tracked lists).
    pub fn load(&self, items: Vec<Item>, start_index: usize, start_position: f64, source_id: Option<String>) -> Result<Value> {
        self.with_engine(|engine, queue| {
            let key = queue.set(items, start_index);
            queue.source_id = source_id;
            match key.and_then(|key| track_of(queue, key)) {
                Some(track) => engine.start(track, start_position.max(0.0)),
                None => engine.stop(),
            }
        })?;
        Ok(self.settled())
    }

    /// Loads the queue saved last time (paused, where it was), unless one is loaded already: then
    /// it's left alone. None when nothing was saved.
    pub fn restore_last_queue(&self) -> Result<Option<Value>> {
        let saved = self.persist.lock().unwrap().load_queue();
        let restored = self.with_engine(|engine, queue| {
            if !queue.is_empty() {
                return true;
            }
            let Some((saved, position)) = saved else { return false };
            match queue.restore(saved, position.index).and_then(|key| track_of(queue, key)) {
                Some(track) => {
                    engine.pause();
                    engine.start(track, position.position_secs);
                    true
                }
                None => false,
            }
        })?;
        Ok(restored.then(|| self.settled()))
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
        let first_at_end = self.inner.lock().unwrap().next_at_end_first;
        self.with_engine(|engine, queue| {
            let Some(key) = engine.status().key else { return };
            match queue.peek_next(key, false) {
                Some(next) => start_key(engine, queue, Some(next)),
                // The end of the queue: nothing, or (nextAtEnd "first") the first track, paused.
                None if first_at_end => {
                    engine.pause();
                    start_key(engine, queue, queue.order().first().and_then(|&i| queue.key_at(i)));
                }
                None => {}
            }
        })?;
        Ok(self.settled())
    }

    pub fn previous(&self) -> Result<Value> {
        let restart_after = self.inner.lock().unwrap().previous_restarts_after;
        self.with_engine(|engine, queue| {
            let status = engine.status();
            let Some(key) = status.key else { return };
            // Like on Android: past previousRestartsAfterSeconds (or at the start of the queue), previous restarts the track.
            match queue.previous(key) {
                Some(previous) if restart_after <= 0.0 || status.position_secs <= restart_after => {
                    start_key(engine, queue, Some(previous))
                }
                _ => engine.seek(0.0),
            }
        })?;
        Ok(self.settled())
    }

    /// previousRestartsAfterSeconds and nextAtEnd.
    pub fn set_skip_rules(&self, previous_restarts_after: Option<f64>, next_at_end: Option<&str>) -> Result<()> {
        if let Some(seconds) = previous_restarts_after {
            if !seconds.is_finite() || seconds < 0.0 {
                return Err("previousRestartsAfterSeconds must be >= 0".into());
            }
        }
        if let Some(value) = next_at_end {
            if value != "nothing" && value != "first" {
                return Err("nextAtEnd must be nothing or first".into());
            }
        }
        let mut inner = self.inner.lock().unwrap();
        if let Some(seconds) = previous_restarts_after {
            inner.previous_restarts_after = seconds;
        }
        if let Some(value) = next_at_end {
            inner.next_at_end_first = value == "first";
        }
        self.persist.lock().unwrap().update_settings(|s| {
            if previous_restarts_after.is_some() {
                s.previous_restarts_after_seconds = previous_restarts_after;
            }
            if let Some(value) = next_at_end {
                s.next_at_end = Some(value.to_string());
            }
        });
        Ok(())
    }

    /// resumeLastQueue, at the first initialize: "paused" (the default) loads the last queue, so the
    /// system media controls and media keys can continue it; "play" also plays it.
    pub fn resume_at_start(&self) {
        let mode = self.persist.lock().unwrap().settings().resume_last_queue.as_ref().and_then(resume_mode).unwrap_or("paused");
        if mode == "off" {
            return;
        }
        let result = self.restore_last_queue().and_then(|restored| match restored {
            Some(_) if mode == "play" => self.play().map(|_| ()),
            _ => Ok(()),
        });
        if let Err(e) = result {
            eprintln!("native-audio: can't resume the last queue: {e}");
        }
    }

    /// `mode`: "off", "paused" or "play", or true / false (paused / off).
    pub fn set_resume_last_queue(&self, mode: Value) -> Result<()> {
        let mode = resume_mode(&mode).ok_or("resumeLastQueue must be off, paused or play")?;
        self.persist.lock().unwrap().update_settings(|s| s.resume_last_queue = Some(json!(mode)));
        Ok(())
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
            queue.set_repeat(repeat);
            resync(engine, queue);
        })?;
        Ok(self.state())
    }

    /// The playlist the queue plays changed (see Queue::update): what stays keeps playing, new
    /// items are added, missing ones removed. `skip_removed_current`: when the playing song left the
    /// playlist, move on now instead of letting it finish. With an empty queue, `items` becomes the queue.
    pub fn update_queue(&self, items: Vec<Item>, source_id: Option<String>, options: UpdateOptions, skip_removed_current: bool) -> Result<Value> {
        let summary = self.with_engine(|engine, queue| {
            let mut summary = UpdateSummary::default();
            if queue.is_empty() {
                // Ready to play, like after setQueue.
                summary.added = items.len();
                match queue.set(items, 0).and_then(|key| track_of(queue, key)) {
                    Some(track) => engine.start(track, 0.0),
                    None => engine.stop(),
                }
            } else {
                let current = engine.status().key;
                summary = queue.update(items, current, options);
                match current.filter(|&c| skip_removed_current && queue.is_leaving(c)) {
                    // Moving on drops it (on_transition); with nothing after it, it goes now.
                    Some(current) => match queue.peek_next(current, false).filter(|&next| next != current) {
                        Some(next) => start_key(engine, queue, Some(next)),
                        None => {
                            if let Some(index) = queue.index_of(current) {
                                queue.remove(index);
                            }
                            engine.stop();
                        }
                    },
                    None => resync(engine, queue),
                }
            }
            if source_id.is_some() {
                queue.source_id = source_id;
            }
            summary
        })?;
        Ok(json!({ "state": self.settled(), "added": summary.added, "removed": summary.removed }))
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
            inner.resume = None;
            inner.engine.take()
        };
        // Stopping the engine waits for its feeder thread, which may need the queue: no locks held here.
        drop(engine);
        let mut inner = self.inner.lock().unwrap();
        let mut queue = self.queue.lock().unwrap();
        queue.set(Vec::new(), 0);
        // The saved queue stays, for restoreLastQueue next time.
        inner.saved_queue_revision = queue.revision();
    }

    /// 0 to 1.
    pub fn set_volume(&self, volume: f64) -> Value {
        let volume = if volume.is_finite() { volume.clamp(0.0, 1.0) as f32 } else { 1.0 };
        {
            let mut inner = self.inner.lock().unwrap();
            inner.volume = volume;
            let fading = matches!(inner.sleep, Some(SleepTimer::At { ends_at, fade_secs, .. })
                if ends_at.saturating_duration_since(Instant::now()).as_secs_f64() < fade_secs);
            if let Some(engine) = inner.engine.as_ref().filter(|_| !fading) {
                engine.set_volume(inner.volume_curve.gain(volume));
            }
            if inner.background_started {
                inner.volume_unsaved = true;
            } else {
                self.persist.lock().unwrap().update_settings(|s| s.volume = Some(volume));
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
        self.persist.lock().unwrap().update_settings(|s| s.volume_curve = Some(curve.name().into()));
        Ok(())
    }

    pub fn set_skip_interval(&self, seconds: f64) -> Value {
        let seconds = if seconds.is_finite() { seconds.max(0.0) } else { 0.0 };
        self.inner.lock().unwrap().skip_interval = seconds;
        self.persist.lock().unwrap().update_settings(|s| s.skip_interval = Some(seconds));
        self.state()
    }

    pub fn set_repeat_added(&self, on: bool) {
        self.queue.lock().unwrap().repeat_added = on;
        self.persist.lock().unwrap().update_settings(|s| s.repeat_added_tracks = Some(on));
    }

    pub fn set_track_progress(&self, on: bool) {
        self.inner.lock().unwrap().track_progress = on;
        self.persist.lock().unwrap().update_settings(|s| s.track_progress = Some(on));
    }

    /// Shows or hides the player in the system's media controls (and the media keys with it).
    pub fn set_media_controls(&self, on: bool) {
        let mut inner = self.inner.lock().unwrap();
        self.persist.lock().unwrap().update_settings(|s| s.media_controls = Some(on));
        if inner.media_controls == on {
            return;
        }
        inner.media_controls = on;
        if let Some(panel) = inner.panel.as_ref() {
            panel.set_enabled(on);
        }
    }

    // ---- saved logs and lists

    pub fn playback_events(&self) -> Value {
        json!({ "events": self.persist.lock().unwrap().pending_events() })
    }

    pub fn acknowledge_playback_events(&self, ids: &[String]) {
        self.persist.lock().unwrap().acknowledge_events(ids);
    }

    pub fn set_tracked_lists(&self, lists: Vec<ListConfig>) -> Result<()> {
        self.persist.lock().unwrap().set_list_configs(lists)
    }

    pub fn tracked_list(&self, id: &str) -> Value {
        json!({ "entries": self.persist.lock().unwrap().list_entries(id) })
    }

    pub fn set_tracked_list(&self, id: &str, entries: Vec<ListEntry>, merge: bool) -> Result<Value> {
        Ok(json!({ "entries": self.persist.lock().unwrap().set_list(id, entries, merge)? }))
    }

    pub fn tracked_list_changes(&self) -> Value {
        json!({ "changes": self.persist.lock().unwrap().pending_list_changes() })
    }

    pub fn acknowledge_tracked_list_changes(&self, ids: &[String]) {
        self.persist.lock().unwrap().acknowledge_list_changes(ids);
    }

    pub fn item_progress(&self, item_ids: Option<Vec<i64>>) -> Value {
        json!({ "entries": self.persist.lock().unwrap().progress(item_ids.as_deref()) })
    }

    pub fn set_item_progress(&self, entries: Vec<ProgressEntry>, merge: bool) -> Result<Value> {
        let mut parsed = Vec::with_capacity(entries.len());
        for (i, e) in entries.into_iter().enumerate() {
            if !e.progress.is_finite() || !(0.0..=1.0).contains(&e.progress) {
                return Err(format!("entries[{i}]: progress must be between 0 and 1"));
            }
            let updated_at_ms = e.updated_at_ms.unwrap_or_else(super::persist::now_ms);
            parsed.push((e.item_id, Progress { progress: e.progress, position_ms: e.position_ms.unwrap_or(0).max(0), updated_at_ms }));
        }
        Ok(json!({ "entries": self.persist.lock().unwrap().set_progress(parsed, merge) }))
    }

    pub fn progress_checkpoint(&self) -> Option<Value> {
        self.persist.lock().unwrap().checkpoint()
    }

    pub fn clear_progress_checkpoint(&self) {
        self.persist.lock().unwrap().clear_checkpoint();
    }

    // ---- the sleep timer

    /// Pauses after `minutes` (fading out over the last `fade_out_secs`), or with `end_of_track` when
    /// the current track ends. Replaces a running timer.
    pub fn set_sleep_timer(&self, minutes: Option<f64>, end_of_track: bool, fade_out_secs: Option<f64>) -> Result<Value> {
        let fade = fade_out_secs.unwrap_or(SLEEP_FADE_OUT_SECS);
        if !fade.is_finite() || fade < 0.0 {
            return Err("fadeOutSeconds must be >= 0".into());
        }
        let timer = if end_of_track {
            SleepTimer::EndOfTrack
        } else {
            let minutes = minutes.filter(|m| m.is_finite() && *m > 0.0).ok_or("minutes must be > 0, or endOfTrack true")?;
            let duration = Duration::from_secs_f64(minutes * 60.0);
            SleepTimer::At {
                ends_at: Instant::now() + duration,
                ends_at_ms: super::persist::now_ms() + duration.as_millis() as i64,
                fade_secs: fade.min(duration.as_secs_f64()),
            }
        };
        self.with_engine(|_, _| ())?;
        {
            let mut inner = self.inner.lock().unwrap();
            Self::clear_sleep_timer(&mut inner);
            inner.sleep = Some(timer);
            if let (SleepTimer::EndOfTrack, Some(engine)) = (timer, inner.engine.as_ref()) {
                engine.pause_after_current(true);
            }
        }
        Ok(self.state())
    }

    pub fn cancel_sleep_timer(&self) -> Value {
        Self::clear_sleep_timer(&mut self.inner.lock().unwrap());
        self.state()
    }

    /// Stops a running timer or fade, and puts the volume back.
    fn clear_sleep_timer(inner: &mut Inner) {
        if inner.sleep.take().is_some() {
            if let Some(engine) = inner.engine.as_ref() {
                engine.pause_after_current(false);
                engine.set_volume(inner.volume_curve.gain(inner.volume));
            }
        }
    }

    /// The background loop's part: fades out and pauses when the time is up; the end-of-track timer
    /// is done once the engine paused at a track's end (or the queue ended).
    fn check_sleep_timer(&self) {
        let mut inner = self.inner.lock().unwrap();
        let Some(timer) = inner.sleep else { return };
        let Some(engine) = inner.engine.as_ref() else { return };
        match timer {
            SleepTimer::EndOfTrack => {
                if engine.take_paused_after_current() || engine.status().ended {
                    Self::clear_sleep_timer(&mut inner);
                }
            }
            SleepTimer::At { ends_at, fade_secs, .. } => {
                let left = ends_at.saturating_duration_since(Instant::now()).as_secs_f64();
                if left <= 0.0 {
                    engine.pause();
                    Self::clear_sleep_timer(&mut inner);
                } else if left < fade_secs {
                    // Fading out: the set volume, scaled down to nothing at the end.
                    engine.set_volume(inner.volume_curve.gain(inner.volume) * (left / fade_secs) as f32);
                }
            }
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
                self.persist.lock().unwrap().update_settings(|s| s.output_device = id.clone());
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

/// An item's progress as setItemProgress sends it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEntry {
    pub item_id: i64,
    pub progress: f64,
    pub position_ms: Option<i64>,
    pub updated_at_ms: Option<i64>,
}

/// What the background loop knows about the current play.
#[derive(Default)]
struct Tracking {
    /// The engine's play number and request count when last seen (see engine::Status).
    play: Option<u64>,
    requests: u64,
    key: Option<u64>,
    item_id: Option<i64>,
    position: f64,
    duration: f64,
    playing: bool,
    /// `start` was logged: it actually played.
    started: bool,
    /// `complete` or `skip` was logged.
    finished: bool,
    /// Tracked lists this play counted towards (each once per play).
    counted: HashSet<String>,
    progress_saved: Option<Instant>,
    position_saved: Option<Instant>,
    saved_position: Option<(usize, f64)>,
}

/// Counts the current play towards the tracked lists whose countAfterSeconds it reached
/// (`completed`: it played to the end, which always counts).
fn count_lists(t: &mut Tracking, queue: &Queue, persist: &mut Persist, completed: bool, out: &mut Vec<(&'static str, Value)>) {
    let configs: Vec<ListConfig> = persist.list_configs().to_vec();
    for list in configs {
        if t.counted.contains(&list.id) || (!completed && t.position < list.count_after_seconds) {
            continue;
        }
        let value = match list.track.as_str() {
            "item" => match t.item_id {
                Some(id) => json!(id),
                None => continue,
            },
            _ => match &queue.source_id {
                Some(source) => json!(source),
                None => continue,
            },
        };
        t.counted.insert(list.id.clone());
        if let Some(change) = persist.record_list(&list.id, value) {
            out.push((TRACKED_LIST_EVENT, change));
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

/// resumeLastQueue as "off", "paused" or "play".
fn resume_mode(value: &Value) -> Option<&'static str> {
    match value {
        Value::Bool(true) => Some("paused"),
        Value::Bool(false) => Some("off"),
        Value::String(s) => ["off", "paused", "play"].into_iter().find(|m| m == s),
        _ => None,
    }
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
