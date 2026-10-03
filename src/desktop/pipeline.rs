//! From tracks to output samples: decodes the tracks in play order and converts them into one
//! continuous stream at the output's rate and channel count. Used by the engine's feeder thread and
//! by the test program, so tests check exactly what plays.

use std::collections::VecDeque;
use std::path::PathBuf;

use super::convert::Converter;
use super::decode::TrackDecoder;

/// Where a track starts in the output stream.
#[derive(Debug, Clone)]
pub struct TrackStart {
    /// Output frame (counted from the pipeline's `first_frame`) where the track's first sample plays.
    pub output_frame: u64,
    pub index: usize,
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
    index: usize,
    start_frame: u64,
    input_frames: u64,
}

pub struct Pipeline {
    tracks: Vec<PathBuf>,
    out_rate: u32,
    out_channels: usize,
    current: Option<Current>,
    converter: Option<Converter>,
    steps: VecDeque<Step>,
    ended: bool,
}

impl Pipeline {
    /// Starts track `index` at `position` seconds; its first sample is output frame `first_frame`.
    pub fn new(tracks: Vec<PathBuf>, index: usize, position: f64, out_rate: u32, out_channels: usize, first_frame: u64) -> Self {
        let mut pipeline = Self {
            tracks,
            out_rate,
            out_channels,
            current: None,
            converter: None,
            steps: VecDeque::new(),
            ended: false,
        };
        pipeline.open(index, position, first_frame);
        pipeline
    }

    pub fn next(&mut self) -> Step {
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
                    let length = self.converter.as_ref().unwrap().output_length(current.input_frames);
                    self.open(current.index + 1, 0.0, current.start_frame + length);
                }
            }
        }
    }

    /// Opens track `index` (skipping ones that can't be played). Keeps the converter when the rate
    /// and channels stay the same, so there's no seam; otherwise flushes it first.
    fn open(&mut self, mut index: usize, position: f64, start_frame: u64) {
        while index < self.tracks.len() {
            let opened = TrackDecoder::open(&self.tracks[index]).and_then(|mut decoder| {
                if position > 0.0 {
                    decoder.seek(position)?;
                }
                Ok(decoder)
            });
            match opened {
                Ok(decoder) => {
                    let reusable = self.converter.as_ref().is_some_and(|c| c.accepts(decoder.sample_rate, decoder.channels));
                    if !reusable {
                        self.flush_converter();
                        match Converter::new(decoder.sample_rate, decoder.channels, self.out_rate, self.out_channels) {
                            Ok(converter) => self.converter = Some(converter),
                            Err(e) => {
                                self.steps.push_back(Step::Error(e));
                                index += 1;
                                continue;
                            }
                        }
                    }
                    self.steps.push_back(Step::TrackStart(TrackStart {
                        output_frame: start_frame,
                        index,
                        offset_secs: position,
                        duration_secs: decoder.duration_secs,
                    }));
                    self.current = Some(Current { decoder, index, start_frame, input_frames: 0 });
                    return;
                }
                Err(e) => {
                    self.steps.push_back(Step::Error(e));
                    index += 1;
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
