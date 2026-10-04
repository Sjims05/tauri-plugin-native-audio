//! Gapless playback on desktop.
//!
//! One output stream (cpal) runs for the whole session. A feeder thread decodes the tracks in play
//! order into a ring buffer, opening the next track (asked from the queue, see NextTrack) while the
//! current one is still playing, so the next track's first sample follows the previous track's last
//! one with nothing in between.
//! Pausing outputs silence instead of stopping the stream, so resuming has no startup delay either.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SizedSample};
use rtrb::{Consumer, Producer, RingBuffer};

use super::pipeline::{NextTrack, Pipeline, Step, Track};

/// Seconds of audio the feeder keeps ready ahead of the output.
const BUFFER_SECONDS: f32 = 2.0;

/// Where playback is.
#[derive(Debug, Clone)]
pub struct Status {
    /// The playing track's key, None when nothing is loaded.
    pub key: Option<u64>,
    /// The track already prepared to follow it (decoding moves on a couple of seconds early), if any.
    pub upcoming: Option<u64>,
    /// Decoding reached the end: nothing follows what's prepared.
    pub end_prepared: bool,
    /// A start() / stop() isn't done yet (the track is being opened).
    pub loading: bool,
    /// Counts the plays: each track start (a new track, the same one again with repeat one, or
    /// after start()) has its own number. None while nothing plays.
    pub play: Option<u64>,
    /// How many start() / stop() calls there were: a new `play` without one is the queue moving on
    /// by itself (the previous track played to the end).
    pub requests: u64,
    pub position_secs: f64,
    pub duration_secs: Option<f64>,
    pub playing: bool,
    /// The last track finished and nothing follows.
    pub ended: bool,
    /// Output frames that had to be filled with silence because nothing was decoded yet (a gap).
    pub underrun_frames: u64,
    pub error: Option<String>,
}

enum Command {
    /// `id`: confirmed in Shared::done once the track is open (or nothing could be played).
    Start { track: Track, position: f64, id: u64 },
    Stop { id: u64 },
    Shutdown,
}

/// A track start in the output stream: from output frame `start_frame` on, `track` plays,
/// beginning `offset_secs` into it.
#[derive(Clone)]
struct Marker {
    /// Numbers the plays (see Status::play).
    play: u64,
    start_frame: u64,
    track: Track,
    offset_secs: f64,
    duration_secs: Option<f64>,
}

struct Shared {
    playing: AtomicBool,
    volume_bits: AtomicU32,
    /// Output frames played (popped from the buffer) so far.
    frames_played: AtomicU64,
    underrun_frames: AtomicU64,
    /// Decoding reached the end (nothing follows) ...
    end_prepared: AtomicBool,
    /// ... and its audio is all in the buffer ...
    ended: AtomicBool,
    /// ... and this is the output frame where it stops.
    end_frame: AtomicU64,
    /// Bumped to make the output drop everything buffered (seek, skip); acknowledged in flush_ack.
    flush_gen: AtomicU64,
    flush_ack: AtomicU64,
    markers: Mutex<VecDeque<Marker>>,
    error: Mutex<Option<String>>,
    /// The last start() / stop() asked for, and the last one done.
    requested: AtomicU64,
    done: AtomicU64,
    /// The output stream stopped working (device unplugged or disabled, the system default changed):
    /// it needs reopening.
    device_lost: AtomicBool,
    /// Pause once the current track is over (the end-of-track sleep timer) ...
    pause_after_current: AtomicBool,
    /// ... at this output frame, where the next track starts (u64::MAX: not known yet / none) ...
    pause_at_frame: AtomicU64,
    /// ... and it did.
    paused_after_current: AtomicBool,
}

pub struct Engine {
    shared: Arc<Shared>,
    commands: Sender<Command>,
    pub sample_rate: u32,
    pub channels: usize,
    /// The chosen output device's id (see output_devices), or None when following the system default.
    pub device_id: Option<String>,
    output_thread: Option<thread::JoinHandle<()>>,
    feeder_thread: Option<thread::JoinHandle<()>>,
    stop_output: Sender<()>,
    /// Tests: the output, pulled by hand instead of by an audio device.
    #[cfg(test)]
    manual: Option<Arc<Mutex<Renderer>>>,
}

