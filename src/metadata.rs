use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::tag::{ItemKey, Tag, TagExt};
use std::collections::BTreeMap;
use std::path::Path;

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

pub fn write_tags(path: &Path, fields: &BTreeMap<String, String>) -> bool {
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
