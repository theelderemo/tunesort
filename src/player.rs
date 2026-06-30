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

use gstreamer as gst;
use gstreamer::glib;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const SPECTRUM_BANDS: u32 = 64;
const SPECTRUM_THRESHOLD: i32 = -80;
const PEAK_BUCKETS: usize = 1600;
const PEAK_RATE: i32 = 8000;

pub enum PlayerEvent {
    Ready { duration: f64 },
    Time { position: f64, duration: f64 },
    Play,
    Pause,
    Ended,

    Peaks { id: u64, peaks: Vec<f32> },

    Spectrum(Vec<f32>),
}

pub struct Player {
    playbin: gst::Element,
    spectrum: gst::Element,
    tx: async_channel::Sender<PlayerEvent>,
    want_play: Rc<Cell<bool>>,

    load_id: Arc<AtomicU64>,
    pub visualizer_on: Rc<Cell<bool>>,

    _bus_guard: gst::bus::BusWatchGuard,
}

impl Player {
    pub fn new(tx: async_channel::Sender<PlayerEvent>) -> Player {
        gst::init().expect("failed to initialise GStreamer");

        let playbin = gst::ElementFactory::make("playbin")
            .build()
            .expect("playbin missing (install gstreamer1.0-plugins-base)");

        let spectrum = gst::ElementFactory::make("spectrum")
            .property("bands", SPECTRUM_BANDS)
            .property("threshold", SPECTRUM_THRESHOLD)
            .property("post-messages", false)
            .property("interval", 30_000_000u64)
            .property("multi-channel", false)
            .build()
            .expect("spectrum element missing (install gstreamer1.0-plugins-good)");

        playbin.set_property("audio-filter", &spectrum);

        let want_play = Rc::new(Cell::new(false));
        let bus_guard = attach_bus_watch(&playbin, tx.clone(), want_play.clone());

        let player = Player {
            playbin: playbin.clone(),
            spectrum,
            tx: tx.clone(),
            want_play,
            load_id: Arc::new(AtomicU64::new(0)),
            visualizer_on: Rc::new(Cell::new(false)),
            _bus_guard: bus_guard,
        };

        player.attach_time_pump();
        player
    }

    fn attach_time_pump(&self) {
        let playbin = self.playbin.clone();
        let tx = self.tx.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
            let position = playbin
                .query_position::<gst::ClockTime>()
                .map(|p| p.nseconds() as f64 / 1e9)
                .unwrap_or(0.0);
            let duration = playbin
                .query_duration::<gst::ClockTime>()
                .map(|d| d.nseconds() as f64 / 1e9)
                .unwrap_or(0.0);
            let _ = tx.try_send(PlayerEvent::Time { position, duration });
            glib::ControlFlow::Continue
        });
    }

    pub fn load(&self, path: &Path, autoplay: bool) {
        let _ = self.playbin.set_state(gst::State::Null);
        if let Ok(uri) = glib::filename_to_uri(path, None) {
            self.playbin.set_property("uri", uri.as_str());
        }
        self.want_play.set(autoplay);
        let target = if autoplay { gst::State::Playing } else { gst::State::Paused };
        let _ = self.playbin.set_state(target);
        let id = self.load_id.fetch_add(1, Ordering::SeqCst) + 1;
        self.spawn_peaks(path, id);
    }

    pub fn current_load_id(&self) -> u64 {
        self.load_id.load(Ordering::SeqCst)
    }

    pub fn play(&self) {
        self.want_play.set(true);
        let _ = self.playbin.set_state(gst::State::Playing);
    }

    pub fn pause(&self) {
        self.want_play.set(false);
        let _ = self.playbin.set_state(gst::State::Paused);
    }

    pub fn play_pause(&self) {
        if self.is_playing() {
            self.pause();
        } else {
            self.play();
        }
    }

    pub fn stop(&self) {
        self.want_play.set(false);
        let _ = self.playbin.set_state(gst::State::Null);
    }

    pub fn is_playing(&self) -> bool {
        self.playbin.current_state() == gst::State::Playing
    }

    pub fn set_volume(&self, v: f64) {
        self.playbin.set_property("volume", v.clamp(0.0, 1.0));
    }

    pub fn set_muted(&self, m: bool) {
        self.playbin.set_property("mute", m);
    }

    pub fn seek_relative(&self, delta: f64) {
        let pos = match self.playbin.query_position::<gst::ClockTime>() {
            Some(p) => p.nseconds() as f64 / 1e9,
            None => return,
        };
        let mut t = pos + delta;
        if let Some(d) = self.playbin.query_duration::<gst::ClockTime>() {
            t = t.clamp(0.0, d.nseconds() as f64 / 1e9);
        } else {
            t = t.max(0.0);
        }
        self.seek_to(t);
    }

    pub fn seek_to(&self, seconds: f64) {
        let target = gst::ClockTime::from_nseconds((seconds.max(0.0) * 1e9) as u64);
        let _ = self.playbin.seek_simple(
            gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
            target,
        );
    }

    pub fn set_visualizer(&self, on: bool) {
        self.visualizer_on.set(on);
        self.spectrum.set_property("post-messages", on);
    }

    fn spawn_peaks(&self, path: &Path, id: u64) {
        let path = path.to_path_buf();
        let tx = self.tx.clone();
        let load_id = self.load_id.clone();
        std::thread::spawn(move || {
            let mut peaks = decode_peaks(&path, &load_id, id).unwrap_or_default();
            if peaks.is_empty() && crate::library::is_midi(&path) {
                peaks = midi_note_density(&path);
            }
            if load_id.load(Ordering::SeqCst) == id {
                let _ = tx.try_send(PlayerEvent::Peaks { id, peaks });
            }
        });
    }
}

