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
use cpal::{FromSample, SizedSample};
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
}

pub struct Engine {
    shared: Arc<Shared>,
    commands: Sender<Command>,
    pub sample_rate: u32,
    pub channels: usize,
    output_thread: Option<thread::JoinHandle<()>>,
    feeder_thread: Option<thread::JoinHandle<()>>,
    stop_output: Sender<()>,
}

impl Engine {
    /// Opens the default output device. `next_track` tells what follows each track.
    pub fn new(next_track: NextTrack) -> Result<Self, String> {
        let shared = Arc::new(Shared {
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
        });

        // The device's own rate and channels; tracks are converted to it.
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or("no audio output device")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let sample_rate = supported.sample_rate().0;
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

        let (commands, command_rx) = mpsc::channel();
        let feeder_shared = shared.clone();
        let feeder_thread = thread::Builder::new()
            .name("native-audio-feeder".into())
            .spawn(move || Feeder::new(producer, feeder_shared, sample_rate, channels, next_track).run(command_rx))
            .map_err(|e| e.to_string())?;

        Ok(Self {
            shared,
            commands,
            sample_rate,
            channels,
            output_thread: Some(output_thread),
            feeder_thread: Some(feeder_thread),
            stop_output,
        })
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
    mut consumer: Consumer<f32>,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String> {
    let channels = config.channels as usize;
    let mut seen_flush = 0u64;
    let silence = T::from_sample(0.0f32);
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                // Seek / skip: drop what's buffered for the old position.
                let flush = shared.flush_gen.load(Ordering::SeqCst);
                if flush != seen_flush {
                    let available = consumer.slots();
                    if let Ok(chunk) = consumer.read_chunk(available) {
                        chunk.commit_all();
                    }
                    seen_flush = flush;
                    shared.flush_ack.store(flush, Ordering::SeqCst);
                }
                if !shared.playing.load(Ordering::SeqCst) {
                    data.fill(silence);
                    return;
                }
                let volume = f32::from_bits(shared.volume_bits.load(Ordering::SeqCst));
                let mut played = 0u64;
                let mut missing = 0u64;
                for frame in data.chunks_mut(channels) {
                    if consumer.slots() >= channels {
                        for sample in frame.iter_mut() {
                            *sample = T::from_sample(consumer.pop().unwrap_or(0.0) * volume);
                        }
                        played += 1;
                    } else {
                        frame.fill(silence);
                        missing += 1;
                    }
                }
                shared.frames_played.fetch_add(played, Ordering::SeqCst);
                if missing > 0 && !shared.ended.load(Ordering::SeqCst) {
                    shared.underrun_frames.fetch_add(missing, Ordering::SeqCst);
                }
            },
            |e| eprintln!("native-audio output error: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
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
        }
    }

    fn run(mut self, commands: Receiver<Command>) {
        loop {
            // Idle (nothing to decode): wait for a command. Busy: just check for one.
            let idle = self.pipeline.is_none() && self.pending.is_empty();
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
        let Some(pipeline) = self.pipeline.as_mut() else { return false };
        match pipeline.next() {
            Step::Samples(samples) => {
                self.pending.extend(samples);
                self.write_pending();
            }
            Step::TrackStart(start) => {
                self.shared.markers.lock().unwrap().push_back(Marker {
                    start_frame: start.output_frame,
                    track: start.track,
                    offset_secs: start.offset_secs,
                    duration_secs: start.duration_secs,
                });
                self.confirm_start();
            }
            Step::Error(e) => *self.shared.error.lock().unwrap() = Some(e),
            Step::End => {
                self.pipeline = None;
                self.shared.end_prepared.store(true, Ordering::SeqCst);
                self.confirm_start();
                self.mark_end_when_drained();
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

    /// After the last track: say "ended" once its audio has been written out.
    fn mark_end_when_drained(&mut self) {
        while !self.pending.is_empty() {
            if !self.write_pending() {
                thread::sleep(Duration::from_millis(5));
            }
        }
        // Ended once the output has played up to here, not when the last audio is buffered.
        self.shared.end_frame.store(self.frames_written, Ordering::SeqCst);
        self.shared.ended.store(true, Ordering::SeqCst);
    }
}
