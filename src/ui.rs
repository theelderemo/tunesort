use adw::prelude::*;
use gtk::{cairo, gdk, gio, glib, pango};

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::config::{self, Config, NUM_QUICKSLOTS};
use crate::library::Library;
use crate::metadata;
use crate::player::{Player, PlayerEvent};

const REPEATABLE: &[&str] =
    &["volume_up", "volume_down", "seek_forward", "seek_back", "next", "prev"];

const VIS_THEMES: &[&str] = &["amber", "ember", "mono", "forest", "ice"];

fn vis_theme_colors(name: &str) -> [&'static str; 3] {
    match name {
        "ember" => ["#5a2a22", "#9c5f4d", "#cf8a5a"],
        "mono" => ["#3a3a3f", "#6e6a63", "#bdb6ab"],
        "forest" => ["#2c3a2a", "#566b4a", "#8fae74"],
        "ice" => ["#2a3a40", "#4a6b73", "#8fb6bd"],
        _ => ["#7a5a2a", "#b0884e", "#d8b070"],
    }
}

fn fmt_time(seconds: f64) -> String {
    let s = seconds.max(0.0) as i64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn parse_hex(hex: &str) -> (f64, f64, f64) {
    let h = hex.trim_start_matches('#');
    if h.len() >= 6 {
        let r = u8::from_str_radix(&h[0..2], 16).unwrap_or(0) as f64 / 255.0;
        let g = u8::from_str_radix(&h[2..4], 16).unwrap_or(0) as f64 / 255.0;
        let b = u8::from_str_radix(&h[4..6], 16).unwrap_or(0) as f64 / 255.0;
        (r, g, b)
    } else {
        (0.0, 0.0, 0.0)
    }
}

struct Row {
    idx: usize,
    title: String,
}

struct SlotWidgets {
    card: gtk::Box,
    label: gtk::Label,
    path: gtk::Label,
}

struct App {
    config: RefCell<Config>,
    lib: RefCell<Library>,
    player: Player,

    is_playing: Cell<bool>,
    muted: Cell<bool>,
    duration: Cell<f64>,
    position: Cell<f64>,
    volume: Cell<f64>,
    current_tags: RefCell<BTreeMap<String, String>>,
    capture_action: RefCell<Option<String>>,
    peaks: RefCell<Vec<f32>>,
    spectrum: RefCell<Vec<f32>>,
    held: RefCell<HashSet<String>>,
    updating_slider: Cell<bool>,

    window: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    css: gtk::CssProvider,

    lib_label: gtk::Label,

    title: gtk::Label,
    subtitle: gtk::Label,
    waveform: gtk::DrawingArea,
    visualizer: gtk::DrawingArea,
    time_label: gtk::Label,
    play_btn: gtk::Button,
    undo_btn: gtk::Button,
    vol_slider: gtk::Scale,
    vol_label: gtk::Label,
    slots: RefCell<Vec<SlotWidgets>>,
    slots_row: gtk::Box,
    meta_panel: gtk::Box,
    meta_inputs: RefCell<Vec<(String, gtk::Entry)>>,

    track_store: gio::ListStore,
    track_selection: gtk::SingleSelection,

    settings_win: RefCell<Option<adw::Window>>,
    keys_container: RefCell<Option<gtk::Box>>,
}

pub fn build(app: &adw::Application) {
    let (tx, rx) = async_channel::unbounded::<PlayerEvent>();
    let player = Player::new(tx);
    let config = Config::load();
    let volume = config.get_f64("volume", 0.8);

    let brand = gtk::Label::new(Some("tunesort"));
    brand.add_css_class("ts-accent");
    brand.add_css_class("ts-brand");
    let dot = gtk::Label::new(Some("·"));
    dot.add_css_class("ts-muted");
    let lib_label = gtk::Label::new(Some("No library loaded"));
    lib_label.add_css_class("ts-muted");
    lib_label.set_ellipsize(pango::EllipsizeMode::Middle);
    lib_label.set_max_width_chars(60);
    let brand_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    brand_box.append(&brand);
    brand_box.append(&dot);
    brand_box.append(&lib_label);

    let open_btn = button_with_label("Open library", "folder-open-symbolic");
    open_btn.add_css_class("flat");
    let settings_btn = gtk::Button::from_icon_name("emblem-system-symbolic");
    settings_btn.add_css_class("flat");

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    header.pack_start(&brand_box);
    header.pack_end(&settings_btn);
    header.pack_end(&open_btn);

    let title = gtk::Label::new(Some("No track"));
    title.add_css_class("ts-h1");
    title.add_css_class("ts-title");
    title.set_xalign(0.0);
    title.set_ellipsize(pango::EllipsizeMode::End);
    let subtitle = gtk::Label::new(Some(""));
    subtitle.add_css_class("ts-muted");
    subtitle.set_xalign(0.0);
    subtitle.set_ellipsize(pango::EllipsizeMode::End);

    let waveform = gtk::DrawingArea::new();
    waveform.set_content_height(96);
    waveform.set_hexpand(true);
    waveform.add_css_class("ts-waveform");

    let visualizer = gtk::DrawingArea::new();
    visualizer.set_content_height(90);
    visualizer.set_hexpand(true);
    visualizer.set_visible(false);

    let time_label = gtk::Label::new(Some("0:00 / 0:00"));
    time_label.add_css_class("ts-muted");
    time_label.set_xalign(0.0);
    time_label.set_width_chars(11);

    let prev_btn = icon_button("media-skip-backward-symbolic", &["flat", "circular"]);
    let play_btn = icon_button("media-playback-start-symbolic", &["ts-play", "circular"]);
    let next_btn = icon_button("media-skip-forward-symbolic", &["flat", "circular"]);
    let shuffle_btn = icon_button("media-playlist-shuffle-symbolic", &["flat", "circular"]);
    shuffle_btn.set_tooltip_text(Some("Shuffle (S)"));
    let undo_btn = icon_button("edit-undo-symbolic", &["flat", "circular"]);
    undo_btn.set_tooltip_text(Some("Undo last delete/move (U)"));
    let delete_btn = icon_button("user-trash-symbolic", &["flat", "circular", "ts-danger"]);
    delete_btn.set_tooltip_text(Some("Delete → trash (D)"));

    let transport = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    transport.append(&time_label);
    transport.append(&expanding_spacer());
    transport.append(&prev_btn);
    transport.append(&play_btn);
    transport.append(&next_btn);
    transport.append(&expanding_spacer());
    transport.append(&shuffle_btn);
    transport.append(&undo_btn);
    transport.append(&delete_btn);

    let vol_down = gtk::Image::from_icon_name("audio-volume-low-symbolic");
    vol_down.add_css_class("ts-muted");
    let vol_up = gtk::Image::from_icon_name("audio-volume-high-symbolic");
    vol_up.add_css_class("ts-muted");
    let vol_slider = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
    vol_slider.set_value(volume);
    vol_slider.set_draw_value(false);
    vol_slider.set_hexpand(true);
    let vol_label = gtk::Label::new(Some(&format!("{}%", (volume * 100.0) as i32)));
    vol_label.add_css_class("ts-muted");
    vol_label.set_width_chars(5);
    let vol_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    vol_row.append(&vol_down);
    vol_row.append(&vol_slider);
    vol_row.append(&vol_up);
    vol_row.append(&vol_label);

    let slots_title = small_caps_label("Quick-move slots");
    let slots_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    slots_row.set_homogeneous(true);

    let meta_panel = gtk::Box::new(gtk::Orientation::Vertical, 8);
    meta_panel.add_css_class("ts-panel");
    meta_panel.set_margin_top(4);
    set_padding(&meta_panel, 12);
    let meta_header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    meta_header.append(&small_caps_label("Metadata"));
    meta_header.append(&expanding_spacer());
    let save_tags_btn = button_with_label("Save tags", "document-save-symbolic");
    save_tags_btn.add_css_class("flat");
    meta_header.append(&save_tags_btn);
    meta_panel.append(&meta_header);
    let meta_grid = gtk::Grid::new();
    meta_grid.set_row_spacing(8);
    meta_grid.set_column_spacing(8);
    meta_grid.set_column_homogeneous(true);
    let mut meta_inputs: Vec<(String, gtk::Entry)> = Vec::new();
    for (i, field) in metadata::EDITABLE_FIELDS.iter().enumerate() {
        let cell = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let lbl = gtk::Label::new(Some(field));
        lbl.add_css_class("ts-muted");
        lbl.add_css_class("ts-small");
        lbl.set_xalign(0.0);
        let entry = gtk::Entry::new();
        entry.set_hexpand(true);
        cell.append(&lbl);
        cell.append(&entry);
        meta_grid.attach(&cell, (i % 2) as i32, (i / 2) as i32, 1, 1);
        meta_inputs.push(((*field).to_string(), entry));
    }
    meta_panel.append(&meta_grid);

    let player_col = gtk::Box::new(gtk::Orientation::Vertical, 12);
    player_col.add_css_class("ts-card");
    set_padding(&player_col, 16);
    player_col.set_hexpand(true);
    player_col.append(&title);
    player_col.append(&subtitle);
    player_col.append(&waveform);
    player_col.append(&visualizer);
    player_col.append(&transport);
    player_col.append(&vol_row);
    player_col.append(&slots_title);
    player_col.append(&slots_row);
    player_col.append(&meta_panel);

    let player_scroll = gtk::ScrolledWindow::new();
    player_scroll.set_hexpand(true);
    player_scroll.set_vexpand(true);
    player_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    player_scroll.set_child(Some(&player_col));

    let track_store = gio::ListStore::new::<glib::BoxedAnyObject>();
    let track_selection = gtk::SingleSelection::new(Some(track_store.clone()));
    track_selection.set_autoselect(false);
    track_selection.set_can_unselect(true);

    let track_scroll = gtk::ScrolledWindow::new();
    track_scroll.set_vexpand(true);

    let playlist_col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    playlist_col.add_css_class("ts-card");
    set_padding(&playlist_col, 12);
    playlist_col.set_size_request(380, -1);
    let pl_header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    pl_header.append(&small_caps_label("Library"));
    pl_header.append(&expanding_spacer());
    playlist_col.append(&pl_header);
    playlist_col.append(&track_scroll);

    let main_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    set_padding(&main_row, 12);
    main_row.set_vexpand(true);
    main_row.append(&player_scroll);
    main_row.append(&playlist_col);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&main_row));

    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&toolbar));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("tunesort")
        .default_width(1180)
        .default_height(760)
        .content(&toasts)
        .build();

    let css = gtk::CssProvider::new();
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    let state = Rc::new(App {
        config: RefCell::new(config),
        lib: RefCell::new(Library::new()),
        player,
        is_playing: Cell::new(false),
        muted: Cell::new(false),
        duration: Cell::new(0.0),
        position: Cell::new(0.0),
        volume: Cell::new(volume),
        current_tags: RefCell::new(BTreeMap::new()),
        capture_action: RefCell::new(None),
        peaks: RefCell::new(Vec::new()),
        spectrum: RefCell::new(Vec::new()),
        held: RefCell::new(HashSet::new()),
        updating_slider: Cell::new(false),
        window: window.clone(),
        toasts,
        css,
        lib_label,
        title,
        subtitle,
        waveform: waveform.clone(),
        visualizer: visualizer.clone(),
        time_label,
        play_btn: play_btn.clone(),
        undo_btn,
        vol_slider: vol_slider.clone(),
        vol_label,
        slots: RefCell::new(Vec::new()),
        slots_row: slots_row.clone(),
        meta_panel,
        meta_inputs: RefCell::new(meta_inputs),
        track_store,
        track_selection: track_selection.clone(),
        settings_win: RefCell::new(None),
        keys_container: RefCell::new(None),
    });

    state.apply_css();
    state.build_quickslots(&slots_row);

    let factory = gtk::SignalListItemFactory::new();
    {
        let s = state.clone();
        factory.connect_setup(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let idx = gtk::Label::new(None);
            idx.add_css_class("ts-muted");
            idx.set_xalign(1.0);
            idx.set_width_chars(4);
            let name = gtk::Label::new(None);
            name.set_xalign(0.0);
            name.set_hexpand(true);
            name.set_ellipsize(pango::EllipsizeMode::End);
            row.append(&idx);
            row.append(&name);
            item.set_child(Some(&row));

            let gesture = gtk::GestureClick::new();
            gesture.set_button(gdk::BUTTON_SECONDARY);
            let s = s.clone();
            let item = item.clone();
            gesture.connect_pressed(move |g, _, x, y| {
                let pos = item.position();
                if pos == gtk::INVALID_LIST_POSITION {
                    return;
                }
                if let Some(w) = g.widget() {
                    s.show_move_menu(&w, x, y, Some(pos as usize));
                }
            });
            row.add_controller(gesture);
        });
    }
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let obj = match item.item() {
            Some(o) => o,
            None => return,
        };
        let boxed = obj.downcast_ref::<glib::BoxedAnyObject>().unwrap();
        let row_data = boxed.borrow::<Row>();
        if let Some(row) = item.child().and_downcast::<gtk::Box>() {
            if let Some(idx) = row.first_child().and_downcast::<gtk::Label>() {
                idx.set_text(&row_data.idx.to_string());
                if let Some(name) = idx.next_sibling().and_downcast::<gtk::Label>() {
                    name.set_text(&row_data.title);
                }
            }
        }
    });
    let track_list = gtk::ListView::new(Some(track_selection.clone()), Some(factory));
    track_list.set_single_click_activate(true);
    track_list.add_css_class("ts-tracklist");
    track_scroll.set_child(Some(&track_list));

    {
        let s = state.clone();
        let target = player_col.clone();
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gdk::BUTTON_SECONDARY);
        gesture.connect_pressed(move |_, _, x, y| s.show_move_menu(&target, x, y, None));
        player_col.add_controller(gesture);
    }

    wire(&state, &open_btn, &settings_btn, &play_btn, &prev_btn, &next_btn, &shuffle_btn,
         &delete_btn, &save_tags_btn, &track_list);

    {
        let s = state.clone();
        waveform.set_draw_func(move |_, cr, w, h| s.draw_waveform(cr, w, h));
    }
    {
        let s = state.clone();
        visualizer.set_draw_func(move |_, cr, w, h| s.draw_visualizer(cr, w, h));
    }
    {
        let s = state.clone();
        let click = gtk::GestureClick::new();
        click.connect_released(move |g, _, x, _| {
            let w = g.widget().map(|wd| wd.width()).unwrap_or(1) as f64;
            s.seek_fraction((x / w).clamp(0.0, 1.0));
        });
        waveform.add_controller(click);
    }
    {
        let s = state.clone();
        let drag = gtk::GestureDrag::new();
        drag.connect_drag_update(move |g, off_x, _| {
            if let (Some(start), Some(wd)) = (g.start_point(), g.widget()) {
                let w = wd.width().max(1) as f64;
                s.seek_fraction(((start.0 + off_x) / w).clamp(0.0, 1.0));
            }
        });
        waveform.add_controller(drag);
    }

    {
        let s = state.clone();
        vol_slider.connect_value_changed(move |sl| {
            if s.updating_slider.get() {
                return;
            }
            s.set_volume(sl.value(), true);
        });
    }

    attach_keyboard(&state, window.upcast_ref::<gtk::Widget>(), false);

    {
        let s = state.clone();
        glib::spawn_future_local(async move {
            while let Ok(ev) = rx.recv().await {
                s.on_player_event(ev);
            }
        });
    }

    window.present();

    {
        let s = state.clone();
        glib::idle_add_local_once(move || s.post_build());
    }
}