fn attach_bus_watch(
    playbin: &gst::Element,
    tx: async_channel::Sender<PlayerEvent>,
    want_play: Rc<Cell<bool>>,
) -> gst::bus::BusWatchGuard {
    let bus = playbin.bus().expect("playbin has no bus");
    let playbin = playbin.clone();
    bus.add_watch_local(move |_, msg| {
        use gst::MessageView;
        match msg.view() {
            MessageView::Eos(_) => {
                let _ = tx.try_send(PlayerEvent::Ended);
            }
            MessageView::AsyncDone(_) | MessageView::DurationChanged(_) => {
                let duration = playbin
                    .query_duration::<gst::ClockTime>()
                    .map(|d| d.nseconds() as f64 / 1e9)
                    .unwrap_or(0.0);
                let _ = tx.try_send(PlayerEvent::Ready { duration });
                if want_play.get() {
                    let _ = playbin.set_state(gst::State::Playing);
                }
            }
            MessageView::StateChanged(sc) => {
                if msg.src() == Some(playbin.upcast_ref::<gst::Object>()) {
                    match sc.current() {
                        gst::State::Playing => {
                            let _ = tx.try_send(PlayerEvent::Play);
                        }
                        gst::State::Paused | gst::State::Ready => {
                            let _ = tx.try_send(PlayerEvent::Pause);
                        }
                        _ => {}
                    }
                }
            }
            MessageView::Element(el) => {
                if let Some(s) = el.structure() {
                    if s.name() == "spectrum" {
                        if let Some(mags) = read_spectrum(s) {
                            let _ = tx.try_send(PlayerEvent::Spectrum(mags));
                        }
                    }
                }
            }
            MessageView::Error(err) => {
                eprintln!("gstreamer: {} ({:?})", err.error(), err.debug());
            }
            _ => {}
        }
        glib::ControlFlow::Continue
    })
    .expect("failed to add bus watch")
}

