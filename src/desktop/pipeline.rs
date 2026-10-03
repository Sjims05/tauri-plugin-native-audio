//! From tracks to output samples: decodes the tracks in play order and converts them into one
//! continuous stream at the output's rate and channel count. Used by the engine's feeder thread and
//! by the test program, so tests check exactly what plays.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use super::convert::Converter;
use super::decode::TrackDecoder;

/// A track to play: the queue entry's key (stays the same while the queue is edited) and its file.
#[derive(Debug, Clone)]
pub struct Track {
    pub key: u64,
    pub path: PathBuf,
}

/// What follows the track with this key, or None to stop after it. Asked when a track's decoding
/// ends (a couple of seconds before it finishes playing) or it can't be played.
pub type NextTrack = Arc<dyn Fn(u64) -> Option<Track> + Send + Sync>;

/// Where a track starts in the output stream.
#[derive(Debug, Clone)]
pub struct TrackStart {
    /// Output frame (counted from the pipeline's `first_frame`) where the track's first sample plays.
    pub output_frame: u64,
    pub track: Track,
    /// Seconds into the track that output_frame is (non-zero after a seek).
    pub offset_secs: f64,
    pub duration_secs: Option<f64>,
}

pub enum Step {
    Samples(Vec<f32>),
    TrackStart(TrackStart),
    /// A track couldn't be played and was skipped.
    Error(String),
    /// Nothing more follows.
    End,
}

struct Current {
    decoder: TrackDecoder,
    track: Track,
    start_frame: u64,
    input_frames: u64,
}

pub struct Pipeline {
    next_track: NextTrack,
    out_rate: u32,
    out_channels: usize,
    current: Option<Current>,
    converter: Option<Converter>,
    steps: VecDeque<Step>,
    ended: bool,
    /// Tracks that failed (or had no audio) since audio last played: met again, playback stops
    /// instead of going round in circles (repeat with nothing playable).
    failed: HashSet<u64>,
}

impl Pipeline {
    /// Starts `first` at `position` seconds; its first sample is output frame `first_frame`.
    pub fn new(first: Track, position: f64, next_track: NextTrack, out_rate: u32, out_channels: usize, first_frame: u64) -> Self {
        let mut pipeline = Self {
            next_track,
            out_rate,
            out_channels,
            current: None,
            converter: None,
            steps: VecDeque::new(),
            ended: false,
            failed: HashSet::new(),
        };
        pipeline.open(first, position, first_frame);
        pipeline
    }

    pub fn next_step(&mut self) -> Step {
        loop {
            if let Some(step) = self.steps.pop_front() {
                return step;
            }
            if self.ended {
                return Step::End;
            }
            let Some(current) = self.current.as_mut() else {
                self.finish();
                continue;
            };
            match current.decoder.next_block() {
                Ok(Some(block)) => {
                    current.input_frames += (block.len() / current.decoder.channels) as u64;
                    self.failed.clear();
                    let converter = self.converter.as_mut().expect("a converter while a track is open");
                    match converter.push(&block) {
                        Ok(samples) if !samples.is_empty() => return Step::Samples(samples),
                        Ok(_) => continue,
                        Err(e) => return Step::Error(e),
                    }
                }
                Ok(None) | Err(_) => {
                    // The next track starts exactly where this one's audio ends in the output.
                    let current = self.current.take().unwrap();
                    if current.input_frames == 0 {
                        self.failed.insert(current.track.key);
                    }
                    let length = self.converter.as_ref().unwrap().output_length(current.input_frames);
                    match self.following(current.track.key) {
                        Some(track) => self.open(track, 0.0, current.start_frame + length),
                        None => self.finish(),
                    }
                }
            }
        }
    }

    /// The track after `key`, unless it already failed since audio last played.
    fn following(&self, key: u64) -> Option<Track> {
        (self.next_track)(key).filter(|track| !self.failed.contains(&track.key))
    }

    /// Opens `track` (skipping ones that can't be played). Keeps the converter when the rate and
    /// channels stay the same, so there's no seam; otherwise flushes it first.
    fn open(&mut self, mut track: Track, mut position: f64, start_frame: u64) {
        loop {
            let opened = TrackDecoder::open(&track.path).and_then(|mut decoder| {
                if position > 0.0 {
                    decoder.seek(position)?;
                }
                Ok(decoder)
            });
            let result = opened.and_then(|decoder| {
                let reusable = self.converter.as_ref().is_some_and(|c| c.accepts(decoder.sample_rate, decoder.channels));
                if !reusable {
                    self.flush_converter();
                    self.converter = Some(Converter::new(decoder.sample_rate, decoder.channels, self.out_rate, self.out_channels)?);
                }
                Ok(decoder)
            });
            match result {
                Ok(decoder) => {
                    self.steps.push_back(Step::TrackStart(TrackStart {
                        output_frame: start_frame,
                        track: track.clone(),
                        offset_secs: position,
                        duration_secs: decoder.duration_secs,
                    }));
                    self.current = Some(Current { decoder, track, start_frame, input_frames: 0 });
                    return;
                }
                Err(e) => {
                    self.steps.push_back(Step::Error(e));
                    self.failed.insert(track.key);
                    match self.following(track.key) {
                        Some(next) => {
                            track = next;
                            position = 0.0;
                        }
                        None => break,
                    }
                }
            }
        }
        self.finish();
    }

    fn flush_converter(&mut self) {
        if let Some(mut converter) = self.converter.take() {
            match converter.flush() {
                Ok(samples) if !samples.is_empty() => self.steps.push_back(Step::Samples(samples)),
                Ok(_) => {}
                Err(e) => self.steps.push_back(Step::Error(e)),
            }
        }
    }

    fn finish(&mut self) {
        if !self.ended {
            self.flush_converter();
            self.ended = true;
        }
    }
}
