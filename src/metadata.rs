//Copyright 2026 Christopher Dickinson
//
// Licensed under the Apache License, Version 2.0 (the "License");
//you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::tag::{ItemKey, Tag, TagExt};
use std::collections::BTreeMap;
use std::path::Path;

const MAX_FIELD_BYTES: usize = 1024 * 1024;

pub const EDITABLE_FIELDS: &[&str] =
    &["title", "artist", "album", "albumartist", "date", "genre", "tracknumber"];

fn item_key(field: &str) -> Option<ItemKey> {
    Some(match field {
        "title" => ItemKey::TrackTitle,
        "artist" => ItemKey::TrackArtist,
        "album" => ItemKey::AlbumTitle,
        "albumartist" => ItemKey::AlbumArtist,
        "date" => ItemKey::RecordingDate,
        "genre" => ItemKey::Genre,
        "tracknumber" => ItemKey::TrackNumber,
        _ => return None,
    })
}

pub fn read_tags(path: &Path) -> BTreeMap<String, String> {
    if crate::library::is_midi(path) {
        return read_midi_tags(path);
    }
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for f in EDITABLE_FIELDS {
        out.insert((*f).to_string(), String::new());
    }
    let tagged = match lofty::read_from_path(path) {
        Ok(t) => t,
        Err(_) => return out,
    };
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    if let Some(tag) = tag {
        for f in EDITABLE_FIELDS {
            if let Some(key) = item_key(f) {
                if let Some(val) = tag.get_string(&key) {
                    out.insert((*f).to_string(), val.to_string());
                }
            }
        }
    }
    out
}

