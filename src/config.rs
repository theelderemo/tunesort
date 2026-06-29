use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

pub const NUM_QUICKSLOTS: usize = 9;

pub const DEFAULT_QUICKSLOT_COUNT: usize = 5;

pub const DEFAULT_MAX_RECENT: usize = 10;

pub fn actions() -> Vec<(&'static str, &'static str)> {
    vec![
        ("play_pause", "Play / pause"),
        ("next", "Next track"),
        ("prev", "Previous track"),
        ("volume_up", "Volume up one step"),
        ("volume_down", "Volume down one step"),
        ("mute", "Toggle mute"),
        ("seek_forward", "Seek forward"),
        ("seek_back", "Seek backward"),
        ("delete", "Delete current (to trash)"),
        ("shuffle", "Shuffle library"),
        ("undo", "Undo last delete/move"),
        ("open_library", "Open library folder"),
        ("toggle_settings", "Open / close settings"),
        ("toggle_visualizer", "Toggle visualizer"),
        ("toggle_metadata", "Toggle metadata editor"),
        ("quickslot_1", "Quick-move to slot 1"),
        ("quickslot_2", "Quick-move to slot 2"),
        ("quickslot_3", "Quick-move to slot 3"),
        ("quickslot_4", "Quick-move to slot 4"),
        ("quickslot_5", "Quick-move to slot 5"),
        ("quickslot_6", "Quick-move to slot 6"),
        ("quickslot_7", "Quick-move to slot 7"),
        ("quickslot_8", "Quick-move to slot 8"),
        ("quickslot_9", "Quick-move to slot 9"),
        ("set_slot_1", "Assign current folder to slot 1"),
        ("set_slot_2", "Assign current folder to slot 2"),
        ("set_slot_3", "Assign current folder to slot 3"),
        ("set_slot_4", "Assign current folder to slot 4"),
        ("set_slot_5", "Assign current folder to slot 5"),
        ("set_slot_6", "Assign current folder to slot 6"),
        ("set_slot_7", "Assign current folder to slot 7"),
        ("set_slot_8", "Assign current folder to slot 8"),
        ("set_slot_9", "Assign current folder to slot 9"),
    ]
}