#[allow(clippy::too_many_arguments)]
fn wire(
    state: &Rc<App>,
    open_btn: &gtk::Button,
    settings_btn: &gtk::Button,
    play_btn: &gtk::Button,
    prev_btn: &gtk::Button,
    next_btn: &gtk::Button,
    shuffle_btn: &gtk::Button,
    delete_btn: &gtk::Button,
    save_tags_btn: &gtk::Button,
    track_list: &gtk::ListView,
) {
    macro_rules! on {
        ($w:expr, $body:expr) => {{
            let s = state.clone();
            $w.connect_clicked(move |_| $body(&s));
        }};
    }
    on!(open_btn, |s: &Rc<App>| s.act_open_library());
    on!(settings_btn, |s: &Rc<App>| s.act_toggle_settings());
    on!(play_btn, |s: &Rc<App>| s.act_play_pause());
    on!(prev_btn, |s: &Rc<App>| s.act_skip(-1));
    on!(next_btn, |s: &Rc<App>| s.act_skip(1));
    on!(shuffle_btn, |s: &Rc<App>| s.act_shuffle());
    on!(delete_btn, |s: &Rc<App>| s.act_delete());
    on!(save_tags_btn, |s: &Rc<App>| s.save_metadata());
    {
        let s = state.clone();
        track_list.connect_activate(move |_, pos| s.play_track(pos as i64, true));
    }
}

fn attach_keyboard(state: &Rc<App>, widget: &gtk::Widget, settings: bool) {
    let key = gtk::EventControllerKey::new();
    key.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let s = state.clone();
        let settings = settings;
        key.connect_key_pressed(move |_, keyval, code, mstate| {
            s.key_pressed(keyval, code, mstate, settings)
        });
    }
    {
        let s = state.clone();
        key.connect_key_released(move |_, keyval, code, _| {
            s.key_released(keyval, code);
        });
    }
    widget.add_controller(key);
}

impl App {
    fn load_current(&self, autoplay: bool) {
        let path = self.lib.borrow().current().cloned();
        match path {
            None => {
                self.is_playing.set(false);
                self.update_nowplaying(None);
                self.player.stop();
                self.peaks.borrow_mut().clear();
                self.waveform.queue_draw();
            }
            Some(path) => {
                self.peaks.borrow_mut().clear();
                self.waveform.queue_draw();
                self.player.load(&path, autoplay);
                self.is_playing.set(autoplay);
                self.update_nowplaying(Some(&path));
                self.sync_selection();
                self.update_playbtn();
            }
        }
    }

    fn play_track(&self, idx: i64, autoplay: bool) {
        let ok = self.lib.borrow_mut().go_to(idx).is_some();
        if ok {
            self.load_current(autoplay);
        }
    }

    fn act_play_pause(self: &Rc<Self>) {
        if self.lib.borrow().current().is_none() {
            return;
        }
        self.player.play_pause();
    }

    fn act_skip(self: &Rc<Self>, direction: i64) {
        if self.lib.borrow().tracks.is_empty() {
            return;
        }
        if direction > 0 {
            self.lib.borrow_mut().next();
        } else {
            self.lib.borrow_mut().prev();
        }

        self.load_current(true);
    }

    fn act_seek(self: &Rc<Self>, direction: f64) {
        let step = self.config.borrow().get_f64("seek_step", 5.0) * direction;
        self.player.seek_relative(step);
    }