fn read_midi_tags(path: &Path) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for f in EDITABLE_FIELDS {
        out.insert((*f).to_string(), String::new());
    }
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => return out,
    };
    if data.len() > 50 * 1024 * 1024 {
        return out;
    }
    let smf = match midly::Smf::parse(&data) {
        Ok(s) => s,
        Err(_) => return out,
    };
    let format_num: u8 = match smf.header.format {
        midly::Format::SingleTrack => 0,
        midly::Format::Parallel => 1,
        midly::Format::Sequential => 2,
    };
    out.insert("format".to_string(), format_num.to_string());
    out.insert("tracks".to_string(), smf.tracks.len().to_string());
    let mut first_title: Option<String> = None;
    let mut copyright: Option<String> = None;
    let mut first_tempo: Option<u32> = None;
    let mut first_time_sig: Option<String> = None;
    let mut first_key_sig: Option<String> = None;
    let mut instruments: Vec<String> = Vec::new();
    for track in &smf.tracks {
        for event in track {
            match &event.kind {
                midly::TrackEventKind::Meta(msg) => match msg {
                    midly::MetaMessage::TrackName(bytes) => {
                        if first_title.is_none() && bytes.len() <= MAX_FIELD_BYTES {
                            if let Ok(s) = std::str::from_utf8(bytes) {
                                let s = s.trim();
                                if !s.is_empty() {
                                    first_title = Some(s.to_string());
                                }
                            }
                        }
                    }
                    midly::MetaMessage::InstrumentName(bytes) => {
                        if bytes.len() <= MAX_FIELD_BYTES {
                            if let Ok(s) = std::str::from_utf8(bytes) {
                                let s = s.trim();
                                if !s.is_empty() {
                                    instruments.push(s.to_string());
                                }
                            }
                        }
                    }
                    midly::MetaMessage::Copyright(bytes) => {
                        if copyright.is_none() && bytes.len() <= MAX_FIELD_BYTES {
                            if let Ok(s) = std::str::from_utf8(bytes) {
                                let s = s.trim();
                                if !s.is_empty() {
                                    copyright = Some(s.to_string());
                                }
                            }
                        }
                    }
                    midly::MetaMessage::Tempo(t) => {
                        if first_tempo.is_none() {
                            let usec = t.as_int();
                            if usec > 0 {
                                first_tempo = Some(60_000_000u32 / usec);
                            }
                        }
                    }
                    midly::MetaMessage::TimeSignature(num, denom_pow, _, _) => {
                        if first_time_sig.is_none() {
                            let denom = 1u32.checked_shl(*denom_pow as u32).unwrap_or(1);
                            first_time_sig = Some(format!("{num}/{denom}"));
                        }
                    }
                    midly::MetaMessage::KeySignature(sharps, minor) => {
                        if first_key_sig.is_none() {
                            first_key_sig = Some(key_name(*sharps, *minor));
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
    if let Some(t) = first_title {
        out.insert("title".to_string(), t);
    }
    if let Some(c) = copyright {
        out.insert("comment".to_string(), c);
    }
    if let Some(bpm) = first_tempo {
        out.insert("bpm".to_string(), bpm.to_string());
    }
    if let Some(ts) = first_time_sig {
        out.insert("time_sig".to_string(), ts);
    }
    if let Some(ks) = first_key_sig {
        out.insert("key".to_string(), ks);
    }
    if !instruments.is_empty() {
        out.insert("instruments".to_string(), instruments.join(", "));
    }
    out
}

fn key_name(sharps: i8, minor: bool) -> String {
    static MAJOR_SHARP: &[&str] = &["C", "G", "D", "A", "E", "B", "F#", "C#"];
    static MAJOR_FLAT: &[&str] = &["C", "F", "Bb", "Eb", "Ab", "Db", "Gb", "Cb"];
    static MINOR_SHARP: &[&str] = &["A", "E", "B", "F#", "C#", "G#", "D#", "A#"];
    static MINOR_FLAT: &[&str] = &["A", "D", "G", "C", "F", "Bb", "Eb", "Ab"];
    let (table, idx) = match (sharps >= 0, minor) {
        (true, false) => (MAJOR_SHARP, sharps as usize),
        (false, false) => (MAJOR_FLAT, sharps.unsigned_abs() as usize),
        (true, true) => (MINOR_SHARP, sharps as usize),
        (false, true) => (MINOR_FLAT, sharps.unsigned_abs() as usize),
    };
    let name = table.get(idx).copied().unwrap_or("?");
    if minor { format!("{name} min") } else { format!("{name} maj") }
}

pub fn display_title(path: &Path, tags: &BTreeMap<String, String>) -> String {
    let title = tags.get("title").map(|s| s.trim()).unwrap_or("");
    let artist = tags.get("artist").map(|s| s.trim()).unwrap_or("");
    if !title.is_empty() && !artist.is_empty() {
        return format!("{artist} — {title}");
    }
    if !title.is_empty() {
        return title.to_string();
    }
    path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
}

fn write_midi_tags(path: &Path, fields: &BTreeMap<String, String>) -> bool {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => return false,
    };
    if data.len() > 50 * 1024 * 1024 {
        return false;
    }
    let new_title: Vec<u8> = fields
        .get("title")
        .map(|s| s.trim().as_bytes().to_vec())
        .unwrap_or_default();
    let smf = match midly::Smf::parse(&data) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let mut tracks: Vec<Vec<midly::TrackEvent<'_>>> = Vec::new();
    for (ti, track) in smf.tracks.iter().enumerate() {
        let mut events: Vec<midly::TrackEvent<'_>> = Vec::new();
        let mut name_written = false;
        for event in track {
            if ti == 0 {
                if let midly::TrackEventKind::Meta(midly::MetaMessage::TrackName(_)) = &event.kind {
                    if !new_title.is_empty() && !name_written {
                        events.push(midly::TrackEvent {
                            delta: event.delta,
                            kind: midly::TrackEventKind::Meta(
                                midly::MetaMessage::TrackName(&new_title),
                            ),
                        });
                        name_written = true;
                    }
                    continue;
                }
            }
            events.push(*event);
        }
        if ti == 0 && !name_written && !new_title.is_empty() {
            events.insert(
                0,
                midly::TrackEvent {
                    delta: midly::num::u28::from(0u32),
                    kind: midly::TrackEventKind::Meta(
                        midly::MetaMessage::TrackName(&new_title),
                    ),
                },
            );
        }
        tracks.push(events);
    }
    let new_smf = midly::Smf { header: smf.header, tracks };
    let mut out: Vec<u8> = Vec::new();
    if new_smf.write_std(&mut out).is_err() {
        return false;
    }
    std::fs::write(path, &out).is_ok()
}

pub fn write_tags(path: &Path, fields: &BTreeMap<String, String>) -> bool {
    if crate::library::is_midi(path) {
        return write_midi_tags(path, fields);
    }
    let mut tagged = match lofty::read_from_path(path) {
        Ok(t) => t,
        Err(_) => return false,
    };
    if tagged.primary_tag_mut().is_none() {
        let tag_type = tagged.primary_tag_type();
        tagged.insert_tag(Tag::new(tag_type));
    }
    let tag = match tagged.primary_tag_mut() {
        Some(t) => t,
        None => return false,
    };
    for (key, value) in fields {
        let item_key = match item_key(key) {
            Some(k) => k,
            None => continue,
        };
        let value = value.trim();
        if value.is_empty() {
            tag.remove_key(&item_key);
        } else {
            tag.insert_text(item_key, value.to_string());
        }
    }
    tag.save_to_path(path, WriteOptions::default()).is_ok()
}
