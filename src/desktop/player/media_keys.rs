//! The system media controls and media keys.

use super::*;

impl DesktopAudio {
    pub(super) fn sync_panel(&self, sync: &mut PanelSync, state: &Value) {
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

    pub(super) fn on_media_event(&self, event: MediaControlEvent) {
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

pub(super) fn signed(direction: SeekDirection, seconds: f64) -> f64 {
    match direction {
        SeekDirection::Forward => seconds,
        SeekDirection::Backward => -seconds,
    }
}