    fn nudge_volume(self: &Rc<Self>, direction: f64) {
        let step = self.config.borrow().get_f64("volume_step", 0.05);
        self.set_volume(self.volume.get() + step * direction, false);
    }

    fn set_volume(&self, value: f64, from_slider: bool) {
        let v = value.clamp(0.0, 1.0);
        self.volume.set(v);
        self.player.set_volume(v);
        self.config.borrow_mut().set_setting("volume", serde_json::json!((v * 1000.0).round() / 1000.0));
        if !from_slider {
            self.updating_slider.set(true);
            self.vol_slider.set_value(v);
            self.updating_slider.set(false);
        }
        self.vol_label.set_text(&format!("{}%", (v * 100.0) as i32));
    }

    fn act_mute(self: &Rc<Self>) {
        let m = !self.muted.get();
        self.muted.set(m);
        self.player.set_muted(m);
        self.notify(if m { "Muted" } else { "Unmuted" });
    }

    fn act_delete(self: &Rc<Self>) {
        let path = self.lib.borrow().current().cloned();
        let path = match path {
            Some(p) => p,
            None => return,
        };
        let deleted = self.lib.borrow_mut().delete_current().is_some();
        if deleted {
            self.notify(&format!("Trashed  {}", base_name(&path)));
            self.after_removal();
        } else {
            self.notify("Nothing to delete");
        }
    }

    fn act_quick_move(self: &Rc<Self>, n: usize) {
        let (dest, label) = {
            let cfg = self.config.borrow();
            (cfg.quickslot_path(n - 1), cfg.quickslot_label(n - 1))
        };
        if dest.is_empty() || !Path::new(&dest).is_dir() {
            self.notify(&format!("Slot {n} has no folder set (Ctrl+{n} to assign)"));
            return;
        }
        let shown = if label.is_empty() { None } else { Some(label) };
        self.move_current_to(&dest, shown);
    }

    fn move_current_to(self: &Rc<Self>, dest: &str, label: Option<String>) {
        if self.lib.borrow().current().is_none() {
            return;
        }
        if dest.is_empty() || !Path::new(dest).is_dir() {
            self.notify("That folder no longer exists");
            return;
        }
        let moved = self.lib.borrow_mut().move_current(Path::new(dest)).is_some();
        if moved {
            self.config.borrow_mut().push_recent_destination(dest);
            let shown = label.unwrap_or_else(|| dest.to_string());
            self.notify(&format!("Moved → {shown}"));
            self.after_removal();
        }
    }

    fn move_row_to(self: &Rc<Self>, idx: usize, dest: &str) {
        if dest.is_empty() || !Path::new(dest).is_dir() {
            self.notify("That folder no longer exists");
            return;
        }
        let was_current = self.lib.borrow().index == idx as i64;
        let name = self.lib.borrow().tracks.get(idx).map(|p| base_name(p));
        let moved = self.lib.borrow_mut().move_index(idx, Path::new(dest)).is_some();
        if moved {
            self.config.borrow_mut().push_recent_destination(dest);
            self.notify(&format!("Moved  {}", name.unwrap_or_default()));
            if was_current {
                self.after_removal();
            } else {
                self.refresh_table();
                self.refresh_undo();
                self.sync_selection();
            }
        }
    }

    fn do_move_to(self: &Rc<Self>, track_idx: Option<usize>, dest: &str) {
        match track_idx {
            Some(i) => self.move_row_to(i, dest),
            None => self.move_current_to(dest, None),
        }
    }

    fn browse_and_move(self: &Rc<Self>, track_idx: Option<usize>) {
        let start = {
            let root = self.lib.borrow().root.to_string_lossy().to_string();
            if root.is_empty() { home_dir() } else { root }
        };
        let parent: gtk::Window = self.window.clone().upcast();
        let s = self.clone();
        open_folder_dialog(&parent, &start, "Move track to folder", move |folder| {
            s.do_move_to(track_idx, &folder);
        });
    }

    fn show_move_menu(
        self: &Rc<Self>,
        anchor: &impl IsA<gtk::Widget>,
        x: f64,
        y: f64,
        track_idx: Option<usize>,
    ) {
        let have_track = match track_idx {
            Some(i) => i < self.lib.borrow().tracks.len(),
            None => self.lib.borrow().current().is_some(),
        };
        if !have_track {
            return;
        }

        let pop = gtk::Popover::new();
        pop.set_parent(anchor);
        pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
        set_padding(&col, 6);
        col.set_width_request(240);
        col.append(&small_caps_label("Move to…"));

        let add_dest = |col: &gtk::Box, title: &str, sub: Option<&str>, dest: String| {
            let btn = gtk::Button::new();
            btn.add_css_class("flat");
            let inner = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let t = gtk::Label::new(Some(title));
            t.set_xalign(0.0);
            t.set_ellipsize(pango::EllipsizeMode::End);
            inner.append(&t);
            if let Some(sub) = sub {
                let s = gtk::Label::new(Some(sub));
                s.add_css_class("ts-muted");
                s.add_css_class("ts-small");
                s.set_xalign(0.0);
                s.set_ellipsize(pango::EllipsizeMode::Middle);
                inner.append(&s);
            }
            btn.set_child(Some(&inner));
            let s = self.clone();
            let pop = pop.clone();
            btn.connect_clicked(move |_| {
                pop.popdown();
                s.do_move_to(track_idx, &dest);
            });
            col.append(&btn);
        };

        let count = self.config.borrow().quickslot_count();
        let mut slot_paths: HashSet<String> = HashSet::new();
        let mut added_any = false;
        for i in 0..count {
            let path = self.config.borrow().quickslot_path(i);
            if path.is_empty() {
                continue;
            }
            slot_paths.insert(path.clone());
            let label = self.config.borrow().quickslot_label(i);
            let title = format!("{}  {}", i + 1, label);
            add_dest(&col, &title, Some(&path), path.clone());
            added_any = true;
        }

        let recents: Vec<String> = self
            .config
            .borrow()
            .recent_destinations()
            .into_iter()
            .filter(|r| !slot_paths.contains(r))
            .collect();
        if !recents.is_empty() {
            if added_any {
                col.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            }
            col.append(&small_caps_label("Recent"));
            for r in recents {
                let base = base_name(Path::new(&r));
                add_dest(&col, &base, Some(&r), r.clone());
            }
            added_any = true;
        }

        if added_any {
            col.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        }
        let browse = button_with_label("Browse…", "folder-symbolic");
        browse.add_css_class("flat");
        {
            let s = self.clone();
            let pop = pop.clone();
            browse.connect_clicked(move |_| {
                pop.popdown();
                s.browse_and_move(track_idx);
            });
        }
        col.append(&browse);

        pop.set_child(Some(&col));

        pop.connect_closed(|p| p.unparent());
        pop.popup();
    }

    fn act_set_slot(self: &Rc<Self>, n: usize) {
        let start = {
            let cfg = self.config.borrow();
            let p = cfg.quickslot_path(n - 1);
            if !p.is_empty() {
                p
            } else {
                let root = self.lib.borrow().root.to_string_lossy().to_string();
                if root.is_empty() {
                    home_dir()
                } else {
                    root
                }
            }
        };
        let parent: gtk::Window = self
            .settings_win
            .borrow()
            .clone()
            .map(|w| w.upcast())
            .unwrap_or_else(|| self.window.clone().upcast());
        let s = self.clone();
        open_folder_dialog(&parent, &start, &format!("Assign folder to slot {n}"), move |folder| {
            s.config.borrow_mut().set_quickslot_path(n - 1, &folder);
            s.refresh_quickslots();
            s.notify(&format!("Slot {n} → {folder}"));
        });
    }

    fn after_removal(self: &Rc<Self>) {
        self.refresh_table();
        self.refresh_undo();
        let has_current = self.lib.borrow().current().is_some();
        if has_current {
            if self.config.borrow().get_bool("advance_after_action", true) {
                self.load_current(self.is_playing.get());
            } else {
                let path = self.lib.borrow().current().cloned();
                self.update_nowplaying(path.as_deref());
                self.sync_selection();
            }
        } else {
            self.load_current(false);
        }
    }

    fn act_shuffle(self: &Rc<Self>) {
        if self.lib.borrow().tracks.is_empty() {
            return;
        }
        self.lib.borrow_mut().shuffle(true);
        self.refresh_table();
        self.sync_selection();
        self.notify("Shuffled");
    }

    fn act_undo(self: &Rc<Self>) {
        let restored = self.lib.borrow_mut().undo();
        match restored {
            Some(path) => {
                self.refresh_table();
                self.refresh_undo();
                self.load_current(false);
                self.notify(&format!("Restored  {}", base_name(&path)));
            }
            None => self.notify("Nothing to undo"),
        }
    }

    fn act_open_library(self: &Rc<Self>) {
        let start = {
            let lib = self.lib.borrow();
            let root = lib.root.to_string_lossy().to_string();
            if !root.is_empty() {
                root
            } else {
                let p = self.config.borrow().library_path();
                if p.is_empty() {
                    home_dir()
                } else {
                    p
                }
            }
        };
        let parent = self.window.clone();
        let s = self.clone();
        open_folder_dialog(&parent.upcast(), &start, "Choose music library", move |folder| {
            s.set_library(&folder);
        });
    }