pub fn config_dir() -> PathBuf {
    if let Ok(base) = std::env::var("XDG_CONFIG_HOME") {
        if !base.is_empty() {
            return PathBuf::from(base).join("tunesort");
        }
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".config").join("tunesort")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn default_config() -> Value {
    json!({
        "library_path": "",
        "settings": {
            "auto_advance": true,
            "metadata_editing": false,
            "shuffle_on_load": false,
            "recurse_subfolders": true,
            "volume": 0.8,
            "volume_step": 0.05,
            "seek_step": 5,
            "visualizer": false,
            "visualizer_theme": "amber",
            "advance_after_action": true,
            "quickslot_count": 5,
            "max_recent_destinations": 10
        },

        "theme": {
            "bg": "#0a0a0b",
            "surface": "#141316",
            "surface2": "#1c1a1e",
            "border": "#2a272c",
            "text": "#bdb6ab",
            "muted": "#76706a",
            "accent": "#b0884e",
            "danger": "#9c5f4d",
            "ok": "#6f8a63",
            "wave": "#36333a",
            "wave_progress": "#b0884e"
        },
        "quickslots": [
            {"label": "Slot 1", "path": ""},
            {"label": "Slot 2", "path": ""},
            {"label": "Slot 3", "path": ""},
            {"label": "Slot 4", "path": ""},
            {"label": "Slot 5", "path": ""}
        ],

        "recent_destinations": [],

        "keybindings": {
            "Space": "play_pause",
            "KeyK": "play_pause",
            "ArrowRight": "next",
            "ArrowLeft": "prev",
            "ArrowUp": "volume_up",
            "ArrowDown": "volume_down",
            "KeyM": "mute",
            "KeyL": "seek_forward",
            "KeyJ": "seek_back",
            "KeyD": "delete",
            "KeyS": "shuffle",
            "Ctrl+KeyZ": "undo",
            "KeyU": "undo",
            "KeyO": "open_library",
            "Comma": "toggle_settings",
            "KeyV": "toggle_visualizer",
            "KeyE": "toggle_metadata",
            "Numpad1": "quickslot_1",
            "Numpad2": "quickslot_2",
            "Numpad3": "quickslot_3",
            "Numpad4": "quickslot_4",
            "Numpad5": "quickslot_5",
            "Numpad6": "quickslot_6",
            "Numpad7": "quickslot_7",
            "Numpad8": "quickslot_8",
            "Numpad9": "quickslot_9",
            "Digit1": "quickslot_1",
            "Digit2": "quickslot_2",
            "Digit3": "quickslot_3",
            "Digit4": "quickslot_4",
            "Digit5": "quickslot_5",
            "Digit6": "quickslot_6",
            "Digit7": "quickslot_7",
            "Digit8": "quickslot_8",
            "Digit9": "quickslot_9",
            "Ctrl+Numpad1": "set_slot_1",
            "Ctrl+Numpad2": "set_slot_2",
            "Ctrl+Numpad3": "set_slot_3",
            "Ctrl+Numpad4": "set_slot_4",
            "Ctrl+Numpad5": "set_slot_5",
            "Ctrl+Numpad6": "set_slot_6",
            "Ctrl+Numpad7": "set_slot_7",
            "Ctrl+Numpad8": "set_slot_8",
            "Ctrl+Numpad9": "set_slot_9"
        }
    })
}

fn deep_merge(base: &mut Value, override_: &Value) {
    if let (Some(base_map), Some(over_map)) = (base.as_object_mut(), override_.as_object()) {
        for (key, value) in over_map {
            match base_map.get_mut(key) {
                Some(existing) if existing.is_object() && value.is_object() => {
                    deep_merge(existing, value);
                }
                _ => {
                    base_map.insert(key.clone(), value.clone());
                }
            }
        }
    }
}

pub struct Config {
    pub data: Value,
    pub path: PathBuf,
}

impl Config {
    pub fn load() -> Config {
        Config::load_from(&config_path())
    }

    pub fn load_from(path: &Path) -> Config {
        let mut data = default_config();
        if path.exists() {
            match std::fs::read_to_string(path) {
                Ok(text) => match serde_json::from_str::<Value>(&text) {
                    Ok(on_disk) if on_disk.is_object() => deep_merge(&mut data, &on_disk),
                    _ => {
                        let _ = std::fs::rename(path, path.with_extension("json.bad"));
                    }
                },
                Err(_) => {
                    let _ = std::fs::rename(path, path.with_extension("json.bad"));
                }
            }
        }
        Config::normalize(&mut data);
        Config { data, path: path.to_path_buf() }
    }

    pub fn save(&self) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(&self.data) {
            let tmp = self.path.with_extension("json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }
    }

    pub fn replace(&mut self, new_data: Value) {
        let mut merged = default_config();
        deep_merge(&mut merged, &new_data);
        Config::normalize(&mut merged);
        self.data = merged;
        self.save();
    }

    fn normalize(data: &mut Value) {
        let mut defaults = default_config();
        for key in ["settings", "theme", "keybindings"] {
            if !data.get(key).map(|v| v.is_object()).unwrap_or(false) {
                data[key] = defaults[key].take();
            }
        }

        let slots = data
            .get("quickslots")
            .and_then(|s| s.as_array())
            .cloned()
            .unwrap_or_default();
        let mut slots: Vec<Value> = slots;
        while slots.len() < NUM_QUICKSLOTS {
            slots.push(json!({"label": format!("Slot {}", slots.len() + 1), "path": ""}));
        }
        slots.truncate(NUM_QUICKSLOTS);
        data["quickslots"] = Value::Array(slots);

        let recents: Vec<Value> = data
            .get("recent_destinations")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter(|v| v.is_string()).cloned().collect())
            .unwrap_or_default();
        data["recent_destinations"] = Value::Array(recents);
    }

    pub fn settings(&self) -> &Map<String, Value> {
        self.data["settings"].as_object().unwrap()
    }

    pub fn theme(&self) -> &Map<String, Value> {
        self.data["theme"].as_object().unwrap()
    }

    pub fn theme_pairs(&self) -> Vec<(String, String)> {
        self.theme()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
            .collect()
    }

    pub fn quickslots(&self) -> &Vec<Value> {
        self.data["quickslots"].as_array().unwrap()
    }

    pub fn quickslot_count(&self) -> usize {
        (self.get_f64("quickslot_count", DEFAULT_QUICKSLOT_COUNT as f64) as usize)
            .clamp(1, NUM_QUICKSLOTS)
    }

    pub fn set_quickslot_count(&mut self, n: usize) {
        let n = n.clamp(1, NUM_QUICKSLOTS);
        self.set_setting("quickslot_count", json!(n));
    }

    pub fn max_recent(&self) -> usize {
        (self.get_f64("max_recent_destinations", DEFAULT_MAX_RECENT as f64) as usize).min(50)
    }

    pub fn recent_destinations(&self) -> Vec<String> {
        self.data
            .get("recent_destinations")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str()).map(String::from).collect())
            .unwrap_or_default()
    }

    pub fn push_recent_destination(&mut self, path: &str) {
        let max = self.max_recent();
        if path.is_empty() || max == 0 {
            return;
        }
        let mut list = self.recent_destinations();
        list.retain(|p| p != path);
        list.insert(0, path.to_string());
        list.truncate(max);
        self.data["recent_destinations"] = json!(list);
        self.save();
    }

    pub fn clear_recent_destinations(&mut self) {
        self.data["recent_destinations"] = json!([]);
        self.save();
    }

    pub fn quickslot_label(&self, idx: usize) -> String {
        self.quickslots()
            .get(idx)
            .and_then(|s| s.get("label"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("Slot {}", idx + 1))
    }

    pub fn quickslot_path(&self, idx: usize) -> String {
        self.quickslots()
            .get(idx)
            .and_then(|s| s.get("path"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }

    pub fn set_quickslot_label(&mut self, idx: usize, value: &str) {
        self.data["quickslots"][idx]["label"] = json!(value);
        self.save();
    }

    pub fn set_quickslot_path(&mut self, idx: usize, value: &str) {
        self.data["quickslots"][idx]["path"] = json!(value);
        self.save();
    }

    pub fn keybindings(&self) -> &Map<String, Value> {
        self.data["keybindings"].as_object().unwrap()
    }

    pub fn action_for_key(&self, keystr: &str) -> Option<String> {
        self.keybindings()
            .get(keystr)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    pub fn bind_key(&mut self, keystr: &str, action: &str) {
        if let Some(kb) = self.data["keybindings"].as_object_mut() {
            let stale: Vec<String> = kb
                .iter()
                .filter(|(_, v)| v.as_str() == Some(action))
                .map(|(k, _)| k.clone())
                .collect();
            for k in stale {
                kb.remove(&k);
            }
            kb.insert(keystr.to_string(), json!(action));
        }
        self.save();
    }

    pub fn reset_keybindings(&mut self) {
        self.data["keybindings"] = default_config()["keybindings"].clone();
        self.save();
    }

    pub fn get_bool(&self, key: &str, default: bool) -> bool {
        self.settings().get(key).and_then(|v| v.as_bool()).unwrap_or(default)
    }

    pub fn get_f64(&self, key: &str, default: f64) -> f64 {
        self.settings().get(key).and_then(|v| v.as_f64()).unwrap_or(default)
    }

    pub fn get_str(&self, key: &str, default: &str) -> String {
        self.settings()
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or(default)
            .to_string()
    }

    pub fn set_setting(&mut self, key: &str, value: Value) {
        if let Some(s) = self.data["settings"].as_object_mut() {
            s.insert(key.to_string(), value);
        }
        self.save();
    }

    pub fn theme_color(&self, key: &str) -> String {
        self.theme().get(key).and_then(|v| v.as_str()).unwrap_or("#000000").to_string()
    }

    pub fn set_theme_color(&mut self, key: &str, value: &str) {
        if let Some(t) = self.data["theme"].as_object_mut() {
            t.insert(key.to_string(), json!(value));
        }
        self.save();
    }

    pub fn library_path(&self) -> String {
        self.data.get("library_path").and_then(|v| v.as_str()).unwrap_or("").to_string()
    }

    pub fn set_library_path(&mut self, value: &str) {
        self.data["library_path"] = json!(value);
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_repairs_non_object_sections() {
        let mut data = default_config();
        deep_merge(
            &mut data,
            &json!({"settings": 5, "theme": "x", "keybindings": [1, 2]}),
        );
        Config::normalize(&mut data);

        let cfg = Config { data, path: PathBuf::from("/dev/null") };
        assert!(cfg.data["settings"].is_object());
        assert!(cfg.data["theme"].is_object());
        assert!(cfg.data["keybindings"].is_object());
        assert_eq!(cfg.get_f64("volume", 0.0), 0.8);
        assert_eq!(cfg.theme_color("bg"), "#0a0a0b");
        assert_eq!(cfg.action_for_key("Space").as_deref(), Some("play_pause"));
    }

    #[test]
    fn normalize_keeps_valid_sections() {
        let mut data = default_config();
        deep_merge(&mut data, &json!({"settings": {"volume": 0.3}}));
        Config::normalize(&mut data);
        let cfg = Config { data, path: PathBuf::from("/dev/null") };
        assert_eq!(cfg.get_f64("volume", 0.0), 0.3);
    }
}
