//! Converting decoded audio to the output's sample rate and channel count.
//!
//! One converter keeps running across tracks with the same rate and channels, so a resampled stream
//! has no seam at a track change: no flush, no padding, no restart delay. The resampler's own delay
//! is dropped once at the start, and its padding is cut off when it's finally flushed.

use rubato::{FftFixedIn, Resampler};

pub struct Converter {
    pub in_rate: u32,
    pub in_channels: usize,
    out_rate: u32,
    out_channels: usize,
    resampler: Option<FftFixedIn<f32>>,
    /// Input waiting for a full resampler chunk, one Vec per channel.
    pending: Vec<Vec<f32>>,
    /// Output frames still to drop: the resampler's delay.
    delay_left: usize,
    input_frames: u64,
    output_frames: u64,
}

impl Converter {
    pub fn new(in_rate: u32, in_channels: usize, out_rate: u32, out_channels: usize) -> Result<Self, String> {
        let resampler = if in_rate == out_rate {
            None
        } else {
            Some(FftFixedIn::<f32>::new(in_rate as usize, out_rate as usize, 1024, 2, in_channels).map_err(|e| e.to_string())?)
        };
        let delay_left = resampler.as_ref().map_or(0, |r| r.output_delay());
        Ok(Self {
            in_rate,
            in_channels,
            out_rate,
            out_channels,
            resampler,
            pending: vec![Vec::new(); in_channels],
            delay_left,
            input_frames: 0,
            output_frames: 0,
        })
    }

    /// Whether audio with this rate and channel count can continue through this converter.
    pub fn accepts(&self, rate: u32, channels: usize) -> bool {
        self.in_rate == rate && self.in_channels == channels
    }

    /// Output frames `input_frames` of input become.
    pub fn output_length(&self, input_frames: u64) -> u64 {
        (input_frames as f64 * self.out_rate as f64 / self.in_rate as f64).round() as u64
    }

    /// Converts interleaved input; returns interleaved output (possibly empty while the resampler
    /// collects a full chunk).
    pub fn push(&mut self, input: &[f32]) -> Result<Vec<f32>, String> {
        self.input_frames += (input.len() / self.in_channels) as u64;
        let Some(resampler) = self.resampler.as_mut() else {
            self.output_frames += (input.len() / self.in_channels) as u64;
            return Ok(remap_channels(input, self.in_channels, self.out_channels));
        };
        for frame in input.chunks_exact(self.in_channels) {
            for (channel, sample) in frame.iter().enumerate() {
                self.pending[channel].push(*sample);
            }
        }
        let mut resampled = Vec::new();
        while self.pending[0].len() >= resampler.input_frames_next() {
            let need = resampler.input_frames_next();
            let chunk: Vec<Vec<f32>> = self.pending.iter_mut().map(|c| c.drain(..need).collect()).collect();
            resampled.push(resampler.process(&chunk, None).map_err(|e| e.to_string())?);
        }
        let mut out = Vec::new();
        for channels in resampled {
            out.extend(self.take(channels));
        }
        Ok(out)
    }

    /// Everything still inside (only when no more audio of this rate follows), cut to the exact
    /// length the input becomes, so the resampler's zero padding isn't played.
    pub fn flush(&mut self) -> Result<Vec<f32>, String> {
        if self.resampler.is_none() {
            return Ok(Vec::new());
        }
        let expected = self.output_length(self.input_frames);
        let mut out = Vec::new();
        let rest: Vec<Vec<f32>> = self.pending.iter_mut().map(std::mem::take).collect();
        // rubato rejects an empty buffer: nothing left over is the same as no input.
        let mut input = if rest[0].is_empty() { None } else { Some(rest) };
        // Feed silence until everything real has come out of the resampler's delay line.
        for _ in 0..8 {
            if self.output_frames >= expected {
                break;
            }
            let resampler = self.resampler.as_mut().unwrap();
            let resampled = resampler.process_partial(input.take().as_deref(), None).map_err(|e| e.to_string())?;
            out.extend(self.take(resampled));
        }
        // Cut off the padding beyond the expected length.
        let extra = self.output_frames.saturating_sub(expected) as usize;
        out.truncate(out.len().saturating_sub(extra * self.out_channels));
        self.output_frames = expected;
        Ok(out)
    }

    /// Interleaves resampled output, dropping the resampler's delay at the very start.
    fn take(&mut self, channels: Vec<Vec<f32>>) -> Vec<f32> {
        let frames = channels.first().map_or(0, Vec::len);
        let skip = self.delay_left.min(frames);
        self.delay_left -= skip;
        let mut interleaved = Vec::with_capacity((frames - skip) * self.in_channels);
        for i in skip..frames {
            for channel in &channels {
                interleaved.push(channel[i]);
            }
        }
        self.output_frames += (frames - skip) as u64;
        remap_channels(&interleaved, self.in_channels, self.out_channels)
    }
}

/// Mono plays on every output channel; otherwise the first channels map one to one and extra
/// output channels stay silent.
fn remap_channels(input: &[f32], in_channels: usize, out_channels: usize) -> Vec<f32> {
    if in_channels == out_channels {
        return input.to_vec();
    }
    let mut out = Vec::with_capacity(input.len() / in_channels * out_channels);
    for frame in input.chunks_exact(in_channels) {
        for channel in 0..out_channels {
            out.push(if in_channels == 1 { frame[0] } else if channel < in_channels { frame[channel] } else { 0.0 });
        }
    }
    out
}
