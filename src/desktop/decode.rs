//! Reading one audio file: decoding with symphonia, with gapless trimming of the encoder's delay and
//! padding (symphonia's own for MP3, ours for AAC in MP4).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::{Time, TimeBase};

/// One file being decoded.
pub struct TrackDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    pub sample_rate: u32,
    pub channels: usize,
    /// Seconds, when the file says.
    pub duration_secs: Option<f64>,
    /// The codec's short name (mp3, flac, aac, alac, ...).
    pub codec: String,
    time_base: Option<TimeBase>,
    /// Encoder delay of AAC in MP4 (see mp4_gapless_info), in frames.
    delay_frames: u64,
    /// The real length in frames, when the file says (AAC in MP4).
    length_frames: Option<u64>,
    /// Decoded frames still to drop: the encoder delay, or the part before a seek target.
    skip_frames: u64,
    /// Frames still to return before the end (the encoder's padding after them is dropped).
    frames_left: Option<u64>,
}

impl TrackDecoder {
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let stream = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        // enable_gapless: trim the silence encoders add at the start and end (MP3 LAME/Xing info,
        // M4A iTunSMPB / edit lists), so consecutive tracks join without a gap.
        let format_options = FormatOptions { enable_gapless: true, ..Default::default() };
        let probed = symphonia::default::get_probe()
            .format(&hint, stream, &format_options, &MetadataOptions::default())
            .map_err(|e| format!("{}: unsupported or unreadable file ({e})", path.display()))?;
        let format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| format!("{}: no audio track", path.display()))?;
        let params = &track.codec_params;
        let sample_rate = params.sample_rate.ok_or_else(|| format!("{}: unknown sample rate", path.display()))?;
        let channels = params.channels.map(|c| c.count()).unwrap_or(2).max(1);
        let duration_secs = params.n_frames.map(|n| n as f64 / sample_rate as f64);
        let codec = symphonia::default::get_codecs()
            .get_codec(params.codec)
            .map(|d| d.short_name.to_string())
            .unwrap_or_else(|| "unknown".into());
        let decoder = symphonia::default::get_codecs()
            .make(params, &DecoderOptions::default())
            .map_err(|e| format!("{}: can't decode {codec} ({e})", path.display()))?;
        // symphonia trims MP3's encoder delay and padding itself, but not AAC's in MP4: read it here.
        let (delay_frames, length_frames) = if codec == "aac" {
            mp4_gapless_info(path, sample_rate).map_or((0, None), |(delay, length)| (delay, Some(length)))
        } else {
            (0, None)
        };
        let duration_secs = length_frames.map(|n| n as f64 / sample_rate as f64).or(duration_secs);
        Ok(Self {
            track_id: track.id,
            time_base: params.time_base,
            format,
            decoder,
            sample_rate,
            channels,
            duration_secs,
            codec,
            delay_frames,
            length_frames,
            skip_frames: delay_frames,
            frames_left: length_frames,
        })
    }

    /// The next block of interleaved samples, in the file's own rate and channels; None at the end.
    pub fn next_block(&mut self) -> Result<Option<Vec<f32>>, String> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
                // A new stream starts in the same file (chained Ogg): treat as the end of this track.
                Err(SymphoniaError::ResetRequired) => return Ok(None),
                Err(e) => return Err(e.to_string()),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(audio) => {
                    if audio.frames() == 0 {
                        continue;
                    }
                    let mut samples = SampleBuffer::<f32>::new(audio.capacity() as u64, *audio.spec());
                    samples.copy_interleaved_ref(audio);
                    let mut block = samples.samples().to_vec();
                    // Drop the encoder delay / the part before a seek target, and the padding at the end.
                    let frames = (block.len() / self.channels) as u64;
                    let skip = self.skip_frames.min(frames);
                    self.skip_frames -= skip;
                    block.drain(..skip as usize * self.channels);
                    if let Some(left) = self.frames_left.as_mut() {
                        if *left == 0 {
                            return Ok(None);
                        }
                        let keep = (*left).min((block.len() / self.channels) as u64);
                        block.truncate(keep as usize * self.channels);
                        *left -= keep;
                    }
                    if block.is_empty() {
                        continue;
                    }
                    return Ok(Some(block));
                }
                // A damaged packet: skip it, like other players do.
                Err(SymphoniaError::DecodeError(_)) => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
    }

    /// Jump to `secs` into the track, to the sample.
    pub fn seek(&mut self, secs: f64) -> Result<(), String> {
        let rate = self.sample_rate as f64;
        let target = (secs.max(0.0) * rate).round() as u64;
        // The file's own timeline includes the encoder delay.
        let time = Time::from((target + self.delay_frames) as f64 / rate);
        let seeked = self
            .format
            .seek(SeekMode::Accurate, SeekTo::Time { time, track_id: Some(self.track_id) })
            .map_err(|e| e.to_string())?;
        self.decoder.reset();
        // The seek lands on the start of a packet at or before the target: skip up to the target.
        let to_frames = |ts: u64| match self.time_base {
            Some(tb) => (ts as f64 * tb.numer as f64 / tb.denom as f64 * rate).round() as u64,
            None => ts,
        };
        self.skip_frames = to_frames(seeked.required_ts).saturating_sub(to_frames(seeked.actual_ts));
        self.frames_left = self.length_frames.map(|length| length.saturating_sub(target));
        Ok(())
    }
}