    fn act_toggle_settings(self: &Rc<Self>) {
        if let Some(win) = self.settings_win.borrow().clone() {
            if win.is_visible() {
                win.close();
                return;
            }
            win.present();
            return;
        }
        self.build_settings();
        if let Some(win) = self.settings_win.borrow().clone() {
            win.present();
        }
    }

    fn act_toggle_visualizer(self: &Rc<Self>) {
        let on = !self.config.borrow().get_bool("visualizer", false);
        self.config.borrow_mut().set_setting("visualizer", serde_json::json!(on));
        self.player.set_visualizer(on);
        self.visualizer.set_visible(on);
        self.notify(&format!("Visualizer {}", if on { "on" } else { "off" }));
    }

    fn act_toggle_metadata(self: &Rc<Self>) {
        let on = !self.config.borrow().get_bool("metadata_editing", false);
        self.config.borrow_mut().set_setting("metadata_editing", serde_json::json!(on));
        self.apply_metadata_visibility();
        self.notify(&format!("Metadata editing {}", if on { "on" } else { "off" }));
    }

    fn set_library(self: &Rc<Self>, path: &str) {
        if path.is_empty() || !Path::new(path).is_dir() {
            self.notify("Not a folder");
            return;
        }
        self.config.borrow_mut().set_library_path(path);
        let recurse = self.config.borrow().get_bool("recurse_subfolders", true);
        let count = self.lib.borrow_mut().load(Path::new(path), recurse);
        if count > 0 && self.config.borrow().get_bool("shuffle_on_load", false) {
            self.lib.borrow_mut().shuffle(false);
        }
        self.refresh_table();
        self.lib_label.set_text(&format!("{path}   ·   {count} tracks"));
        self.load_current(false);
        self.notify(&format!("Loaded {count} tracks"));
    }

    fn update_time(&self) {
        self.time_label
            .set_text(&format!("{} / {}", fmt_time(self.position.get()), fmt_time(self.duration.get())));
        self.waveform.queue_draw();
    }

    fn update_playbtn(&self) {
        let icon = if self.is_playing.get() {
            "media-playback-pause-symbolic"
        } else {
            "media-playback-start-symbolic"
        };
        self.play_btn.set_icon_name(icon);
    }

    fn update_nowplaying(&self, path: Option<&Path>) {
        match path {
            None => {
                self.current_tags.borrow_mut().clear();
                self.title.set_text("No track");
                self.subtitle.set_text("");
                self.load_meta_inputs();
            }
            Some(path) => {
                let tags = metadata::read_tags(path);
                self.title.set_text(&metadata::display_title(path, &tags));
                let album = tags.get("album").map(|s| s.trim().to_string()).unwrap_or_default();
                let folder = path
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                self.subtitle.set_text(if album.is_empty() { &folder } else { &album });
                *self.current_tags.borrow_mut() = tags;
                self.update_playbtn();
                self.load_meta_inputs();
            }
        }
    }

    fn refresh_table(&self) {
        self.track_store.remove_all();
        let lib = self.lib.borrow();
        for (i, p) in lib.tracks.iter().enumerate() {
            let title = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            self.track_store.append(&glib::BoxedAnyObject::new(Row { idx: i, title }));
        }
    }

    fn sync_selection(&self) {
        let idx = self.lib.borrow().index;
        if idx >= 0 && (idx as u32) < self.track_store.n_items() {
            self.track_selection.set_selected(idx as u32);
        } else {
            self.track_selection.set_selected(gtk::INVALID_LIST_POSITION);
        }
    }

    fn refresh_undo(&self) {
        let has = !self.lib.borrow().undo_stack.is_empty();
        self.undo_btn.set_sensitive(has);
    }

    fn build_quickslots(self: &Rc<Self>, row: &gtk::Box) {
        let mut slots = self.slots.borrow_mut();
        let count = self.config.borrow().quickslot_count();
        for i in 0..count {
            let cfg = self.config.borrow();
            let has = !cfg.quickslot_path(i).is_empty();
            let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
            card.add_css_class("ts-slot");
            if has {
                card.add_css_class("ts-slot-set");
            }
            set_padding(&card, 8);
            card.set_hexpand(true);

            let top = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            let num = gtk::Label::new(Some(&(i + 1).to_string()));
            num.add_css_class("ts-accent");
            num.add_css_class("ts-bold");
            let label = gtk::Label::new(Some(&cfg.quickslot_label(i)));
            label.set_ellipsize(pango::EllipsizeMode::End);
            label.set_xalign(0.0);
            let edit = gtk::Button::from_icon_name("document-edit-symbolic");
            edit.add_css_class("flat");
            edit.add_css_class("circular");
            edit.set_focusable(false);
            top.append(&num);
            top.append(&label);
            top.append(&expanding_spacer());
            top.append(&edit);

            let path_text = cfg.quickslot_path(i);
            let path = gtk::Label::new(Some(if path_text.is_empty() {
                "— not set —"
            } else {
                &path_text
            }));
            path.add_css_class("ts-muted");
            path.add_css_class("ts-small");
            path.set_ellipsize(pango::EllipsizeMode::Middle);
            path.set_xalign(0.0);
            drop(cfg);

            card.append(&top);
            card.append(&path);

            let n = i + 1;
            {
                let s = self.clone();
                let card_ref = card.clone();
                let edit_ref = edit.clone();
                let click = gtk::GestureClick::new();
                click.connect_released(move |_, _, x, y| {
                    if let Some(picked) = card_ref.pick(x, y, gtk::PickFlags::DEFAULT) {
                        let edit_w: gtk::Widget = edit_ref.clone().upcast();
                        if picked == edit_w || picked.is_ancestor(&edit_ref) {
                            return;
                        }
                    }
                    s.act_quick_move(n);
                });
                card.add_controller(click);
            }
            {
                let s = self.clone();
                edit.connect_clicked(move |_| s.act_set_slot(n));
            }

            row.append(&card);
            slots.push(SlotWidgets { card, label, path });
        }
    }

    fn rebuild_quickslots(self: &Rc<Self>) {
        while let Some(child) = self.slots_row.first_child() {
            self.slots_row.remove(&child);
        }
        self.slots.borrow_mut().clear();
        let row = self.slots_row.clone();
        self.build_quickslots(&row);
    }

    fn refresh_quickslots(&self) {
        let cfg = self.config.borrow();
        for (i, w) in self.slots.borrow().iter().enumerate() {
            let path = cfg.quickslot_path(i);
            w.label.set_text(&cfg.quickslot_label(i));
            w.path.set_text(if path.is_empty() { "— not set —" } else { &path });
            if path.is_empty() {
                w.card.remove_css_class("ts-slot-set");
            } else {
                w.card.add_css_class("ts-slot-set");
            }
        }
    }

    fn apply_metadata_visibility(&self) {
        let on = self.config.borrow().get_bool("metadata_editing", false);
        self.meta_panel.set_visible(on);
    }

    fn load_meta_inputs(&self) {
        let tags = self.current_tags.borrow();
        for (field, entry) in self.meta_inputs.borrow().iter() {
            entry.set_text(tags.get(field).map(|s| s.as_str()).unwrap_or(""));
        }
    }

    fn save_metadata(self: &Rc<Self>) {
        let path = self.lib.borrow().current().cloned();
        let path = match path {
            Some(p) => p,
            None => return,
        };
        let mut fields: BTreeMap<String, String> = BTreeMap::new();
        for (field, entry) in self.meta_inputs.borrow().iter() {
            fields.insert(field.clone(), entry.text().to_string());
        }
        if metadata::write_tags(&path, &fields) {
            self.notify("Tags saved");
            self.update_nowplaying(Some(&path));
        } else {
            self.notify("Could not write tags");
        }
    }

    fn on_player_event(self: &Rc<Self>, ev: PlayerEvent) {
        match ev {
            PlayerEvent::Ready { duration } => {
                self.duration.set(duration);
                self.update_time();
            }
            PlayerEvent::Time { position, duration } => {
                self.position.set(position);
                if duration > 0.0 {
                    self.duration.set(duration);
                }
                self.update_time();
            }
            PlayerEvent::Play => {
                self.is_playing.set(true);
                self.update_playbtn();
            }
            PlayerEvent::Pause => {
                self.is_playing.set(false);
                self.update_playbtn();
            }
            PlayerEvent::Ended => {
                if self.config.borrow().get_bool("auto_advance", true) {
                    self.lib.borrow_mut().next();
                    self.load_current(true);
                } else {
                    self.is_playing.set(false);
                    self.update_playbtn();
                }
            }
            PlayerEvent::Peaks { id, peaks } => {
                if id == self.player.current_load_id() {
                    *self.peaks.borrow_mut() = peaks;
                    self.waveform.queue_draw();
                }
            }
            PlayerEvent::Spectrum(s) => {
                *self.spectrum.borrow_mut() = s;
                if self.visualizer.get_visible() {
                    self.visualizer.queue_draw();
                }
            }
        }
    }

