//! Test program for the desktop player engine.
//!
//!   cargo run --release --example desktop_play -- play <file> [<file> ...]
//!       Plays the files in order through the default output device. Prints the position every
//!       second and, at each track change, how many output frames were silence because nothing was
//!       decoded in time (0 = no gap).
//!
//!   cargo run --release --example desktop_play -- check <file> [<file> ...]
//!       For each file: its codec, rate, channels, length, and how much silence it starts and ends
//!       with after gapless trimming. Then decodes all of them back to back the way the engine does
//!       and confirms the result equals the files decoded one by one, so nothing is added or lost
//!       at a switch.
//!
//!   cargo run --release --example desktop_play -- raw <out.f32> <rate> <channels> <file> [<file> ...]
//!       Runs the files through the engine's pipeline (decoding, gapless trimming, conversion to
//!       <rate> Hz and <channels> channels, the switches between tracks) and writes the output
//!       samples (interleaved 32-bit floats, little endian) to <out.f32>, to compare in other tools.

#[cfg(not(all(feature = "desktop", not(any(target_os = "android", target_os = "ios")))))]
fn main() {
    eprintln!("Build with the `desktop` feature on a desktop OS.");
}

#[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
fn main() {
    use std::path::PathBuf;
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_default();
    let raw_target = if mode == "raw" {
        let out = args.next().map(PathBuf::from);
        let rate = args.next().and_then(|r| r.parse::<u32>().ok());
        let channels = args.next().and_then(|c| c.parse::<usize>().ok());
        Some((out, rate, channels))
    } else {
        None
    };
    let files: Vec<PathBuf> = args.map(PathBuf::from).collect();
    if files.is_empty() || !matches!(mode.as_str(), "play" | "check" | "raw") {
        eprintln!("usage: desktop_play play|check <file> [<file> ...]  |  desktop_play raw <out.f32> <rate> <channels> <file> [<file> ...]");
        std::process::exit(2);
    }
    let result = match mode.as_str() {
        "play" => play(files),
        "check" => check(files),
        _ => match raw_target {
            Some((Some(out), Some(rate), Some(channels))) => raw(out, rate, channels, files),
            _ => Err("raw needs <out.f32> <rate> <channels>".into()),
        },
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

/// The files in order, keyed by their index: the first one, and what follows each.
#[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
fn in_order(files: &[std::path::PathBuf]) -> (tauri_plugin_native_audio::desktop::pipeline::Track, tauri_plugin_native_audio::desktop::pipeline::NextTrack) {
    use tauri_plugin_native_audio::desktop::pipeline::Track;
    let track = |key: u64| files.get(key as usize).map(|path| Track { key, path: path.clone() });
    let first = track(0).expect("at least one file");
    let files = files.to_vec();
    let next = move |key: u64| files.get(key as usize + 1).map(|path| Track { key: key + 1, path: path.clone() });
    (first, std::sync::Arc::new(next))
}

#[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
fn play(files: Vec<std::path::PathBuf>) -> Result<(), String> {
    use std::time::Duration;
    use tauri_plugin_native_audio::desktop::engine::Engine;

    let (first, next) = in_order(&files);
    let engine = Engine::new(next, None)?;
    println!("output: {} Hz, {} channels", engine.sample_rate, engine.channels);
    // NATIVE_AUDIO_TEST_VOLUME=0 plays silently (the output still runs, so gaps are still counted).
    if let Some(volume) = std::env::var("NATIVE_AUDIO_TEST_VOLUME").ok().and_then(|v| v.parse::<f32>().ok()) {
        engine.set_volume(volume);
    }
    engine.start(first, 0.0);
    engine.play();
    let mut last_index = None;
    let mut underruns_at_change = 0;
    loop {
        std::thread::sleep(Duration::from_millis(250));
        let status = engine.status();
        if let Some(e) = &status.error {
            println!("  ! {e}");
        }
        if status.key != last_index {
            if let Some(index) = status.key.map(|k| k as usize) {
                let name = files[index].file_name().unwrap_or_default().to_string_lossy();
                println!(
                    "track {}: {name} ({:.0} s) | silence frames so far: {}{}",
                    index + 1,
                    status.duration_secs.unwrap_or(0.0),
                    status.underrun_frames,
                    if last_index.is_some() {
                        format!(" ({} new at this change)", status.underrun_frames - underruns_at_change)
                    } else {
                        String::new()
                    }
                );
            }
            underruns_at_change = status.underrun_frames;
            last_index = status.key;
        }
        if status.ended {
            println!("done | silence frames in total: {}", status.underrun_frames);
            return Ok(());
        }
    }
}

#[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
fn check(files: Vec<std::path::PathBuf>) -> Result<(), String> {
    use tauri_plugin_native_audio::desktop::decode::TrackDecoder;

    let mut separately: Vec<Vec<f32>> = Vec::new();
    for path in &files {
        let mut decoder = TrackDecoder::open(path)?;
        let (rate, channels, codec) = (decoder.sample_rate, decoder.channels, decoder.codec.clone());
        let mut samples = Vec::new();
        while let Some(block) = decoder.next_block()? {
            samples.extend(block);
        }
        let frames = samples.len() / channels;
        let silent = |frame: &[f32]| frame.iter().all(|s| s.abs() < 1e-4);
        let lead = samples.chunks(channels).take_while(|f| silent(f)).count();
        let tail = samples.chunks(channels).rev().take_while(|f| silent(f)).count();
        println!(
            "{}: {codec}, {rate} Hz, {channels} ch, {:.2} s | silence at start {:.1} ms, at end {:.1} ms",
            path.file_name().unwrap_or_default().to_string_lossy(),
            frames as f64 / rate as f64,
            lead as f64 * 1000.0 / rate as f64,
            tail as f64 * 1000.0 / rate as f64,
        );
        separately.push(samples);
    }

    // The engine's way: one decoder after another into one stream, nothing in between.
    let mut joined = Vec::new();
    let mut boundaries = Vec::new();
    for path in &files {
        let mut decoder = TrackDecoder::open(path)?;
        boundaries.push(joined.len());
        while let Some(block) = decoder.next_block()? {
            joined.extend(block);
        }
    }
    let expected: Vec<f32> = separately.concat();
    if joined == expected {
        println!("back to back: identical to the files decoded one by one ({} samples, {} switches)", joined.len(), boundaries.len() - 1);
        Ok(())
    } else {
        Err(format!("back to back differs: {} samples vs {}", joined.len(), expected.len()))
    }
}

#[cfg(all(feature = "desktop", not(any(target_os = "android", target_os = "ios"))))]
fn raw(out: std::path::PathBuf, rate: u32, channels: usize, files: Vec<std::path::PathBuf>) -> Result<(), String> {
    use std::io::Write;
    use tauri_plugin_native_audio::desktop::pipeline::{Pipeline, Step};

    let mut writer = std::io::BufWriter::new(std::fs::File::create(&out).map_err(|e| e.to_string())?);
    let (first, next) = in_order(&files);
    let mut pipeline = Pipeline::new(first, 0.0, next, rate, channels, 0);
    let mut frames = 0u64;
    loop {
        match pipeline.next_step() {
            Step::Samples(samples) => {
                frames += (samples.len() / channels) as u64;
                for sample in samples {
                    writer.write_all(&sample.to_le_bytes()).map_err(|e| e.to_string())?;
                }
            }
            Step::TrackStart(start) => println!(
                "track {} starts at output frame {} (written so far: {frames})",
                start.track.key + 1,
                start.output_frame
            ),
            Step::Error(e) => println!("  ! {e}"),
            Step::End => break,
        }
    }
    println!("{frames} frames, {rate} Hz, {channels} ch -> {}", out.display());
    Ok(())
}