/// Gapless info of AAC in an MP4 file: (encoder delay, real length), in frames. AAC encoders add
/// silence before the audio (usually 1024 or 2112 frames) and pad the end to a whole packet; the file
/// says how much in iTunes' `iTunSMPB` tag, or in an edit list (`elst`). Prefers iTunSMPB.
fn mp4_gapless_info(path: &Path, sample_rate: u32) -> Option<(u64, u64)> {
    let moov = read_top_level_box(path, b"moov")?;
    itunes_smpb(&moov).or_else(|| edit_list(&moov, sample_rate))
}

/// The body of the first top-level box of this type.
fn read_top_level_box(path: &Path, wanted: &[u8; 4]) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut pos = 0u64;
    while pos + 8 <= len {
        file.seek(SeekFrom::Start(pos)).ok()?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header).ok()?;
        let mut size = u32::from_be_bytes(header[0..4].try_into().ok()?) as u64;
        let mut header_len = 8;
        if size == 1 {
            let mut large = [0u8; 8];
            file.read_exact(&mut large).ok()?;
            size = u64::from_be_bytes(large);
            header_len = 16;
        } else if size == 0 {
            size = len - pos;
        }
        if size < header_len {
            return None;
        }
        if &header[4..8] == wanted {
            let mut body = vec![0u8; (size - header_len) as usize];
            file.read_exact(&mut body).ok()?;
            return Some(body);
        }
        pos += size;
    }
    None
}

/// The child boxes of a box body: (type, body).
fn boxes(data: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + 8 <= data.len() {
        let size = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let kind: [u8; 4] = data[pos + 4..pos + 8].try_into().unwrap();
        let (size, header) = if size == 1 && pos + 16 <= data.len() {
            (u64::from_be_bytes(data[pos + 8..pos + 16].try_into().unwrap()) as usize, 16)
        } else if size == 0 {
            (data.len() - pos, 8)
        } else {
            (size, 8)
        };
        if size < header || pos + size > data.len() {
            break;
        }
        out.push((kind, &data[pos + header..pos + size]));
        pos += size;
    }
    out
}

fn child<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(data).into_iter().find(|(k, _)| k == kind).map(|(_, body)| body)
}

/// iTunes: moov/udta/meta/ilst/"----" named "iTunSMPB", whose text is hex fields:
/// " 00000000 <delay> <padding> <length> ...".
fn itunes_smpb(moov: &[u8]) -> Option<(u64, u64)> {
    let meta = child(child(moov, b"udta")?, b"meta")?;
    // meta is a "full box" (4 bytes of version and flags) in MP4, but not always in QuickTime files.
    let meta = if meta.get(4..8) == Some(b"hdlr") { meta } else { meta.get(4..)? };
    for (kind, item) in boxes(child(meta, b"ilst")?) {
        if &kind != b"----" {
            continue;
        }
        let name = child(item, b"name").and_then(|n| n.get(4..)).map(String::from_utf8_lossy);
        if name.as_deref() != Some("iTunSMPB") {
            continue;
        }
        let text = String::from_utf8_lossy(child(item, b"data")?.get(8..)?).to_string();
        let fields: Vec<u64> = text.split_whitespace().filter_map(|f| u64::from_str_radix(f, 16).ok()).collect();
        let (delay, length) = (*fields.get(1)?, *fields.get(3)?);
        return (length > 0).then_some((delay, length));
    }
    None
}

/// Edit list: moov/trak/edts/elst of the sound track. Its first entry with a media time says where
/// the audio starts (the delay) and how long it plays (the length), in the movie's and the media's
/// timescales.
fn edit_list(moov: &[u8], sample_rate: u32) -> Option<(u64, u64)> {
    let timescale = |header: &[u8]| -> Option<u64> {
        let at = if header.first() == Some(&1) { 20 } else { 12 };
        Some(u32::from_be_bytes(header.get(at..at + 4)?.try_into().ok()?) as u64)
    };
    let movie_scale = timescale(child(moov, b"mvhd")?)?;
    for (kind, trak) in boxes(moov) {
        if &kind != b"trak" {
            continue;
        }
        let mdia = child(trak, b"mdia")?;
        if child(mdia, b"hdlr").and_then(|h| h.get(8..12)) != Some(b"soun") {
            continue;
        }
        let media_scale = timescale(child(mdia, b"mdhd")?)?;
        let elst = child(child(trak, b"edts")?, b"elst")?;
        let version = *elst.first()?;
        let count = u32::from_be_bytes(elst.get(4..8)?.try_into().ok()?) as usize;
        let entry_len = if version == 1 { 20 } else { 12 };
        for i in 0..count {
            let e = elst.get(8 + i * entry_len..8 + (i + 1) * entry_len)?;
            let (duration, media_time) = if version == 1 {
                (u64::from_be_bytes(e[0..8].try_into().ok()?), i64::from_be_bytes(e[8..16].try_into().ok()?))
            } else {
                (u32::from_be_bytes(e[0..4].try_into().ok()?) as u64, i32::from_be_bytes(e[4..8].try_into().ok()?) as i64)
            };
            if media_time < 0 {
                continue; // an empty edit
            }
            let rate = sample_rate as u64;
            let delay = media_time as u64 * rate / media_scale.max(1);
            let length = duration * rate / movie_scale.max(1);
            return (length > 0).then_some((delay, length));
        }
        return None;
    }
    None
}