    fn draw_waveform(&self, cr: &cairo::Context, w: i32, h: i32) {
        let w = w as f64;
        let h = h as f64;
        let peaks = self.peaks.borrow();
        let theme = self.config.borrow();
        let (wr, wg, wb) = parse_hex(&theme.theme_color("wave"));
        let (pr, pg, pb) = parse_hex(&theme.theme_color("wave_progress"));
        let progress = if self.duration.get() > 0.0 {
            (self.position.get() / self.duration.get()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let bar_w = 2.0;
        let gap = 1.0;
        let stride = bar_w + gap;
        let bars = (w / stride).floor().max(1.0) as usize;
        if peaks.is_empty() {
            cr.set_source_rgb(wr, wg, wb);
            cr.rectangle(0.0, h / 2.0 - 1.0, w, 2.0);
            let _ = cr.fill();
            return;
        }
        let n = peaks.len();
        for i in 0..bars {
            let pk = peaks[i * n / bars] as f64;
            let bh = (pk * h).max(2.0);
            let x = i as f64 * stride;
            if (x / w) <= progress {
                cr.set_source_rgb(pr, pg, pb);
            } else {
                cr.set_source_rgb(wr, wg, wb);
            }
            cr.rectangle(x, (h - bh) / 2.0, bar_w, bh);
            let _ = cr.fill();
        }
    }

    fn draw_visualizer(&self, cr: &cairo::Context, w: i32, h: i32) {
        let w = w as f64;
        let h = h as f64;
        let mags = self.spectrum.borrow();
        if mags.is_empty() {
            return;
        }
        let theme = self.config.borrow().get_str("visualizer_theme", "amber");
        let colors = vis_theme_colors(&theme);
        let (r0, g0, b0) = parse_hex(colors[0]);
        let (r1, g1, b1) = parse_hex(colors[1]);
        let (r2, g2, b2) = parse_hex(colors[2]);
        let grad = cairo::LinearGradient::new(0.0, h, 0.0, 0.0);
        grad.add_color_stop_rgb(0.0, r0, g0, b0);
        grad.add_color_stop_rgb(0.6, r1, g1, b1);
        grad.add_color_stop_rgb(1.0, r2, g2, b2);
        let _ = cr.set_source(&grad);
        let bars = mags.len().min(64);
        let bw = w / bars as f64;
        for i in 0..bars {
            let v = mags[i] as f64;
            let bh = (v * h).max(1.0);
            cr.rectangle(i as f64 * bw + 1.0, h - bh, bw - 2.0, bh);
        }
        let _ = cr.fill();
    }

    fn seek_fraction(&self, frac: f64) {
        let dur = self.duration.get();
        if dur > 0.0 {
            self.player.seek_to(frac * dur);
        }
    }

    fn key_pressed(
        self: &Rc<Self>,
        keyval: gdk::Key,
        keycode: u32,
        state: gdk::ModifierType,
        settings: bool,
    ) -> glib::Propagation {
        if is_modifier_key(keyval) {
            return glib::Propagation::Proceed;
        }
        let code = match code_for(keyval, keycode) {
            Some(c) => c,
            None => return glib::Propagation::Proceed,
        };
        let is_repeat = self.held.borrow().contains(&code);
        self.held.borrow_mut().insert(code.clone());

        let keystr = keystr_with_mods(state, &code);

        if self.capture_action.borrow().is_some() {
            self.finish_capture(&keystr);
            return glib::Propagation::Stop;
        }

        let focus = if settings {
            self.settings_win
                .borrow()
                .clone()
                .and_then(|w| GtkWindowExt::focus(&w))
        } else {
            GtkWindowExt::focus(&self.window)
        };
        if is_editable_focus(focus) {
            return glib::Propagation::Proceed;
        }

        let action = match self.config.borrow().action_for_key(&keystr) {
            Some(a) => a,
            None => return glib::Propagation::Proceed,
        };
        if is_repeat && !REPEATABLE.contains(&action.as_str()) {
            return glib::Propagation::Stop;
        }
        self.dispatch(&action);
        glib::Propagation::Stop
    }

    fn key_released(&self, keyval: gdk::Key, keycode: u32) {
        if let Some(code) = code_for(keyval, keycode) {
            self.held.borrow_mut().remove(&code);
        }
    }

    fn dispatch(self: &Rc<Self>, action: &str) {
        match action {
            "play_pause" => self.act_play_pause(),
            "next" => self.act_skip(1),
            "prev" => self.act_skip(-1),
            "volume_up" => self.nudge_volume(1.0),
            "volume_down" => self.nudge_volume(-1.0),
            "mute" => self.act_mute(),
            "seek_forward" => self.act_seek(1.0),
            "seek_back" => self.act_seek(-1.0),
            "delete" => self.act_delete(),
            "shuffle" => self.act_shuffle(),
            "undo" => self.act_undo(),
            "open_library" => self.act_open_library(),
            "toggle_settings" => self.act_toggle_settings(),
            "toggle_visualizer" => self.act_toggle_visualizer(),
            "toggle_metadata" => self.act_toggle_metadata(),
            _ => {
                if let Some(n) = action.strip_prefix("quickslot_").and_then(|s| s.parse::<usize>().ok()) {
                    self.act_quick_move(n);
                } else if let Some(n) = action.strip_prefix("set_slot_").and_then(|s| s.parse::<usize>().ok()) {
                    self.act_set_slot(n);
                }
            }
        }
    }

    fn finish_capture(self: &Rc<Self>, keystr: &str) {
        let action = self.capture_action.borrow_mut().take();
        if let Some(action) = action {
            self.config.borrow_mut().bind_key(keystr, &action);
            self.refresh_keybinds();
            self.notify(&format!("Bound {keystr} → {action}"));
        }
    }

    fn notify(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    fn apply_css(&self) {
        let cfg = self.config.borrow();
        let mut css = String::new();
        for (k, v) in cfg.theme_pairs() {
            css.push_str(&format!("@define-color ts_{k} {v};\n"));
        }
        css.push_str(
            r#"
            window { background-color: @ts_bg; color: @ts_text; }
            .ts-card { background-color: @ts_surface; border: 1px solid @ts_border; border-radius: 10px; }
            .ts-panel { background-color: @ts_surface2; border: 1px solid @ts_border; border-radius: 8px; }
            .ts-muted { color: @ts_muted; }
            .ts-accent { color: @ts_accent; }
            .ts-title { color: @ts_text; }
            .ts-brand { font-weight: 600; font-size: 1.1rem; }
            .ts-h1 { font-size: 1.6rem; font-weight: 600; }
            .ts-small { font-size: 0.8rem; }
            .ts-bold { font-weight: 700; }
            .ts-danger { color: @ts_danger; }
            .ts-slot { background-color: @ts_surface2; border: 1px solid @ts_border; border-radius: 8px; transition: border-color .15s; }
            .ts-slot:hover { border-color: @ts_muted; }
            .ts-slot-set { border-color: @ts_accent; }
            .ts-play { background-color: @ts_accent; color: @ts_bg; min-width: 46px; min-height: 46px; }
            .ts-play:hover { background-color: @ts_accent; }
            .ts-tracklist { background: transparent; }
            .ts-tracklist row:selected { background-color: alpha(@ts_accent, 0.18); }
            .ts-tracklist row:hover { background-color: @ts_surface2; }
            headerbar { background-color: @ts_surface; border-bottom: 1px solid @ts_border; }
            "#,
        );
        self.css.load_from_string(&css);
    }

    fn apply_theme_live(&self) {
        self.apply_css();
        self.waveform.queue_draw();
        self.visualizer.queue_draw();
    }

    fn post_build(self: &Rc<Self>) {
        self.player.set_volume(self.volume.get());
        let (visual, recurse, shuffle_on_load, lib_path) = {
            let cfg = self.config.borrow();
            (
                cfg.get_bool("visualizer", false),
                cfg.get_bool("recurse_subfolders", true),
                cfg.get_bool("shuffle_on_load", false),
                cfg.library_path(),
            )
        };
        self.player.set_visualizer(visual);
        self.visualizer.set_visible(visual);
        self.apply_metadata_visibility();
        self.refresh_quickslots();
        self.refresh_undo();

        if !lib_path.is_empty() && Path::new(&lib_path).is_dir() {
            let count = self.lib.borrow_mut().load(Path::new(&lib_path), recurse);
            if count > 0 && shuffle_on_load {
                self.lib.borrow_mut().shuffle(false);
            }
            self.refresh_table();
            self.lib_label.set_text(&format!("{lib_path}   ·   {count} tracks"));
            self.load_current(false);
        }
    }

    fn build_settings(self: &Rc<Self>) {
        let win = adw::Window::builder()
            .title("Settings")
            .transient_for(&self.window)
            .modal(true)
            .default_width(820)
            .default_height(680)
            .build();

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let header = adw::HeaderBar::new();
        let htitle = gtk::Label::new(Some("Settings"));
        htitle.add_css_class("ts-accent");
        htitle.add_css_class("ts-brand");
        header.set_title_widget(Some(&htitle));
        root.append(&header);

        let notebook = gtk::Notebook::new();
        notebook.set_vexpand(true);
        notebook.append_page(&self.settings_general(), Some(&gtk::Label::new(Some("General"))));
        notebook.append_page(&self.settings_keys(), Some(&gtk::Label::new(Some("Keybindings"))));
        notebook.append_page(&self.settings_slots(), Some(&gtk::Label::new(Some("Quick slots"))));
        notebook.append_page(&self.settings_theme(), Some(&gtk::Label::new(Some("Theme"))));
        notebook.append_page(&self.settings_raw(), Some(&gtk::Label::new(Some("Config file"))));
        root.append(&notebook);

        win.set_content(Some(&root));
        attach_keyboard(self, win.upcast_ref::<gtk::Widget>(), true);
        {
            let s = self.clone();
            win.connect_close_request(move |_| {
                *s.keys_container.borrow_mut() = None;
                *s.settings_win.borrow_mut() = None;
                glib::Propagation::Proceed
            });
        }
        *self.settings_win.borrow_mut() = Some(win);
    }

    fn settings_general(self: &Rc<Self>) -> gtk::Widget {
        let col = gtk::Box::new(gtk::Orientation::Vertical, 12);
        set_padding(&col, 16);

        let add_switch = |col: &gtk::Box, label: &str, key: &'static str, default: bool, s: &Rc<App>, after: Option<fn(&Rc<App>)>| {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let lbl = gtk::Label::new(Some(label));
            lbl.set_xalign(0.0);
            lbl.set_hexpand(true);
            let sw = gtk::Switch::new();
            sw.set_active(s.config.borrow().get_bool(key, default));
            row.append(&lbl);
            row.append(&sw);
            let s2 = s.clone();
            sw.connect_active_notify(move |sw| {
                s2.config.borrow_mut().set_setting(key, serde_json::json!(sw.is_active()));
                if let Some(f) = after {
                    f(&s2);
                }
            });
            col.append(&row);
        };

        add_switch(&col, "Auto-advance to next track on end", "auto_advance", true, self, None);
        add_switch(&col, "Advance to next track after delete/move", "advance_after_action", true, self, None);
        add_switch(&col, "Recurse into subfolders when loading", "recurse_subfolders", true, self, None);
        add_switch(&col, "Shuffle library on load", "shuffle_on_load", false, self, None);
        add_switch(&col, "Enable metadata (ID3/tag) editing", "metadata_editing", false, self, Some(|s: &Rc<App>| s.apply_metadata_visibility()));

        {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let lbl = gtk::Label::new(Some("Visualizer"));
            lbl.set_xalign(0.0);
            lbl.set_hexpand(true);
            let sw = gtk::Switch::new();
            sw.set_active(self.config.borrow().get_bool("visualizer", false));
            row.append(&lbl);
            row.append(&sw);
            let s = self.clone();
            sw.connect_active_notify(move |sw| {
                let on = sw.is_active();
                s.config.borrow_mut().set_setting("visualizer", serde_json::json!(on));
                s.player.set_visualizer(on);
                s.visualizer.set_visible(on);
            });
            col.append(&row);
        }

        {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let lbl = gtk::Label::new(Some("Visualizer theme"));
            lbl.set_xalign(0.0);
            lbl.set_hexpand(true);
            let dd = gtk::DropDown::from_strings(VIS_THEMES);
            let cur = self.config.borrow().get_str("visualizer_theme", "amber");
            if let Some(pos) = VIS_THEMES.iter().position(|t| *t == cur) {
                dd.set_selected(pos as u32);
            }
            row.append(&lbl);
            row.append(&dd);
            let s = self.clone();
            dd.connect_selected_notify(move |dd| {
                let idx = dd.selected() as usize;
                if let Some(theme) = VIS_THEMES.get(idx) {
                    s.config.borrow_mut().set_setting("visualizer_theme", serde_json::json!(theme));
                    s.visualizer.queue_draw();
                }
            });
            col.append(&row);
        }

        {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
            let vstep = labeled_spin(
                "Volume step",
                self.config.borrow().get_f64("volume_step", 0.05),
                0.01,
                0.5,
                0.01,
            );
            let sstep = labeled_spin(
                "Seek step (seconds)",
                self.config.borrow().get_f64("seek_step", 5.0),
                1.0,
                120.0,
                1.0,
            );
            {
                let s = self.clone();
                vstep.1.connect_value_changed(move |sp| {
                    s.config.borrow_mut().set_setting("volume_step", serde_json::json!(sp.value()));
                });
            }
            {
                let s = self.clone();
                sstep.1.connect_value_changed(move |sp| {
                    s.config.borrow_mut().set_setting("seek_step", serde_json::json!(sp.value()));
                });
            }
            row.append(&vstep.0);
            row.append(&sstep.0);
            col.append(&row);
        }

        scrolled(&col)
    }

    fn settings_keys(self: &Rc<Self>) -> gtk::Widget {
        let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
        set_padding(&col, 16);
        let info = gtk::Label::new(Some(
            "Click Rebind, then press the key (or chord) you want. Codes use \
             KeyboardEvent.code so Numpad keys are distinct.",
        ));
        info.add_css_class("ts-muted");
        info.set_wrap(true);
        info.set_xalign(0.0);
        col.append(&info);

        let reset = button_with_label("Reset to defaults", "edit-undo-symbolic");
        reset.add_css_class("flat");
        reset.set_halign(gtk::Align::Start);
        {
            let s = self.clone();
            reset.connect_clicked(move |_| {
                s.config.borrow_mut().reset_keybindings();
                s.refresh_keybinds();
                s.notify("Keybindings reset");
            });
        }
        col.append(&reset);

        let container = gtk::Box::new(gtk::Orientation::Vertical, 4);
        col.append(&container);
        *self.keys_container.borrow_mut() = Some(container);
        self.refresh_keybinds();

        scrolled(&col)
    }

    fn refresh_keybinds(self: &Rc<Self>) {
        let container = match self.keys_container.borrow().clone() {
            Some(c) => c,
            None => return,
        };
        while let Some(child) = container.first_child() {
            container.remove(&child);
        }

        let mut action_keys: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (key, action) in self.config.borrow().keybindings() {
            if let Some(a) = action.as_str() {
                action_keys.entry(a.to_string()).or_default().push(key.clone());
            }
        }
        for (action, desc) in config::actions() {
            let keys = action_keys.get(action).cloned().unwrap_or_default();
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.add_css_class("ts-panel");
            set_padding(&row, 8);
            let lbl = gtk::Label::new(Some(desc));
            lbl.set_xalign(0.0);
            lbl.set_hexpand(true);
            let keys_lbl = gtk::Label::new(Some(if keys.is_empty() {
                "—".to_string()
            } else {
                keys.join("  ")
            }.as_str()));
            keys_lbl.add_css_class("ts-accent");
            keys_lbl.set_width_chars(20);
            keys_lbl.set_xalign(0.0);
            let rebind = gtk::Button::with_label("Rebind");
            rebind.add_css_class("flat");
            let s = self.clone();
            let action_name = action.to_string();
            let desc_name = desc.to_string();
            rebind.connect_clicked(move |_| {
                *s.capture_action.borrow_mut() = Some(action_name.clone());
                s.notify(&format!("Press a key for: {desc_name}"));
            });
            row.append(&lbl);
            row.append(&keys_lbl);
            row.append(&rebind);
            container.append(&row);
        }
    }

    fn settings_slots(self: &Rc<Self>) -> gtk::Widget {
        let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
        set_padding(&col, 16);

        let count_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let count_lbl = gtk::Label::new(Some("Number of quick slots"));
        count_lbl.set_xalign(0.0);
        count_lbl.set_hexpand(true);
        let count_spin = gtk::SpinButton::with_range(1.0, NUM_QUICKSLOTS as f64, 1.0);
        count_spin.set_value(self.config.borrow().quickslot_count() as f64);
        count_row.append(&count_lbl);
        count_row.append(&count_spin);
        col.append(&count_row);

        let rows = gtk::Box::new(gtk::Orientation::Vertical, 8);
        self.populate_slot_rows(&rows);
        col.append(&rows);
        {
            let s = self.clone();
            let rows = rows.clone();
            count_spin.connect_value_changed(move |sp| {
                s.config.borrow_mut().set_quickslot_count(sp.value() as usize);
                s.populate_slot_rows(&rows);
                s.rebuild_quickslots();
            });
        }

        col.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        col.append(&small_caps_label("Recent destinations"));
        let recent_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let recent_lbl = gtk::Label::new(Some("How many to remember"));
        recent_lbl.set_xalign(0.0);
        recent_lbl.set_hexpand(true);
        let recent_spin = gtk::SpinButton::with_range(0.0, 50.0, 1.0);
        recent_spin.set_value(self.config.borrow().max_recent() as f64);
        let clear_btn = button_with_label("Clear", "edit-clear-symbolic");
        clear_btn.add_css_class("flat");
        recent_row.append(&recent_lbl);
        recent_row.append(&recent_spin);
        recent_row.append(&clear_btn);
        col.append(&recent_row);
        {
            let s = self.clone();
            recent_spin.connect_value_changed(move |sp| {
                s.config
                    .borrow_mut()
                    .set_setting("max_recent_destinations", serde_json::json!(sp.value() as usize));
            });
        }
        {
            let s = self.clone();
            clear_btn.connect_clicked(move |_| {
                s.config.borrow_mut().clear_recent_destinations();
                s.notify("Cleared recent destinations");
            });
        }

        scrolled(&col)
    }

    fn populate_slot_rows(self: &Rc<Self>, rows: &gtk::Box) {
        while let Some(child) = rows.first_child() {
            rows.remove(&child);
        }
        let count = self.config.borrow().quickslot_count();
        for i in 0..count {
            rows.append(&self.build_slot_row(i));
        }
    }

    fn build_slot_row(self: &Rc<Self>, i: usize) -> gtk::Widget {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("ts-panel");
        set_padding(&row, 8);
        let num = gtk::Label::new(Some(&(i + 1).to_string()));
        num.add_css_class("ts-accent");
        num.add_css_class("ts-bold");
        let name = gtk::Entry::new();
        name.set_text(&self.config.borrow().quickslot_label(i));
        name.set_width_chars(18);
        let path_lbl = gtk::Label::new(Some(&{
            let p = self.config.borrow().quickslot_path(i);
            if p.is_empty() { "— not set —".to_string() } else { p }
        }));
        path_lbl.add_css_class("ts-muted");
        path_lbl.set_hexpand(true);
        path_lbl.set_xalign(0.0);
        path_lbl.set_ellipsize(pango::EllipsizeMode::Middle);
        let set_btn = button_with_label("Set folder", "folder-symbolic");
        set_btn.add_css_class("flat");
        let clear_btn = gtk::Button::from_icon_name("edit-clear-symbolic");
        clear_btn.add_css_class("flat");
        clear_btn.add_css_class("circular");

        {
            let s = self.clone();
            name.connect_changed(move |e| {
                s.config.borrow_mut().set_quickslot_label(i, &e.text());
                s.refresh_quickslots();
            });
        }
        {
            let s = self.clone();
            let n = i + 1;
            set_btn.connect_clicked(move |_| s.act_set_slot(n));
        }
        {
            let s = self.clone();
            let pl = path_lbl.clone();
            clear_btn.connect_clicked(move |_| {
                s.config.borrow_mut().set_quickslot_path(i, "");
                pl.set_text("— not set —");
                s.refresh_quickslots();
            });
        }

        row.append(&num);
        row.append(&name);
        row.append(&path_lbl);
        row.append(&set_btn);
        row.append(&clear_btn);
        row.upcast()
    }

    fn settings_theme(self: &Rc<Self>) -> gtk::Widget {
        let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
        set_padding(&col, 16);
        let info = gtk::Label::new(Some(
            "Colours apply live. Warm, dark values keep things readable at night.",
        ));
        info.add_css_class("ts-muted");
        info.set_xalign(0.0);
        info.set_wrap(true);
        col.append(&info);

        let grid = gtk::Grid::new();
        grid.set_row_spacing(8);
        grid.set_column_spacing(8);
        grid.set_column_homogeneous(true);
        let pairs = self.config.borrow().theme_pairs();
        for (i, (key, value)) in pairs.into_iter().enumerate() {
            let cell = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let lbl = gtk::Label::new(Some(&key));
            lbl.set_xalign(0.0);
            lbl.set_hexpand(true);
            let btn = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()));
            if let Ok(rgba) = gdk::RGBA::parse(&value) {
                btn.set_rgba(&rgba);
            }
            {
                let s = self.clone();
                let key = key.clone();
                btn.connect_rgba_notify(move |b| {
                    let c = b.rgba();
                    let hex = format!(
                        "#{:02x}{:02x}{:02x}",
                        (c.red() * 255.0) as u8,
                        (c.green() * 255.0) as u8,
                        (c.blue() * 255.0) as u8
                    );
                    s.config.borrow_mut().set_theme_color(&key, &hex);
                    s.apply_theme_live();
                });
            }
            cell.append(&lbl);
            cell.append(&btn);
            grid.attach(&cell, (i % 2) as i32, (i / 2) as i32, 1, 1);
        }
        col.append(&grid);
        scrolled(&col)
    }

