use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const AUDIO_EXTS: &[&str] = &[
    "mp3", "flac", "ogg", "oga", "opus", "wav", "wave", "aiff", "aif", "m4a", "m4b", "mp4", "aac",
    "alac", "ape", "wv", "wma", "mpc", "tta", "dsf", "dff", "ac3", "dts", "amr", "au", "ra", "mka",
    "caf", "spx", "3gp", "webm", "mp2", "m4r",
];

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

#[derive(Clone)]
pub enum Kind {
    Delete,
    Move,
}

pub struct UndoEntry {
    pub kind: Kind,
    pub original: PathBuf,
    pub trashed_file: PathBuf,
    pub trashed_info: PathBuf,
    pub dest: PathBuf,
    pub index: usize,
}

#[derive(Default)]
pub struct Library {
    pub tracks: Vec<PathBuf>,
    pub index: i64,
    pub root: PathBuf,
    pub undo_stack: Vec<UndoEntry>,
}

impl Library {
    pub fn new() -> Library {
        Library { tracks: Vec::new(), index: -1, root: PathBuf::new(), undo_stack: Vec::new() }
    }

    pub fn load(&mut self, path: &Path, recurse: bool) -> usize {
        self.root = path.to_path_buf();
        let mut found: Vec<PathBuf> = Vec::new();
        if path.is_dir() {
            if recurse {
                walk(path, &mut found);
            } else if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let full = entry.path();
                    if full.is_file() && is_audio(&full) {
                        found.push(full);
                    }
                }
            }
        }
        found.sort_by_key(|p| p.to_string_lossy().to_lowercase());
        self.index = if found.is_empty() { -1 } else { 0 };
        self.tracks = found;
        self.tracks.len()
    }

    pub fn current(&self) -> Option<&PathBuf> {
        if self.index >= 0 && (self.index as usize) < self.tracks.len() {
            Some(&self.tracks[self.index as usize])
        } else {
            None
        }
    }

    pub fn go_to(&mut self, idx: i64) -> Option<&PathBuf> {
        if idx >= 0 && (idx as usize) < self.tracks.len() {
            self.index = idx;
            self.current()
        } else {
            None
        }
    }

    pub fn next(&mut self) {
        if self.tracks.is_empty() {
            return;
        }
        if (self.index + 1) < self.tracks.len() as i64 {
            self.index += 1;
        } else {
            self.index = 0;
        }
    }

    pub fn prev(&mut self) {
        if self.tracks.is_empty() {
            return;
        }
        if self.index - 1 >= 0 {
            self.index -= 1;
        } else {
            self.index = self.tracks.len() as i64 - 1;
        }
    }

    pub fn shuffle(&mut self, keep_current: bool) {
        use rand::seq::SliceRandom;
        if self.tracks.is_empty() {
            return;
        }
        let current = self.current().cloned();
        let mut rng = rand::thread_rng();
        self.tracks.shuffle(&mut rng);
        if keep_current {
            if let Some(cur) = current {
                if let Some(pos) = self.tracks.iter().position(|p| *p == cur) {
                    self.index = pos as i64;
                    return;
                }
            }
        }
        self.index = 0;
    }

    pub fn delete_current(&mut self) -> Option<&UndoEntry> {
        let path = self.current()?.clone();
        let idx = self.index as usize;
        let before = trash_snapshot(&path);
        if trash::delete(&path).is_err() {
            return None;
        }
        let (trashed_file, trashed_info) = find_trashed(&path, &before);
        let entry = UndoEntry {
            kind: Kind::Delete,
            original: path,
            trashed_file,
            trashed_info,
            dest: PathBuf::new(),
            index: idx,
        };
        self.undo_stack.push(entry);
        self.remove_at(idx);
        self.undo_stack.last()
    }

    pub fn move_current(&mut self, dest_dir: &Path) -> Option<&UndoEntry> {
        if self.index < 0 {
            return None;
        }
        self.move_index(self.index as usize, dest_dir)
    }

    pub fn move_index(&mut self, idx: usize, dest_dir: &Path) -> Option<&UndoEntry> {
        let path = self.tracks.get(idx)?.clone();
        if dest_dir.as_os_str().is_empty() {
            return None;
        }
        if std::fs::create_dir_all(dest_dir).is_err() {
            return None;
        }
        let name = path.file_name()?.to_owned();
        let dest = unique_dest(dest_dir, &name);
        if move_file(&path, &dest).is_err() {
            return None;
        }
        let entry = UndoEntry {
            kind: Kind::Move,
            original: path,
            trashed_file: PathBuf::new(),
            trashed_info: PathBuf::new(),
            dest,
            index: idx,
        };
        self.undo_stack.push(entry);
        self.remove_at(idx);
        self.undo_stack.last()
    }

    pub fn undo(&mut self) -> Option<PathBuf> {
        let entry = self.undo_stack.pop()?;
        let restored = match entry.kind {
            Kind::Delete => {
                if entry.trashed_file.as_os_str().is_empty() || !entry.trashed_file.exists() {
                    return None;
                }
                if let Some(parent) = entry.original.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let mut target = entry.original.clone();
                if target.exists() {
                    if let (Some(dir), Some(name)) = (target.parent(), target.file_name()) {
                        target = unique_dest(dir, name);
                    }
                }
                if move_file(&entry.trashed_file, &target).is_err() {
                    return None;
                }
                if !entry.trashed_info.as_os_str().is_empty() && entry.trashed_info.exists() {
                    let _ = std::fs::remove_file(&entry.trashed_info);
                }
                target
            }
            Kind::Move => {
                if !entry.dest.exists() {
                    return None;
                }
                let mut target = entry.original.clone();
                if target.exists() {
                    if let (Some(dir), Some(name)) = (target.parent(), target.file_name()) {
                        target = unique_dest(dir, name);
                    }
                }
                if let Some(parent) = target.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if move_file(&entry.dest, &target).is_err() {
                    return None;
                }
                target
            }
        };

        let idx = entry.index.min(self.tracks.len());
        self.tracks.insert(idx, restored.clone());
        self.index = idx as i64;
        Some(restored)
    }

    fn remove_at(&mut self, idx: usize) {
        if idx >= self.tracks.len() {
            return;
        }
        self.tracks.remove(idx);
        if self.tracks.is_empty() {
            self.index = -1;
        } else if idx == self.index as usize {
            self.index = (idx.min(self.tracks.len() - 1)) as i64;
        } else if (idx as i64) < self.index {
            self.index -= 1;
        }
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.is_file() && is_audio(&path) {
                out.push(path);
            }
        }
    }
}

