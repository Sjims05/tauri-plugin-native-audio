//! Playback and the queue: what the commands do.

use super::*;

impl DesktopAudio {
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
        self.with_engine(|engine, queue| {
            let key = queue.key_at(index);
            // Picking a track while shuffled plays it, then shuffles the rest of the queue after it (as on Android).
            if queue.shuffle && key.is_some() {
                queue.set_shuffle(true, key);
            }
            start_key(engine, queue, key)
        })?;
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
            "sourceId": queue.source_id,
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
}
