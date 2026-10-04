//! The sleep timer.

use super::*;

impl DesktopAudio {
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
                ends_at_ms: crate::desktop::persist::now_ms() + duration.as_millis() as i64,
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
    pub(super) fn clear_sleep_timer(inner: &mut Inner) {
        if inner.sleep.take().is_some() {
            if let Some(engine) = inner.engine.as_ref() {
                engine.pause_after_current(false);
                engine.set_volume(inner.volume_curve.gain(inner.volume));
            }
        }
    }

    /// The background loop's part: fades out and pauses when the time is up; the end-of-track timer
    /// is done once the engine paused at a track's end (or the queue ended).
    pub(super) fn check_sleep_timer(&self) {
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
}