fn move_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::fs::copy(src, dst)?;
            std::fs::remove_file(src)
        }
    }
}

fn unique_dest(directory: &Path, name: &std::ffi::OsStr) -> PathBuf {
    let dest = directory.join(name);
    if !dest.exists() {
        return dest;
    }
    let name = Path::new(name);
    let stem = name.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = name.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let mut i = 1;
    loop {
        let candidate = directory.join(format!("{stem} ({i}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
        i += 1;
    }
}

fn trash_files_dirs(for_path: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let xdg = std::env::var("XDG_DATA_HOME").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".local")
            .join("share")
            .to_string_lossy()
            .to_string()
    });
    dirs.push(PathBuf::from(xdg).join("Trash").join("files"));

    let mount = mount_point(for_path);
    let uid = unsafe { libc_getuid() };
    dirs.push(mount.join(".Trash").join(uid.to_string()).join("files"));
    dirs.push(mount.join(format!(".Trash-{uid}")).join("files"));
    dirs
}

unsafe extern "C" {
    fn getuid() -> u32;
}
unsafe fn libc_getuid() -> u32 {
    unsafe { getuid() }
}

fn mount_point(path: &Path) -> PathBuf {
    let mut path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    while !is_mount(&path) {
        match path.parent() {
            Some(parent) if parent != path => path = parent.to_path_buf(),
            _ => break,
        }
    }
    path
}

fn is_mount(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let here = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return false,
    };
    match path.parent() {
        Some(parent) => match std::fs::metadata(parent) {
            Ok(p) => here.dev() != p.dev(),
            Err(_) => true,
        },
        None => true,
    }
}

fn trash_snapshot(for_path: &Path) -> HashSet<PathBuf> {
    let mut snap = HashSet::new();
    for d in trash_files_dirs(for_path) {
        if d.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&d) {
                for entry in entries.flatten() {
                    snap.insert(entry.path());
                }
            }
        }
    }
    snap
}