fn read_spectrum(s: &gst::StructureRef) -> Option<Vec<f32>> {
    let list = s.get::<gst::List>("magnitude").ok()?;
    let span = -(SPECTRUM_THRESHOLD as f32);
    let out: Vec<f32> = list
        .iter()
        .filter_map(|v| v.get::<f32>().ok())
        .map(|db| ((db - SPECTRUM_THRESHOLD as f32) / span).clamp(0.0, 1.0))
        .collect();
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn midi_note_density(path: &Path) -> Vec<f32> {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    if data.len() > 50 * 1024 * 1024 {
        return Vec::new();
    }
    let smf = match midly::Smf::parse(&data) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let mut note_ticks: Vec<u64> = Vec::new();
    for track in &smf.tracks {
        let mut abs_tick: u64 = 0;
        for event in track {
            abs_tick = abs_tick.saturating_add(event.delta.as_int() as u64);
            if let midly::TrackEventKind::Midi {
                message: midly::MidiMessage::NoteOn { vel, .. },
                ..
            } = &event.kind
            {
                if vel.as_int() > 0 {
                    note_ticks.push(abs_tick);
                }
            }
        }
    }
    if note_ticks.is_empty() {
        return Vec::new();
    }
    let max_tick = note_ticks.iter().copied().max().unwrap_or(1).max(1);
    let mut counts = vec![0u32; PEAK_BUCKETS];
    for &tick in &note_ticks {
        let idx = ((tick as f64 / max_tick as f64) * (PEAK_BUCKETS - 1) as f64) as usize;
        counts[idx.min(PEAK_BUCKETS - 1)] =
            counts[idx.min(PEAK_BUCKETS - 1)].saturating_add(1);
    }
    let max_count = counts.iter().copied().max().unwrap_or(1).max(1) as f32;
    counts.iter().map(|&c| c as f32 / max_count).collect()
}

fn decode_peaks(path: &Path, load_id: &AtomicU64, id: u64) -> Option<Vec<f32>> {
    let uri = glib::filename_to_uri(path, None).ok()?;

    let pipeline = gst::Pipeline::new();
    let src = gst::ElementFactory::make("uridecodebin")
        .property("uri", uri.as_str())
        .build()
        .ok()?;
    let convert = gst::ElementFactory::make("audioconvert").build().ok()?;
    let resample = gst::ElementFactory::make("audioresample").build().ok()?;
    let caps = gst::Caps::builder("audio/x-raw")
        .field("format", "S16LE")
        .field("channels", 1i32)
        .field("rate", PEAK_RATE)
        .field("layout", "interleaved")
        .build();
    let appsink = gst_app::AppSink::builder()
        .caps(&caps)
        .sync(false)
        .max_buffers(8)
        .drop(false)
        .build();

    pipeline
        .add_many([&src, &convert, &resample, appsink.upcast_ref::<gst::Element>()])
        .ok()?;
    gst::Element::link_many([&convert, &resample, appsink.upcast_ref::<gst::Element>()]).ok()?;

    let convert_weak = convert.downgrade();
    src.connect_pad_added(move |_, src_pad| {
        if let Some(convert) = convert_weak.upgrade() {
            if let Some(sink_pad) = convert.static_pad("sink") {
                if !sink_pad.is_linked() {
                    let _ = src_pad.link(&sink_pad);
                }
            }
        }
    });

    if pipeline.set_state(gst::State::Playing).is_err() {
        let _ = pipeline.set_state(gst::State::Null);
        return None;
    }

    let mut samples: Vec<f32> = Vec::new();
    loop {
        if load_id.load(Ordering::SeqCst) != id {
            let _ = pipeline.set_state(gst::State::Null);
            return None;
        }
        match appsink.pull_sample() {
            Ok(sample) => {
                if let Some(buffer) = sample.buffer() {
                    if let Ok(map) = buffer.map_readable() {
                        for chunk in map.as_slice().chunks_exact(2) {
                            let v = i16::from_le_bytes([chunk[0], chunk[1]]) as f32 / 32768.0;
                            samples.push(v.abs());
                        }
                    }
                }
            }
            Err(_) => break,
        }
    }
    let _ = pipeline.set_state(gst::State::Null);

    if samples.is_empty() {
        return Some(Vec::new());
    }

    let buckets = PEAK_BUCKETS.min(samples.len());
    let per = (samples.len() as f64 / buckets as f64).max(1.0);
    let mut peaks: Vec<f32> = Vec::with_capacity(buckets);
    let mut max = 0.0f32;
    for i in 0..buckets {
        let start = (i as f64 * per) as usize;
        let end = (((i + 1) as f64 * per) as usize).min(samples.len());
        let mut peak = 0.0f32;
        for &s in &samples[start..end.max(start + 1).min(samples.len())] {
            if s > peak {
                peak = s;
            }
        }
        if peak > max {
            max = peak;
        }
        peaks.push(peak);
    }
    if max > 0.0 {
        for p in peaks.iter_mut() {
            *p /= max;
        }
    }
    Some(peaks)
}