impl Engine {
    /// Opens the output device with id `device` (from output_devices), or follows the system default
    /// when None or not there. `next_track` tells what follows each track.
    pub fn new(next_track: NextTrack, device: Option<&str>) -> Result<Self, String> {
        let shared = new_shared();

        // The device's own rate and channels; tracks are converted to it.
        let host = cpal::default_host();
        let chosen = device.and_then(|id| find_device(&host, id).map(|d| (id.to_string(), d)));
        let (device_id, device) = match chosen {
            Some((id, device)) => (Some(id), device),
            // The system default itself (on Windows its virtual endpoint), not the device that's
            // the default right now: so per-app routing (Windows settings, SteelSeries Sonar, ...)
            // applies, and a change of default is reported (device_lost) to reopen on the new one.
            None => (None, host.default_output_device().ok_or("no audio output device")?),
        };
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let sample_rate = supported.sample_rate();
        let channels = supported.channels() as usize;
        let capacity = (sample_rate as f32 * BUFFER_SECONDS) as usize * channels;
        let (producer, consumer) = RingBuffer::<f32>::new(capacity);

        // cpal streams can't move between threads on every platform, so one thread owns it.
        let (stop_output, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let output_shared = shared.clone();
        let output_thread = thread::Builder::new()
            .name("native-audio-output".into())
            .spawn(move || {
                let stream = match build_stream(&device, supported, consumer, output_shared) {
                    Ok(stream) => stream,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let _ = ready_tx.send(stream.play().map_err(|e| e.to_string()));
                let _ = stop_rx.recv();
                drop(stream);
            })
            .map_err(|e| e.to_string())?;
        ready_rx.recv().map_err(|_| "audio output thread stopped".to_string())??;

        let (commands, feeder_thread) = start_feeder(producer, shared.clone(), sample_rate, channels, next_track)?;

        Ok(Self {
            shared,
            commands,
            sample_rate,
            channels,
            device_id,
            output_thread: Some(output_thread),
            feeder_thread: Some(feeder_thread),
            stop_output,
            #[cfg(test)]
            manual: None,
        })
    }

    /// Tests: an engine without an audio device, whose output is pulled with `pull`.
    #[cfg(test)]
    pub fn new_manual(next_track: NextTrack, sample_rate: u32, channels: usize) -> Result<Self, String> {
        let shared = new_shared();
        let capacity = (sample_rate as f32 * BUFFER_SECONDS) as usize * channels;
        let (producer, consumer) = RingBuffer::<f32>::new(capacity);
        let renderer = Arc::new(Mutex::new(Renderer::new(consumer, shared.clone(), channels)));
        // Seeks and skips wait for the output to drop what's buffered: answer that like an output would.
        let (stop_output, stop_rx) = mpsc::channel::<()>();
        let flusher = renderer.clone();
        let output_thread = thread::spawn(move || loop {
            flusher.lock().unwrap().drop_flushed();
            match stop_rx.recv_timeout(Duration::from_millis(1)) {
                Err(RecvTimeoutError::Timeout) => {}
                _ => return,
            }
        });
        let (commands, feeder_thread) = start_feeder(producer, shared.clone(), sample_rate, channels, next_track)?;
        Ok(Self {
            shared,
            commands,
            sample_rate,
            channels,
            device_id: None,
            output_thread: Some(output_thread),
            feeder_thread: Some(feeder_thread),
            stop_output,
            manual: Some(renderer),
        })
    }

    /// Tests: the next `frames` of output, waiting (up to a few seconds) until they're decoded or
    /// nothing more comes. Paused: silence, and the position doesn't move, like a real output.
    #[cfg(test)]
    pub fn pull(&self, frames: usize) -> Vec<f32> {
        let renderer = self.manual.as_ref().expect("a manual engine");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let ready = {
                let renderer = renderer.lock().unwrap();
                renderer.consumer.slots() >= frames * self.channels
                    || self.shared.end_prepared.load(Ordering::SeqCst)
                    || !self.shared.playing.load(Ordering::SeqCst)
            };
            if ready || Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        let mut out = vec![0.0f32; frames * self.channels];
        renderer.lock().unwrap().render(&mut out);
        out
    }

    /// Plays `track` from `position` seconds on (once playing: play() / pause() don't change),
    /// dropping whatever was prepared.
    pub fn start(&self, track: Track, position: f64) {
        let id = self.shared.requested.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.commands.send(Command::Start { track, position, id });
    }

    /// Drops everything: nothing loaded.
    pub fn stop(&self) {
        let id = self.shared.requested.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.commands.send(Command::Stop { id });
    }

    /// Waits for the engine to finish the start() / stop() calls made so far, without holding on
    /// to the engine (so the caller can let go of its locks while waiting).
    pub fn waiter(&self) -> Waiter {
        Waiter(self.shared.clone())
    }

    /// Seconds into the current track.
    pub fn seek(&self, position: f64) {
        if let Some(track) = self.current_track() {
            self.start(track, position);
        }
    }

    /// The output device stopped working: open a new Engine.
    /// Pause exactly where the current track ends (the next one is then ready at its start), or not.
    pub fn pause_after_current(&self, on: bool) {
        // Under the markers lock, like the feeder adding a track start: one of the two always sees
        // the other's change.
        let markers = self.shared.markers.lock().unwrap();
        self.shared.pause_after_current.store(on, Ordering::SeqCst);
        let at = if on {
            // The next track may be prepared already.
            let played = self.shared.frames_played.load(Ordering::SeqCst);
            markers.iter().find(|m| m.start_frame > played).map_or(u64::MAX, |m| m.start_frame)
        } else {
            u64::MAX
        };
        self.shared.pause_at_frame.store(at, Ordering::SeqCst);
    }

    /// Playback paused at the end of a track (pause_after_current) since the last call.
    pub fn take_paused_after_current(&self) -> bool {
        self.shared.paused_after_current.swap(false, Ordering::SeqCst)
    }

    pub fn device_lost(&self) -> bool {
        self.shared.device_lost.load(Ordering::SeqCst)
    }

    /// The playing track.
    pub fn current_track(&self) -> Option<Track> {
        let played = self.shared.frames_played.load(Ordering::SeqCst);
        let markers = self.shared.markers.lock().unwrap();
        markers.iter().take_while(|m| m.start_frame <= played).last().map(|m| m.track.clone())
    }

    pub fn play(&self) {
        self.shared.playing.store(true, Ordering::SeqCst);
    }

    pub fn pause(&self) {
        self.shared.playing.store(false, Ordering::SeqCst);
    }

    /// 0.0 to 1.0.
    pub fn set_volume(&self, volume: f32) {
        self.shared.volume_bits.store(volume.clamp(0.0, 1.0).to_bits(), Ordering::SeqCst);
    }

    pub fn status(&self) -> Status {
        let played = self.shared.frames_played.load(Ordering::SeqCst);
        let mut markers = self.shared.markers.lock().unwrap();
        // Markers of tracks that finished playing aren't needed anymore.
        while markers.len() > 1 && markers[1].start_frame <= played {
            markers.pop_front();
        }
        let current = markers.front().filter(|m| m.start_frame <= played).cloned();
        let upcoming = markers.iter().find(|m| m.start_frame > played).map(|m| m.track.key);
        let ended = self.shared.ended.load(Ordering::SeqCst) && played >= self.shared.end_frame.load(Ordering::SeqCst);
        Status {
            key: current.as_ref().map(|m| m.track.key),
            upcoming,
            end_prepared: self.shared.end_prepared.load(Ordering::SeqCst),
            loading: self.shared.done.load(Ordering::SeqCst) < self.shared.requested.load(Ordering::SeqCst),
            play: current.as_ref().map(|m| m.play),
            requests: self.shared.requested.load(Ordering::SeqCst),
            position_secs: current
                .as_ref()
                .map_or(0.0, |m| m.offset_secs + (played - m.start_frame) as f64 / self.sample_rate as f64),
            duration_secs: current.and_then(|m| m.duration_secs),
            playing: self.shared.playing.load(Ordering::SeqCst),
            ended,
            underrun_frames: self.shared.underrun_frames.load(Ordering::SeqCst),
            error: self.shared.error.lock().unwrap().clone(),
        }
    }
}

pub struct Waiter(Arc<Shared>);

impl Waiter {
    /// Returns whether the engine caught up within `timeout`.
    pub fn wait(&self, timeout: Duration) -> bool {
        let target = self.0.requested.load(Ordering::SeqCst);
        let deadline = Instant::now() + timeout;
        while self.0.done.load(Ordering::SeqCst) < target {
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(1));
        }
        true
    }
}

/// An output device: `id` is stable across runs (for setOutputDevice), `name` is for people.
pub struct OutputDevice {
    pub id: String,
    pub name: String,
}

/// The output devices, and the system default's id.
pub fn output_devices() -> (Vec<OutputDevice>, Option<String>) {
    let host = cpal::default_host();
    let devices = host
        .output_devices()
        .map(|devices| {
            devices
                .filter_map(|d| Some(OutputDevice { id: d.id().ok()?.to_string(), name: d.to_string() }))
                .collect()
        })
        .unwrap_or_default();
    (devices, default_output_device().map(|d| d.id))
}

/// The device that's the system default right now.
pub fn default_output_device() -> Option<OutputDevice> {
    let device = cpal::default_host().default_output_device()?;
    Some(OutputDevice { id: device.id().ok()?.to_string(), name: device.to_string() })
}

fn find_device(host: &cpal::Host, id: &str) -> Option<cpal::Device> {
    host.output_devices().ok()?.find(|d| d.id().is_ok_and(|d| d.to_string() == id))
}

fn new_shared() -> Arc<Shared> {
    Arc::new(Shared {
        playing: AtomicBool::new(false),
        volume_bits: AtomicU32::new(1.0f32.to_bits()),
        frames_played: AtomicU64::new(0),
        underrun_frames: AtomicU64::new(0),
        end_prepared: AtomicBool::new(true),
        ended: AtomicBool::new(true),
        end_frame: AtomicU64::new(0),
        flush_gen: AtomicU64::new(0),
        flush_ack: AtomicU64::new(0),
        markers: Mutex::new(VecDeque::new()),
        error: Mutex::new(None),
        requested: AtomicU64::new(0),
        done: AtomicU64::new(0),
        device_lost: AtomicBool::new(false),
        pause_after_current: AtomicBool::new(false),
        pause_at_frame: AtomicU64::new(u64::MAX),
        paused_after_current: AtomicBool::new(false),
    })
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        let _ = self.stop_output.send(());
        if let Some(t) = self.feeder_thread.take() {
            let _ = t.join();
        }
        if let Some(t) = self.output_thread.take() {
            let _ = t.join();
        }
    }
}