fn find_trashed(original: &Path, before: &HashSet<PathBuf>) -> (PathBuf, PathBuf) {
    let topdir = mount_point(original);
    let orig_name = original.file_name();
    let mut fallback: Option<(PathBuf, PathBuf)> = None;

    for d in trash_files_dirs(original) {
        if !d.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for entry in entries.flatten() {
            let full = entry.path();
            if before.contains(&full) {
                continue;
            }

            let name = entry.file_name();
            let info = d.parent().map(|p| {
                let mut fname = name.clone();
                fname.push(".trashinfo");
                p.join("info").join(fname)
            });

            if let Some(info) = &info {
                if let Some(orig_in_info) = trashinfo_original_path(info, &topdir) {
                    if paths_equal(&orig_in_info, original) {
                        return (full, info.clone());
                    }
                }
            }

            if fallback.is_none() && name.as_os_str() == orig_name.unwrap_or_default() {
                let info = info.filter(|p| p.exists()).unwrap_or_default();
                fallback = Some((full, info));
            }
        }
    }
    fallback.unwrap_or_default()
}

fn trashinfo_original_path(info: &Path, topdir: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(info).ok()?;
    let raw = text
        .lines()
        .find_map(|l| l.strip_prefix("Path="))?
        .trim();
    let decoded = percent_decode(raw);
    let path = PathBuf::from(decoded);
    if path.is_absolute() {
        Some(path)
    } else {
        Some(topdir.join(path))
    }
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tunesort-test-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn percent_decode_handles_escapes() {
        assert_eq!(percent_decode("/music/a%20b.mp3"), "/music/a b.mp3");

        assert_eq!(percent_decode("/music/cafe%CC%81.mp3"), "/music/cafe\u{301}.mp3");

        assert_eq!(percent_decode("/100%/x.mp3"), "/100%/x.mp3");
        assert_eq!(percent_decode("plain.flac"), "plain.flac");
    }

    #[test]
    fn trashinfo_path_absolute_and_relative() {
        let dir = scratch_dir("trashinfo");
        let info = dir.join("song.mp3.trashinfo");
        std::fs::write(
            &info,
            "[Trash Info]\nPath=/lib/My%20Song.mp3\nDeletionDate=2026-01-01T00:00:00\n",
        )
        .unwrap();
        assert_eq!(
            trashinfo_original_path(&info, Path::new("/mnt")),
            Some(PathBuf::from("/lib/My Song.mp3"))
        );

        std::fs::write(&info, "[Trash Info]\nPath=sub/track.flac\n").unwrap();
        assert_eq!(
            trashinfo_original_path(&info, Path::new("/mnt/disk")),
            Some(PathBuf::from("/mnt/disk/sub/track.flac"))
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn find_trashed_ignores_unrelated_concurrent_entry() {
        let home = scratch_dir("find");
        let files = home.join("Trash").join("files");
        let info = home.join("Trash").join("info");
        std::fs::create_dir_all(&files).unwrap();
        std::fs::create_dir_all(&info).unwrap();

        std::fs::write(files.join("aaa-decoy.mp3"), b"x").unwrap();
        std::fs::write(
            info.join("aaa-decoy.mp3.trashinfo"),
            "[Trash Info]\nPath=/somewhere/else/decoy.mp3\n",
        )
        .unwrap();

        std::fs::write(files.join("song.mp3"), b"y").unwrap();
        std::fs::write(
            info.join("song.mp3.trashinfo"),
            "[Trash Info]\nPath=/lib/song.mp3\n",
        )
        .unwrap();

        let original = PathBuf::from("/lib/song.mp3");
        let before = HashSet::new();

        let prev = std::env::var_os("XDG_DATA_HOME");
        std::env::set_var("XDG_DATA_HOME", &home);
        let (trashed_file, trashed_info) = find_trashed(&original, &before);
        match prev {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }

        assert_eq!(trashed_file, files.join("song.mp3"));
        assert_eq!(trashed_info, info.join("song.mp3.trashinfo"));
        std::fs::remove_dir_all(&home).ok();
    }
}
