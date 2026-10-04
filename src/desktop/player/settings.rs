//! Settings: volume, options and the output device.

use super::*;

impl DesktopAudio {
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