fn build_stream(
    device: &cpal::Device,
    supported: cpal::SupportedStreamConfig,
    consumer: Consumer<f32>,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String> {
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    match format {
        cpal::SampleFormat::F32 => output_stream::<f32>(device, &config, consumer, shared),
        cpal::SampleFormat::I16 => output_stream::<i16>(device, &config, consumer, shared),
        cpal::SampleFormat::U16 => output_stream::<u16>(device, &config, consumer, shared),
        cpal::SampleFormat::I32 => output_stream::<i32>(device, &config, consumer, shared),
        other => Err(format!("unsupported output sample format {other:?}")),
    }
}

fn output_stream<T: SizedSample + FromSample<f32> + Send + 'static>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    consumer: Consumer<f32>,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String> {
    let error_shared = shared.clone();
    let mut renderer = Renderer::new(consumer, shared, config.channels as usize);
    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _| renderer.render(data),
            move |e: cpal::Error| match e.kind() {
                // A glitch, or the system moved the stream itself: it keeps playing.
                cpal::ErrorKind::Xrun | cpal::ErrorKind::DeviceChanged => {}
                _ => {
                    eprintln!("native-audio output: {e}");
                    error_shared.device_lost.store(true, Ordering::SeqCst);
                }
            },
            None,
        )
        .map_err(|e| e.to_string())
}