    fn settings_raw(self: &Rc<Self>) -> gtk::Widget {
        let col = gtk::Box::new(gtk::Orientation::Vertical, 8);
        set_padding(&col, 16);
        let info = gtk::Label::new(Some(
            "The complete config file. Edit and Apply for full mpv-style control. \
             Invalid JSON is rejected.",
        ));
        info.add_css_class("ts-muted");
        info.set_wrap(true);
        info.set_xalign(0.0);
        col.append(&info);

        let text = gtk::TextView::new();
        text.set_monospace(true);
        text.add_css_class("ts-panel");
        let buffer = text.buffer();
        buffer.set_text(
            &serde_json::to_string_pretty(&self.config.borrow().data).unwrap_or_default(),
        );
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_vexpand(true);
        scroll.set_min_content_height(360);
        scroll.set_child(Some(&text));
        col.append(&scroll);

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let apply = button_with_label("Apply", "object-select-symbolic");
        apply.add_css_class("flat");
        let reload = button_with_label("Reload from disk", "view-refresh-symbolic");
        reload.add_css_class("flat");
        let path_lbl = gtk::Label::new(Some(&self.config.borrow().path.to_string_lossy()));
        path_lbl.add_css_class("ts-muted");
        path_lbl.add_css_class("ts-small");
        path_lbl.set_valign(gtk::Align::Center);
        row.append(&apply);
        row.append(&reload);
        row.append(&path_lbl);
        col.append(&row);

        {
            let s = self.clone();
            let buffer = buffer.clone();
            apply.connect_clicked(move |_| {
                let txt = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
                match serde_json::from_str::<serde_json::Value>(&txt) {
                    Ok(data) => {
                        s.config.borrow_mut().replace(data);
                        let v = s.config.borrow().get_f64("volume", s.volume.get());
                        s.volume.set(v);
                        s.apply_theme_live();
                        s.refresh_keybinds();
                        s.refresh_quickslots();
                        s.apply_metadata_visibility();
                        s.set_volume(v, false);
                        s.notify("Config applied");
                    }
                    Err(e) => s.notify(&format!("Invalid JSON: {e}")),
                }
            });
        }
        {
            let s = self.clone();
            let buffer = buffer.clone();
            reload.connect_clicked(move |_| {
                buffer.set_text(
                    &serde_json::to_string_pretty(&s.config.borrow().data).unwrap_or_default(),
                );
            });
        }

        col.upcast()
    }
}

