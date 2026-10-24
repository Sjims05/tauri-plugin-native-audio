//! The system media controls and media keys: this player's, or, while the app controls another device
//! (set_remote_playback), that device's.

use super::*;

impl DesktopAudio {
    pub(super) fn sync_panel(&self, sync: &mut PanelSync, state: &Value) {
        let mut inner = self.inner.lock().unwrap();
        // Showing another device's playback: not this player's.
        if inner.remote.is_some() {
            return;
        }
        if std::mem::take(&mut inner.panel_resync) {
            *sync = PanelSync::default();
        }
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

    /// The app controls another device (`Some`: what plays there, given again as it changes), or not anymore
    /// (`None`). Meanwhile the media controls (the Windows media panel, the media keys) show that device's song and
    /// send their presses to the app's [`Self::set_remote_commands`] handler, and this player is paused.
    pub fn set_remote_playback(&self, playback: Option<RemotePlayback>) {
        if playback.is_some() && self.inner.lock().unwrap().remote.is_none() {
            let _ = self.pause();
        }
        let mut inner = self.inner.lock().unwrap();
        let before = std::mem::replace(&mut inner.remote, playback.clone());
        if playback.is_none() {
            inner.panel_resync = before.is_some();
        }
        let Some(panel) = inner.panel.as_ref().filter(|_| inner.media_controls) else { return };
        match playback {
            Some(now) => {
                let song = |p: &RemotePlayback| (p.title.clone(), p.artist.clone(), p.artwork_url.clone(), p.duration);
                if before.as_ref().is_none_or(|b| song(b) != song(&now)) {
                    panel.set_track(TrackInfo {
                        key: remote_key(&now),
                        path: PathBuf::new(),
                        title: now.title.clone(),
                        artist: now.artist.clone(),
                        artwork_url: now.artwork_url.clone(),
                        duration_secs: Some(now.duration).filter(|d| *d > 0.0),
                    });
                }
                panel.set_playback(now.playing, now.position);
            }
            // Back to this player: the background loop shows its track (or nothing).
            None if before.is_some() => panel.set_stopped(),
            None => {}
        }
    }

    /// Where the media controls' presses go while the app controls another device.
    pub fn set_remote_commands(&self, handler: Arc<dyn Fn(RemoteCommand) + Send + Sync>) {
        self.inner.lock().unwrap().remote_commands = Some(handler);
    }

    pub(super) fn on_media_event(&self, event: MediaControlEvent) {
        // Controlling another device: the press is for it (seeking by steps and the volume aren't).
        let remote = {
            let inner = self.inner.lock().unwrap();
            inner.remote.as_ref().map(|r| (r.playing, inner.remote_commands.clone()))
        };
        if let Some((playing, send)) = remote {
            let command = match event {
                MediaControlEvent::Play => RemoteCommand::Play,
                MediaControlEvent::Pause | MediaControlEvent::Stop => RemoteCommand::Pause,
                MediaControlEvent::Toggle if playing => RemoteCommand::Pause,
                MediaControlEvent::Toggle => RemoteCommand::Play,
                MediaControlEvent::Next => RemoteCommand::Next,
                MediaControlEvent::Previous => RemoteCommand::Previous,
                MediaControlEvent::SetPosition(position) => RemoteCommand::Seek { position: position.0.as_secs_f64() },
                _ => return,
            };
            if let Some(send) = send {
                send(command);
            }
            return;
        }
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
}

/// What the background loop last told the media controls.
#[derive(Default)]
pub(super) struct PanelSync {
    key: Option<u64>,
    duration: Option<f64>,
    /// Playing, position, and when.
    reported: Option<(bool, f64, Instant)>,
}

/// A key for another device's song (the media controls keep a cover per key).
fn remote_key(playback: &RemotePlayback) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (&playback.device, &playback.title, &playback.artist, &playback.artwork_url).hash(&mut hasher);
    hasher.finish()
}

pub(super) fn signed(direction: SeekDirection, seconds: f64) -> f64 {
    match direction {
        SeekDirection::Forward => seconds,
        SeekDirection::Backward => -seconds,
    }
}