/// What the output does with each buffer it has to fill: the buffered audio at the volume, silence
/// while paused, stopping exactly at the end-of-track sleep timer's frame, and dropping what's
/// buffered when a seek or skip asks for it.
struct Renderer {
    consumer: Consumer<f32>,
    shared: Arc<Shared>,
    channels: usize,
    seen_flush: u64,
}

impl Renderer {
    fn new(consumer: Consumer<f32>, shared: Arc<Shared>, channels: usize) -> Self {
        Self { consumer, shared, channels, seen_flush: 0 }
    }

    /// Seek / skip: drop what's buffered for the old position.
    fn drop_flushed(&mut self) {
        let flush = self.shared.flush_gen.load(Ordering::SeqCst);
        if flush != self.seen_flush {
            let available = self.consumer.slots();
            if let Ok(chunk) = self.consumer.read_chunk(available) {
                chunk.commit_all();
            }
            self.seen_flush = flush;
            self.shared.flush_ack.store(flush, Ordering::SeqCst);
        }
    }

    fn render<T: Sample + FromSample<f32>>(&mut self, data: &mut [T]) {
        let shared = self.shared.clone();
        let silence = T::from_sample(0.0f32);
        self.drop_flushed();
        if !shared.playing.load(Ordering::SeqCst) {
            data.fill(silence);
            return;
        }
        let volume = f32::from_bits(shared.volume_bits.load(Ordering::SeqCst));
        // The end-of-track sleep timer: stop right where the next track starts.
        let pause_at = shared.pause_at_frame.load(Ordering::SeqCst);
        let until_pause = pause_at.saturating_sub(shared.frames_played.load(Ordering::SeqCst));
        let mut played = 0u64;
        let mut missing = 0u64;
        for frame in data.chunks_mut(self.channels) {
            if played >= until_pause {
                frame.fill(silence);
            } else if self.consumer.slots() >= self.channels {
                for sample in frame.iter_mut() {
                    *sample = T::from_sample(self.consumer.pop().unwrap_or(0.0) * volume);
                }
                played += 1;
            } else {
                frame.fill(silence);
                missing += 1;
            }
        }
        shared.frames_played.fetch_add(played, Ordering::SeqCst);
        if played >= until_pause {
            shared.playing.store(false, Ordering::SeqCst);
            shared.pause_at_frame.store(u64::MAX, Ordering::SeqCst);
            shared.pause_after_current.store(false, Ordering::SeqCst);
            shared.paused_after_current.store(true, Ordering::SeqCst);
        }
        if missing > 0 && !shared.ended.load(Ordering::SeqCst) {
            shared.underrun_frames.fetch_add(missing, Ordering::SeqCst);
        }
    }
}