fn open_folder_dialog(
    parent: &gtk::Window,
    start: &str,
    title: &str,
    on_choose: impl Fn(String) + 'static,
) {
    let start_path = if !start.is_empty() && Path::new(start).is_dir() {
        PathBuf::from(start)
    } else {
        PathBuf::from(home_dir())
    };
    let cwd = Rc::new(RefCell::new(
        start_path.canonicalize().unwrap_or(start_path),
    ));

    let win = gtk::Window::builder()
        .title(title)
        .transient_for(parent)
        .modal(true)
        .default_width(640)
        .default_height(520)
        .build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    set_padding(&root, 12);
    root.add_css_class("ts-card");

    let title_lbl = gtk::Label::new(Some(title));
    title_lbl.add_css_class("ts-accent");
    title_lbl.add_css_class("ts-brand");
    title_lbl.set_xalign(0.0);
    let path_lbl = gtk::Label::new(Some(&cwd.borrow().to_string_lossy()));
    path_lbl.add_css_class("ts-muted");
    path_lbl.set_xalign(0.0);
    path_lbl.set_ellipsize(pango::EllipsizeMode::Middle);

    let listing = gtk::ListBox::new();
    listing.add_css_class("ts-panel");
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_child(Some(&listing));

    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some("…or paste a path and press Enter"));
    entry.set_hexpand(true);
    let use_btn = button_with_label("Use this folder", "object-select-symbolic");
    use_btn.add_css_class("flat");
    let cancel_btn = gtk::Button::with_label("Cancel");
    cancel_btn.add_css_class("flat");
    let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bottom.append(&entry);
    bottom.append(&use_btn);
    bottom.append(&cancel_btn);

    root.append(&title_lbl);
    root.append(&path_lbl);
    root.append(&scroll);
    root.append(&bottom);
    win.set_child(Some(&root));

    let render: Rc<RefCell<Option<Box<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    {
        let listing = listing.clone();
        let path_lbl = path_lbl.clone();
        let cwd = cwd.clone();
        let render_ref = render.clone();
        let do_render = move || {
            path_lbl.set_text(&cwd.borrow().to_string_lossy());
            while let Some(child) = listing.first_child() {
                listing.remove(&child);
            }
            let here = cwd.borrow().clone();
            if let Some(parent) = here.parent() {
                if parent != here {
                    add_dir_row(&listing, "..", parent.to_path_buf(), &cwd, &render_ref);
                }
            }
            let mut entries: Vec<(String, PathBuf)> = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&here) {
                for e in rd.flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') {
                        continue;
                    }
                    if e.path().is_dir() {
                        entries.push((name, e.path()));
                    }
                }
            }
            entries.sort_by_key(|(n, _)| n.to_lowercase());
            for (name, path) in entries {
                add_dir_row(&listing, &name, path, &cwd, &render_ref);
            }
        };
        *render.borrow_mut() = Some(Box::new(do_render));
    }
    if let Some(f) = render.borrow().as_ref() {
        f();
    }

    {
        let cwd = cwd.clone();
        let render = render.clone();
        entry.connect_activate(move |e| {
            let p = PathBuf::from(e.text().to_string());
            if p.is_dir() {
                *cwd.borrow_mut() = p.canonicalize().unwrap_or(p);
                if let Some(f) = render.borrow().as_ref() {
                    f();
                }
            }
        });
    }

    let on_choose = Rc::new(on_choose);
    {
        let win = win.clone();
        let cwd = cwd.clone();
        let on_choose = on_choose.clone();
        use_btn.connect_clicked(move |_| {
            let chosen = cwd.borrow().to_string_lossy().to_string();
            win.close();
            on_choose(chosen);
        });
    }
    {
        let win = win.clone();
        cancel_btn.connect_clicked(move |_| win.close());
    }

    win.present();
}

