//! The desktop player end to end, without an audio device: real files through the decoder, the
//! engine, the queue and the background loop, with the output pulled by hand (Engine::new_manual)
//! and the loop ticked between pulls.

use std::fs;

use super::background::LoopState;
use super::*;

const RATE: u32 = 48_000;
const CHANNELS: usize = 2;
/// Output pulled per loop round in these tests (the real loop runs every 100 ms).
const CHUNK: usize = 1_000;

struct Harness {
    audio: DesktopAudio,
    dir: PathBuf,
    state: LoopState,
}

impl Harness {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("native-audio-player-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self::open(dir)
    }

    /// The player as it starts again with the same data folder (an app restart).
    fn open(dir: PathBuf) -> Self {
        let audio = DesktopAudio::new(None, Some(dir.join("data")));
        audio.inner.lock().unwrap().manual_output = Some((RATE, CHANNELS));
        Self { audio, dir, state: LoopState::default() }
    }

    /// A WAV file of `frames` stereo frames whose samples are unique to `seed`; returns it as a queue item.
    fn track(&self, name: &str, frames: usize, seed: i32, id: i64) -> Item {
        let path = self.dir.join(format!("{name}.wav"));
        let samples = samples(frames, seed);
        let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
        let data_len = (samples.len() * 2) as u32;
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
        bytes.extend_from_slice(&(CHANNELS as u16).to_le_bytes());
        bytes.extend_from_slice(&RATE.to_le_bytes());
        bytes.extend_from_slice(&(RATE * CHANNELS as u32 * 2).to_le_bytes());
        bytes.extend_from_slice(&((CHANNELS * 2) as u16).to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for s in &samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        fs::write(&path, bytes).unwrap();
        Item { src: path.display().to_string(), id: Some(id), title: Some(name.into()), artist: None, artwork_url: None }
    }

    /// The next frames of output, with a loop round after every CHUNK frames.
    fn play_frames(&mut self, frames: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(frames * CHANNELS);
        let mut left = frames;
        while left > 0 {
            let n = left.min(CHUNK);
            let pulled = self.audio.inner.lock().unwrap().engine.as_ref().unwrap().pull(n);
            out.extend(pulled);
            self.audio.tick(&mut self.state);
            left -= n;
        }
        out
    }

    fn events(&self) -> Vec<(String, i64)> {
        let events = self.audio.playback_events();
        events["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| (e["type"].as_str().unwrap().to_string(), e["itemId"].as_i64().unwrap()))
            .collect()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.audio.dispose();
    }
}

fn samples(frames: usize, seed: i32) -> Vec<i16> {
    (0..frames)
        .flat_map(|i| {
            let left = ((i as i32 * 37 + seed * 1_001) % 20_000 - 10_000) as i16;
            [left, left.wrapping_neg()]
        })
        .collect()
}

/// What the decoder makes of `samples` (16-bit integers as floats).
fn as_output(frames: usize, seed: i32) -> Vec<f32> {
    samples(frames, seed).into_iter().map(|s| s as f32 / 32_768.0).collect()
}

fn assert_audio(actual: &[f32], expected: &[f32], what: &str) {
    assert_eq!(actual.len(), expected.len(), "{what}: length");
    if let Some(i) = actual.iter().zip(expected).position(|(a, b)| (a - b).abs() > 1e-6) {
        panic!("{what}: differs at sample {i} (frame {}): {} vs {}", i / CHANNELS, actual[i], expected[i]);
    }
}

fn silent(samples: &[f32]) -> bool {
    samples.iter().all(|s| *s == 0.0)
}

#[test]
fn a_queue_plays_gaplessly_and_is_logged() {
    let mut h = Harness::new("gapless");
    let lengths = [12_000, 7_000, 9_000];
    let items: Vec<Item> = lengths.iter().enumerate().map(|(i, &n)| h.track(&format!("t{i}"), n, i as i32 + 1, i as i64 + 1)).collect();
    h.audio.set_tracked_lists(vec![crate::desktop::persist::ListConfig { id: "recent".into(), track: "item".into(), limit: 5, count_after_seconds: 0.0 }]).unwrap();
    h.audio.load(items, 0, 0.0, None).unwrap();
    h.audio.play().unwrap();

    let total: usize = lengths.iter().sum();
    let out = h.play_frames(total + 2_000);
    let expected: Vec<f32> = lengths.iter().enumerate().flat_map(|(i, &n)| as_output(n, i as i32 + 1)).collect();
    // Back to back, sample for sample: nothing added or lost between the tracks.
    assert_audio(&out[..total * CHANNELS], &expected, "the queue");
    assert!(silent(&out[total * CHANNELS..]), "silence after the last track");
    h.play_frames(CHUNK); // one more round to see the end
    assert_eq!(h.audio.state()["status"], "ended");

    let kinds: Vec<(String, i64)> = h.events();
    let expected_events: Vec<(String, i64)> =
        (1..=3).flat_map(|id| [("start".to_string(), id), ("complete".to_string(), id)]).collect();
    assert_eq!(kinds, expected_events);
    let recent: Vec<i64> = h.audio.tracked_list("recent")["entries"].as_array().unwrap().iter().map(|e| e["id"].as_i64().unwrap()).collect();
    assert_eq!(recent, vec![3, 2, 1]);
}

#[test]
fn skipping_is_logged_as_a_skip() {
    let mut h = Harness::new("skip");
    let items = vec![h.track("a", 20_000, 1, 1), h.track("b", 20_000, 2, 2)];
    h.audio.load(items, 0, 0.0, None).unwrap();
    h.audio.play().unwrap();
    h.play_frames(5_000);
    h.audio.next().unwrap();
    let out = h.play_frames(3_000);
    assert_audio(&out, &as_output(3_000, 2), "b from its start");
    assert_eq!(h.events(), vec![("start".into(), 1), ("skip".into(), 1), ("start".into(), 2)]);
}

#[test]
fn repeat_one_loops_gaplessly() {
    let mut h = Harness::new("repeat-one");
    let item = h.track("loop", 6_000, 4, 1);
    h.audio.load(vec![item], 0, 0.0, None).unwrap();
    h.audio.set_repeat_mode("one").unwrap();
    h.audio.play().unwrap();
    let out = h.play_frames(15_000);
    let once = as_output(6_000, 4);
    let expected: Vec<f32> = once.iter().chain(&once).chain(&once[..3_000 * CHANNELS]).copied().collect();
    assert_audio(&out, &expected, "three times round");
}

#[test]
fn the_end_of_track_sleep_timer_stops_exactly_where_the_next_track_starts() {
    let mut h = Harness::new("sleep");
    let items = vec![h.track("a", 10_000, 1, 1), h.track("b", 10_000, 2, 2)];
    h.audio.load(items, 0, 0.0, None).unwrap();
    h.audio.play().unwrap();
    h.audio.set_sleep_timer(None, true, None).unwrap();
    let out = h.play_frames(14_000);
    assert_audio(&out[..10_000 * CHANNELS], &as_output(10_000, 1), "the first track");
    let after = &out[10_000 * CHANNELS..];
    let first_sound = after.iter().position(|s| *s != 0.0);
    let b = as_output(10_000, 2);
    assert!(
        first_sound.is_none(),
        "nothing of the next track: sound {} frames after a; b's start there: {:?}; state {}",
        first_sound.unwrap_or(0) / CHANNELS,
        first_sound.map(|i| after[i..].iter().take(6).zip(&b).map(|(x, y)| (x - y).abs() < 1e-6).collect::<Vec<_>>()),
        h.audio.state()
    );
    let state = h.audio.state();
    assert_eq!(state["isPlaying"], false);
    assert_eq!(state["queueIndex"], 1, "the next track is ready");
    assert_eq!(state["currentTime"], 0.0);
    assert_eq!(state["sleepTimerEndOfTrack"], false, "the timer cleared itself");
    // Play continues with the next track.
    h.audio.play().unwrap();
    assert_audio(&h.play_frames(2_000), &as_output(2_000, 2), "the next track");
}

#[test]
fn update_queue_keeps_the_playing_track_going() {
    let mut h = Harness::new("update");
    let items = vec![h.track("a", 10_000, 1, 1), h.track("b", 10_000, 2, 2), h.track("c", 10_000, 3, 3)];
    let (a, c) = (items[0].clone(), items[2].clone());
    h.audio.load(items, 0, 0.0, None).unwrap();
    h.audio.play().unwrap();
    let first = h.play_frames(4_000);
    // The playlist is now C, A: B removed, A moved after C.
    let result = h.audio.update_queue(vec![c, a], None, Default::default(), false).unwrap();
    assert_eq!((result["added"].as_u64(), result["removed"].as_u64()), (Some(0), Some(1)));
    assert_eq!(result["state"]["queueIndex"], 1);
    assert_eq!(result["state"]["queueLength"], 2);
    // A plays on without a break, and nothing follows it (it's last now, repeat off).
    let rest = h.play_frames(8_000);
    let played: Vec<f32> = first.into_iter().chain(rest).collect();
    assert_audio(&played[..10_000 * CHANNELS], &as_output(10_000, 1), "a, uninterrupted");
    assert!(silent(&played[10_000 * CHANNELS..]));
}

#[test]
fn the_rust_api_reads_the_queue_and_its_source_and_updates_it() {
    // What app.native_audio() does on desktop: queue() and update_queue() through the API's types.
    let h = Harness::new("rust-api");
    let items = vec![h.track("a", 10_000, 1, 1), h.track("b", 10_000, 2, 2)];
    h.audio.load(items.clone(), 0, 0.0, Some("album:1".into())).unwrap();
    let queue: crate::Queue = serde_json::from_value(h.audio.queue()).unwrap();
    assert_eq!((queue.items.len(), queue.current_index, queue.source_id.as_deref()), (2, 0, Some("album:1")));
    assert_eq!(queue.items[1].id, Some(2));

    let (options, skip) = crate::desktop::commands::update_options(None, Some("keep".into()), Some("end".into()), None).unwrap();
    assert!(!skip && options.keep_removed);
    assert!(crate::desktop::commands::update_options(Some("later".into()), None, None, None).is_err());
    let result = h.audio.update_queue(vec![items[0].clone()], Some("album:2".into()), options, skip).unwrap();
    let result: crate::UpdateQueueResult = serde_json::from_value(result).unwrap();
    assert_eq!(result, crate::UpdateQueueResult { added: 0, removed: 1 });
    let queue: crate::Queue = serde_json::from_value(h.audio.queue()).unwrap();
    assert_eq!((queue.items.len(), queue.source_id.as_deref()), (2, Some("album:2")), "b kept (removedItems: keep)");
}

#[test]
fn the_queue_and_settings_come_back_after_a_restart() {
    let dir;
    {
        let mut h = Harness::new("restart");
        dir = h.dir.clone();
        let items = vec![h.track("a", 10_000, 1, 1), h.track("b", 30_000, 2, 2)];
        h.audio.load(items, 0, 0.0, Some("pl-1".into())).unwrap();
        h.audio.set_repeat_mode("all").unwrap();
        h.audio.set_volume(0.5);
        h.audio.play().unwrap();
        h.play_frames(10_000 + 12_000); // 12 000 frames (0.25 s) into b
        h.audio.pause().unwrap();
        h.play_frames(CHUNK); // a round to save the paused position
    }
    let mut h = Harness::open(dir);
    let state = h.audio.restore_last_queue().unwrap().expect("a saved queue");
    assert_eq!(state["queueIndex"], 1);
    assert_eq!(state["repeatMode"], "all");
    assert_eq!(state["volume"], 0.5);
    assert!((state["currentTime"].as_f64().unwrap() - 0.25).abs() < 0.03, "position {}", state["currentTime"]);
    // It continues from there (at half volume: the quadratic curve makes that a quarter of the amplitude).
    h.audio.set_volume(1.0);
    h.audio.play().unwrap();
    let position = (state["currentTime"].as_f64().unwrap() * RATE as f64).round() as usize;
    let out = h.play_frames(2_000);
    let expected = &as_output(30_000, 2)[position * CHANNELS..(position + 2_000) * CHANNELS];
    assert_audio(&out, expected, "b from where it was");
}

#[test]
fn the_volume_follows_its_curve() {
    let mut h = Harness::new("volume");
    // A constant full-scale-ish signal, so the output's level is easy to read.
    let item = h.track("tone", 200_000, 0, 1);
    h.audio.load(vec![item], 0, 0.0, None).unwrap();
    h.audio.play().unwrap();
    let level = |h: &mut Harness| {
        let out = h.play_frames(2_000);
        let input = as_output(200_000, 0);
        // Output vs input energy over the same frames is the gain (position doesn't matter for the ratio).
        let peak_out = out.iter().fold(0f32, |m, s| m.max(s.abs()));
        let peak_in = input.iter().take(out.len()).fold(0f32, |m, s| m.max(s.abs()));
        peak_out / peak_in
    };
    for (curve, power) in [("linear", 1), ("quadratic", 2), ("cubic", 3)] {
        h.audio.set_volume_curve(curve).unwrap();
        for step in [10, 25, 50, 60, 75, 90, 100] {
            let v = step as f64 / 100.0;
            h.audio.set_volume(v);
            h.play_frames(1_000); // let the change apply
            let gain = level(&mut h) as f64;
            let expected = v.powi(power);
            assert!((gain - expected).abs() < 0.01, "{curve} at {step}%: gain {gain:.3}, expected {expected:.3}");
            println!("{curve:>9} {step:>3}%: gain {gain:.3} ({:+.1} dB)", 20.0 * gain.max(1e-6).log10());
        }
    }
}