fn start_feeder(
    producer: Producer<f32>,
    shared: Arc<Shared>,
    sample_rate: u32,
    channels: usize,
    next_track: NextTrack,
) -> Result<(Sender<Command>, thread::JoinHandle<()>), String> {
    let (commands, command_rx) = mpsc::channel();
    let feeder = thread::Builder::new()
        .name("native-audio-feeder".into())
        .spawn(move || Feeder::new(producer, shared, sample_rate, channels, next_track).run(command_rx))
        .map_err(|e| e.to_string())?;
    Ok((commands, feeder))
}

/// Moves the pipeline's output into the ring buffer.
struct Feeder {
    producer: Producer<f32>,
    shared: Arc<Shared>,
    sample_rate: u32,
    channels: usize,
    next_track: NextTrack,
    pipeline: Option<Pipeline>,
    /// Converted samples that didn't fit in the ring buffer yet.
    pending: VecDeque<f32>,
    /// Output frames written to the ring buffer so far (same clock as Shared::frames_played).
    frames_written: u64,
    /// The start() being opened, confirmed at its first track start (or end).
    starting: Option<u64>,
    /// Decoding ended: "ended" once the rest of the audio is in the buffer.
    draining_end: bool,
    plays: u64,
}

impl Feeder {
    fn new(producer: Producer<f32>, shared: Arc<Shared>, sample_rate: u32, channels: usize, next_track: NextTrack) -> Self {
        Self {
            producer,
            shared,
            sample_rate,
            channels,
            next_track,
            pipeline: None,
            pending: VecDeque::new(),
            frames_written: 0,
            starting: None,
            draining_end: false,
            plays: 0,
        }
    }

