//! The background loop: state events, playback tracking (events, tracked lists, progress, what's
//! saved), the output device and the sleep timer's checks.

use super::*;
use super::media_keys::PanelSync;

impl DesktopAudio {
    /// The background loop (state events, track changes, the media controls, the output device),
    /// and the media controls, the first time the engine is used.
    pub(super) fn start_background(&self, inner: &mut Inner) -> Result<()> {
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
        // Tests drive the loop themselves (tick).
        #[cfg(test)]
        if inner.manual_output.is_some() {
            return Ok(());
        }
        let me = self.clone();
        thread::Builder::new()
            .name("native-audio-state".into())
            .spawn(move || me.background_loop())
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub(super) fn background_loop(self) {
        let mut state = LoopState::default();
        loop {
            thread::sleep(Duration::from_millis(100));
            self.tick(&mut state);
        }
    }

    /// One round of the background loop.
    pub(super) fn tick(&self, l: &mut LoopState) {
        for (event, payload) in self.track(&mut l.tracking) {
            self.emit(event, &payload);
        }
        let poll = l.last_device_check.elapsed() >= DEVICE_CHECK_EVERY;
        if poll {
            l.last_device_check = Instant::now();
        }
        self.check_device(poll);
        self.check_sleep_timer();
        let state = self.state();
        self.sync_panel(&mut l.panel, &state);
        // Sent when something changes, and every 250 ms while playing (the position).
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
        if summary != l.last_state || (playing && l.last_sent.elapsed() >= Duration::from_millis(250)) {
            self.emit(STATE_EVENT, &state);
            l.last_state = summary;
            l.last_sent = Instant::now();
        }
    }

    /// Follows playback for what's saved: playback events (start / complete / skip), tracked lists,
    /// item progress, the progress checkpoint and the queue (with where it is). A new play that
    /// nobody asked for is the queue moving on by itself: the previous track completed. Also lets
    /// the queue start over (repeat all) when a new track starts. Returns the events to send.
    pub(super) fn track(&self, t: &mut Tracking) -> Vec<(&'static str, Value)> {
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
                        resync(engine, &mut queue);
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
    pub(super) fn finish_play(&self, t: &mut Tracking, queue: &Queue, persist: &mut Persist, completed: bool, track_progress: bool, out: &mut Vec<(&'static str, Value)>) {
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
    pub(super) fn check_device(&self, poll: bool) {
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
}

/// What the background loop keeps between rounds.
pub(super) struct LoopState {
    last_state: String,
    last_sent: Instant,
    last_device_check: Instant,
    panel: PanelSync,
    tracking: Tracking,
}

impl Default for LoopState {
    fn default() -> Self {
        Self {
            last_state: String::new(),
            last_sent: Instant::now(),
            last_device_check: Instant::now(),
            panel: PanelSync::default(),
            tracking: Tracking::default(),
        }
    }
}

/// What the background loop knows about the current play.
#[derive(Default)]
pub(super) struct Tracking {
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
pub(super) fn count_lists(t: &mut Tracking, queue: &Queue, persist: &mut Persist, completed: bool, out: &mut Vec<(&'static str, Value)>) {
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
