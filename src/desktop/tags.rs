//! A file's own title, artist, album and embedded cover, for the system media controls when the
//! app didn't pass them (the same fallback as on Android).

use std::fs::File;
use std::path::Path;

use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey};
use symphonia::core::probe::Hint;

#[derive(Debug, Default, Clone)]
pub struct Tags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// The embedded cover: its bytes and media type (image/jpeg, image/png).
    pub cover: Option<(Vec<u8>, String)>,
}

pub fn read(path: &Path) -> Tags {
    let Ok(file) = File::open(path) else { return Tags::default() };
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let Ok(mut probed) =
        symphonia::default::get_probe().format(&hint, stream, &FormatOptions::default(), &MetadataOptions::default())
    else {
        return Tags::default();
    };
    let mut tags = Tags::default();
    // Tags before the audio (ID3v2) come with the probe; the container's own (MP4, FLAC, Vorbis) with the format.
    if let Some(revision) = probed.metadata.get().as_ref().and_then(|m| m.current().cloned()) {
        merge(&mut tags, &revision);
    }
    if let Some(revision) = probed.format.metadata().current() {
        merge(&mut tags, revision);
    }
    tags
}

fn merge(tags: &mut Tags, revision: &MetadataRevision) {
    for tag in revision.tags() {
        let value = tag.value.to_string().trim().to_string();
        if value.is_empty() {
            continue;
        }
        let slot = match tag.std_key {
            Some(StandardTagKey::TrackTitle) => &mut tags.title,
            Some(StandardTagKey::Artist) => &mut tags.artist,
            Some(StandardTagKey::AlbumArtist) if tags.artist.is_none() => &mut tags.artist,
            Some(StandardTagKey::Album) => &mut tags.album,
            _ => continue,
        };
        slot.get_or_insert(value);
    }
    if tags.cover.is_none() {
        // The front cover if it's marked, otherwise the first picture.
        let visuals = revision.visuals();
        let front = visuals
            .iter()
            .find(|v| v.usage == Some(symphonia::core::meta::StandardVisualKey::FrontCover))
            .or_else(|| visuals.first());
        if let Some(visual) = front {
            tags.cover = Some((visual.data.to_vec(), visual.media_type.clone()));
        }
    }
}