    fn run(mut self, commands: Receiver<Command>) {
        loop {
            // Idle (nothing to decode): wait for a command. Busy: just check for one.
            let idle = self.pipeline.is_none() && self.pending.is_empty() && !self.draining_end;
            let command = if idle {
                match commands.recv_timeout(Duration::from_millis(200)) {
                    Ok(c) => Some(c),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            } else {
                commands.try_recv().ok()
            };
            match command {
                Some(Command::Shutdown) => return,
                Some(Command::Start { track, position, id }) => {
                    self.reset();
                    self.starting = Some(id);
                    self.pipeline = Some(Pipeline::new(
                        track,
                        position,
                        self.next_track.clone(),
                        self.sample_rate,
                        self.channels,
                        self.frames_written,
                    ));
                }
                Some(Command::Stop { id }) => {
                    self.reset();
                    self.shared.end_prepared.store(true, Ordering::SeqCst);
                    self.shared.end_frame.store(self.frames_written, Ordering::SeqCst);
                    self.shared.ended.store(true, Ordering::SeqCst);
                    self.shared.done.store(id, Ordering::SeqCst);
                }
                None => {}
            }
            if !self.fill() {
                thread::sleep(Duration::from_millis(5));
            }
        }
    }

    /// Drops everything prepared and queued for output.
    fn reset(&mut self) {
        let flush = self.shared.flush_gen.fetch_add(1, Ordering::SeqCst) + 1;
        // Wait (briefly) until the output has dropped the old audio, so the frame clocks line up.
        let deadline = Instant::now() + Duration::from_millis(500);
        while self.shared.flush_ack.load(Ordering::SeqCst) != flush && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        self.pending.clear();
        self.frames_written = self.shared.frames_played.load(Ordering::SeqCst);
        self.shared.markers.lock().unwrap().clear();
        self.shared.end_prepared.store(false, Ordering::SeqCst);
        self.shared.ended.store(false, Ordering::SeqCst);
        *self.shared.error.lock().unwrap() = None;
        self.pipeline = None;
        self.starting = None;
        self.shared.pause_at_frame.store(u64::MAX, Ordering::SeqCst);
        self.draining_end = false;
    }

    /// The start() being opened is done.
    fn confirm_start(&mut self) {
        if let Some(id) = self.starting.take() {
            self.shared.done.store(id, Ordering::SeqCst);
        }
    }

    /// Moves audio from the pipeline into the ring buffer. Returns false when there was nothing to
    /// do (buffer full, or nothing left).
    fn fill(&mut self) -> bool {
        // First whatever is still waiting.
        if !self.pending.is_empty() {
            let wrote = self.write_pending();
            if !self.pending.is_empty() {
                return wrote;
            }
        }
        if self.draining_end {
            // Ended once the output has played up to here, not when the last audio is buffered.
            self.draining_end = false;
            self.shared.end_frame.store(self.frames_written, Ordering::SeqCst);
            self.shared.ended.store(true, Ordering::SeqCst);
            return true;
        }
        let Some(pipeline) = self.pipeline.as_mut() else { return false };
        match pipeline.next_step() {
            Step::Samples(samples) => {
                self.pending.extend(samples);
                self.write_pending();
            }
            Step::TrackStart(start) => {
                self.plays += 1;
                let mut markers = self.shared.markers.lock().unwrap();
                // The end-of-track sleep timer pauses where the next track starts: any track start but
                // the first after a start() (that's the one playing). Even if the output already got
                // there (decoding fell behind), it stops right at the start. Checked under the markers
                // lock, like pause_after_current, so a timer set right now isn't missed.
                if self.shared.pause_after_current.load(Ordering::SeqCst) && self.starting.is_none() {
                    let _ = self.shared.pause_at_frame.compare_exchange(u64::MAX, start.output_frame, Ordering::SeqCst, Ordering::SeqCst);
                }
                markers.push_back(Marker {
                    play: self.plays,
                    start_frame: start.output_frame,
                    track: start.track,
                    offset_secs: start.offset_secs,
                    duration_secs: start.duration_secs,
                });
                drop(markers);
                self.confirm_start();
            }
            Step::Error(e) => *self.shared.error.lock().unwrap() = Some(e),
            Step::End => {
                self.pipeline = None;
                self.shared.end_prepared.store(true, Ordering::SeqCst);
                self.confirm_start();
                self.draining_end = true;
            }
        }
        true
    }

    /// Writes pending samples, whole frames only. Returns whether anything was written.
    fn write_pending(&mut self) -> bool {
        let free_frames = self.producer.slots() / self.channels;
        let frames = (self.pending.len() / self.channels).min(free_frames);
        if frames == 0 {
            return false;
        }
        let count = frames * self.channels;
        if let Ok(mut chunk) = self.producer.write_chunk_uninit(count) {
            let (first, second) = chunk.as_mut_slices();
            for slot in first.iter_mut().chain(second.iter_mut()) {
                slot.write(self.pending.pop_front().unwrap_or(0.0));
            }
            unsafe { chunk.commit_all() };
            self.frames_written += frames as u64;
        }
        true
    }
}