#[allow(clippy::type_complexity)]
fn add_dir_row(
    listing: &gtk::ListBox,
    name: &str,
    path: PathBuf,
    cwd: &Rc<RefCell<PathBuf>>,
    render: &Rc<RefCell<Option<Box<dyn Fn()>>>>,
) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    set_padding(&row, 6);
    let icon = gtk::Image::from_icon_name("folder-symbolic");
    icon.add_css_class("ts-accent");
    let lbl = gtk::Label::new(Some(name));
    lbl.set_xalign(0.0);
    row.append(&icon);
    row.append(&lbl);
    let list_row = gtk::ListBoxRow::new();
    list_row.set_child(Some(&row));
    listing.append(&list_row);

    let cwd = cwd.clone();
    let render = render.clone();
    let gesture = gtk::GestureClick::new();
    gesture.connect_released(move |_, _, _, _| {
        if path.is_dir() {
            *cwd.borrow_mut() = path.canonicalize().unwrap_or_else(|_| path.clone());
            if let Some(f) = render.borrow().as_ref() {
                f();
            }
        }
    });
    list_row.add_controller(gesture);
}

fn set_padding(w: &impl IsA<gtk::Widget>, p: i32) {
    let w = w.as_ref();
    w.set_margin_top(p);
    w.set_margin_bottom(p);
    w.set_margin_start(p);
    w.set_margin_end(p);
}

fn expanding_spacer() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    b.set_hexpand(true);
    b
}

fn small_caps_label(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(&text.to_uppercase()));
    l.add_css_class("ts-muted");
    l.add_css_class("ts-small");
    l.set_xalign(0.0);
    l
}

fn icon_button(icon: &str, classes: &[&str]) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    for c in classes {
        b.add_css_class(c);
    }
    b.set_focusable(false);
    b
}

fn button_with_label(label: &str, icon: &str) -> gtk::Button {
    let b = gtk::Button::builder().build();
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&gtk::Image::from_icon_name(icon));
    content.append(&gtk::Label::new(Some(label)));
    b.set_child(Some(&content));
    b.set_focusable(false);
    b
}

fn labeled_spin(label: &str, value: f64, min: f64, max: f64, step: f64) -> (gtk::Box, gtk::SpinButton) {
    let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let lbl = gtk::Label::new(Some(label));
    lbl.add_css_class("ts-muted");
    lbl.add_css_class("ts-small");
    lbl.set_xalign(0.0);
    let spin = gtk::SpinButton::with_range(min, max, step);
    spin.set_digits(2);
    spin.set_value(value);
    col.append(&lbl);
    col.append(&spin);
    (col, spin)
}

fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::Widget {
    let s = gtk::ScrolledWindow::new();
    s.set_vexpand(true);
    s.set_hscrollbar_policy(gtk::PolicyType::Never);
    s.set_child(Some(child));
    s.upcast()
}

fn base_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

fn home_dir() -> String {
    dirs::home_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|| ".".to_string())
}

fn is_editable_focus(focus: Option<gtk::Widget>) -> bool {
    let mut cur = focus;
    while let Some(w) = cur {
        if w.is::<gtk::Text>() || w.is::<gtk::TextView>() {
            return true;
        }
        cur = w.parent();
    }
    false
}

fn is_modifier_key(k: gdk::Key) -> bool {
    use gdk::Key;
    matches!(
        k,
        Key::Shift_L | Key::Shift_R | Key::Control_L | Key::Control_R | Key::Alt_L | Key::Alt_R
            | Key::Meta_L | Key::Meta_R | Key::Super_L | Key::Super_R | Key::Hyper_L | Key::Hyper_R
            | Key::Caps_Lock | Key::Num_Lock | Key::ISO_Level3_Shift
    )
}

fn keystr_with_mods(state: gdk::ModifierType, code: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if state.contains(gdk::ModifierType::CONTROL_MASK) {
        parts.push("Ctrl");
    }
    if state.contains(gdk::ModifierType::SHIFT_MASK) {
        parts.push("Shift");
    }
    if state.contains(gdk::ModifierType::ALT_MASK) {
        parts.push("Alt");
    }
    if state.contains(gdk::ModifierType::SUPER_MASK) {
        parts.push("Meta");
    }
    let mut s = parts.join("+");
    if !s.is_empty() {
        s.push('+');
    }
    s.push_str(code);
    s
}

fn code_for(keyval: gdk::Key, keycode: u32) -> Option<String> {
    if let Some(code) = code_from_keycode(keycode) {
        return Some(code.to_string());
    }
    code_from_keyval(keyval)
}

fn code_from_keycode(keycode: u32) -> Option<&'static str> {
    let evdev = keycode.checked_sub(8)?;
    let s = match evdev {
        1 => "Escape",
        2 => "Digit1",
        3 => "Digit2",
        4 => "Digit3",
        5 => "Digit4",
        6 => "Digit5",
        7 => "Digit6",
        8 => "Digit7",
        9 => "Digit8",
        10 => "Digit9",
        11 => "Digit0",
        12 => "Minus",
        13 => "Equal",
        14 => "Backspace",
        15 => "Tab",
        16 => "KeyQ",
        17 => "KeyW",
        18 => "KeyE",
        19 => "KeyR",
        20 => "KeyT",
        21 => "KeyY",
        22 => "KeyU",
        23 => "KeyI",
        24 => "KeyO",
        25 => "KeyP",
        26 => "BracketLeft",
        27 => "BracketRight",
        28 => "Enter",
        29 => "ControlLeft",
        30 => "KeyA",
        31 => "KeyS",
        32 => "KeyD",
        33 => "KeyF",
        34 => "KeyG",
        35 => "KeyH",
        36 => "KeyJ",
        37 => "KeyK",
        38 => "KeyL",
        39 => "Semicolon",
        40 => "Quote",
        41 => "Backquote",
        42 => "ShiftLeft",
        43 => "Backslash",
        44 => "KeyZ",
        45 => "KeyX",
        46 => "KeyC",
        47 => "KeyV",
        48 => "KeyB",
        49 => "KeyN",
        50 => "KeyM",
        51 => "Comma",
        52 => "Period",
        53 => "Slash",
        54 => "ShiftRight",
        55 => "NumpadMultiply",
        56 => "AltLeft",
        57 => "Space",
        58 => "CapsLock",
        59 => "F1",
        60 => "F2",
        61 => "F3",
        62 => "F4",
        63 => "F5",
        64 => "F6",
        65 => "F7",
        66 => "F8",
        67 => "F9",
        68 => "F10",
        69 => "NumLock",
        70 => "ScrollLock",
        71 => "Numpad7",
        72 => "Numpad8",
        73 => "Numpad9",
        74 => "NumpadSubtract",
        75 => "Numpad4",
        76 => "Numpad5",
        77 => "Numpad6",
        78 => "NumpadAdd",
        79 => "Numpad1",
        80 => "Numpad2",
        81 => "Numpad3",
        82 => "Numpad0",
        83 => "NumpadDecimal",
        87 => "F11",
        88 => "F12",
        96 => "NumpadEnter",
        97 => "ControlRight",
        98 => "NumpadDivide",
        100 => "AltRight",
        102 => "Home",
        103 => "ArrowUp",
        104 => "PageUp",
        105 => "ArrowLeft",
        106 => "ArrowRight",
        107 => "End",
        108 => "ArrowDown",
        109 => "PageDown",
        110 => "Insert",
        111 => "Delete",
        125 => "MetaLeft",
        126 => "MetaRight",
        _ => return None,
    };
    Some(s)
}

fn code_from_keyval(keyval: gdk::Key) -> Option<String> {
    use gdk::Key;
    let s = match keyval {
        Key::space | Key::KP_Space => "Space",
        Key::Right => "ArrowRight",
        Key::Left => "ArrowLeft",
        Key::Up => "ArrowUp",
        Key::Down => "ArrowDown",
        Key::comma => "Comma",
        Key::period => "Period",
        Key::Return => "Enter",
        Key::Escape => "Escape",
        Key::BackSpace => "Backspace",
        Key::Tab => "Tab",
        Key::Delete => "Delete",

        Key::KP_0 => "Numpad0",
        Key::KP_1 => "Numpad1",
        Key::KP_2 => "Numpad2",
        Key::KP_3 => "Numpad3",
        Key::KP_4 => "Numpad4",
        Key::KP_5 => "Numpad5",
        Key::KP_6 => "Numpad6",
        Key::KP_7 => "Numpad7",
        Key::KP_8 => "Numpad8",
        Key::KP_9 => "Numpad9",

        Key::KP_Insert => "Numpad0",
        Key::KP_End => "Numpad1",
        Key::KP_Down => "Numpad2",
        Key::KP_Page_Down => "Numpad3",
        Key::KP_Left => "Numpad4",
        Key::KP_Begin => "Numpad5",
        Key::KP_Right => "Numpad6",
        Key::KP_Home => "Numpad7",
        Key::KP_Up => "Numpad8",
        Key::KP_Page_Up => "Numpad9",
        _ => {
            if let Some(c) = keyval.to_unicode() {
                if c.is_ascii_alphabetic() {
                    return Some(format!("Key{}", c.to_ascii_uppercase()));
                }
                if c.is_ascii_digit() {
                    return Some(format!("Digit{c}"));
                }
            }
            return None;
        }
    };
    Some(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keycode_maps_to_physical_code() {
        assert_eq!(code_from_keycode(40), Some("KeyD"));

        assert_eq!(code_from_keycode(10), Some("Digit1"));
        assert_eq!(code_from_keycode(14), Some("Digit5"));

        assert_eq!(code_from_keycode(113), Some("ArrowLeft"));
        assert_eq!(code_from_keycode(87), Some("Numpad1"));
        assert_eq!(code_from_keycode(65), Some("Space"));
    }

    #[test]
    fn keycode_out_of_range_falls_through() {
        assert_eq!(code_from_keycode(0), None);
        assert_eq!(code_from_keycode(7), None);
        assert_eq!(code_from_keycode(92), None);
    }
}
