//! Microphone capture for dictation.
//!
//! The input stream exists only while a take is running. That costs ~100ms at
//! the start of a recording, and buys the thing an always-on capture loop can
//! never give back: when Commons is not recording, macOS shows no microphone
//! indicator, because the microphone is genuinely closed.
//!
//! cpal's `Stream` is not `Send` on macOS, so it lives on a thread of its own
//! and is driven by commands. The device callback only pushes into a lock-free
//! ring buffer; that same thread drains it into the take.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::HeapRb;

enum Cmd {
    Start,
    Stop,
}

#[derive(Default)]
struct Take {
    samples: Vec<f32>,
    sample_rate: u32,
    /// RMS of the most recent chunk, for the level meter.
    level: f32,
    /// Set when the device could not be opened, so the UI can say why.
    error: Option<String>,
}

pub struct Recorder {
    tx: Sender<Cmd>,
    take: Arc<Mutex<Take>>,
    recording: Arc<AtomicBool>,
}

impl Recorder {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let take = Arc::new(Mutex::new(Take::default()));
        let recording = Arc::new(AtomicBool::new(false));
        {
            let take = take.clone();
            let recording = recording.clone();
            std::thread::spawn(move || run(rx, take, recording));
        }
        Recorder {
            tx,
            take,
            recording,
        }
    }

    /// Begins a take. Returns as soon as the request is queued; the stream is
    /// live a moment later, which `is_recording` reports.
    pub fn start(&self) -> Result<(), String> {
        if self.recording.load(Ordering::SeqCst) {
            return Ok(());
        }
        {
            let mut t = self.take.lock().unwrap();
            t.samples.clear();
            t.level = 0.0;
            t.error = None;
        }
        self.tx
            .send(Cmd::Start)
            .map_err(|_| "recorder thread is gone".to_string())
    }

    /// Ends the take and returns its samples with the rate they were captured
    /// at. `None` when nothing was captured.
    pub fn stop(&self) -> Option<(Vec<f32>, u32)> {
        let _ = self.tx.send(Cmd::Stop);
        // The stream thread clears the flag once the device is closed; waiting
        // for it means the returned take includes every sample it captured.
        for _ in 0..200 {
            if !self.recording.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut t = self.take.lock().unwrap();
        let rate = t.sample_rate;
        let samples = std::mem::take(&mut t.samples);
        if samples.is_empty() || rate == 0 {
            None
        } else {
            Some((samples, rate))
        }
    }

    pub fn is_recording(&self) -> bool {
        self.recording.load(Ordering::SeqCst)
    }

    pub fn level(&self) -> f32 {
        self.take.lock().unwrap().level
    }

    pub fn error(&self) -> Option<String> {
        self.take.lock().unwrap().error.clone()
    }
}

fn run(rx: Receiver<Cmd>, take: Arc<Mutex<Take>>, recording: Arc<AtomicBool>) {
    while let Ok(cmd) = rx.recv() {
        if !matches!(cmd, Cmd::Start) {
            continue;
        }
        if let Err(e) = capture(&rx, &take, &recording) {
            eprintln!("[audio] {e}");
            take.lock().unwrap().error = Some(e);
            recording.store(false, Ordering::SeqCst);
        }
    }
}

fn capture(
    rx: &Receiver<Cmd>,
    take: &Arc<Mutex<Take>>,
    recording: &Arc<AtomicBool>,
) -> Result<(), String> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or("no microphone is available")?;
    let supported = device
        .default_input_config()
        .map_err(|e| format!("microphone has no usable config: {e}"))?;

    let sample_rate = supported.sample_rate();
    let channels = supported.channels() as usize;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();

    take.lock().unwrap().sample_rate = sample_rate;

    let rb = HeapRb::<f32>::new(sample_rate as usize * 4);
    let (mut prod, mut cons) = rb.split();
    let err_fn = |e| eprintln!("[audio] stream error: {e}");
    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                push_mono(&mut prod, data, channels, |s| s);
            },
            err_fn,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                push_mono(&mut prod, data, channels, |s| s as f32 / 32768.0);
            },
            err_fn,
            None,
        ),
        other => return Err(format!("unsupported sample format: {other:?}")),
    }
    .map_err(|e| format!("could not open the microphone: {e}"))?;

    stream
        .play()
        .map_err(|e| format!("could not start the microphone: {e}"))?;
    recording.store(true, Ordering::SeqCst);

    let mut chunk = vec![0f32; 4096];
    loop {
        match rx.recv_timeout(Duration::from_millis(8)) {
            Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            // A Start while already recording is the UI double-firing; ignore.
            Ok(Cmd::Start) => {}
            Err(RecvTimeoutError::Timeout) => {}
        }
        drain(&mut cons, &mut chunk, take);
    }

    // Closing the device can take a beat; drain what it already handed us.
    drop(stream);
    drain(&mut cons, &mut chunk, take);
    recording.store(false, Ordering::SeqCst);
    Ok(())
}

fn drain<C: Consumer<Item = f32>>(cons: &mut C, chunk: &mut [f32], take: &Arc<Mutex<Take>>) {
    loop {
        let n = cons.pop_slice(chunk);
        if n == 0 {
            return;
        }
        let samples = &chunk[..n];
        let rms = (samples.iter().map(|s| s * s).sum::<f32>() / n as f32).sqrt();
        let mut t = take.lock().unwrap();
        t.level = rms;
        t.samples.extend_from_slice(samples);
    }
}

/// Downmixes interleaved frames to mono and pushes them into the ring buffer.
fn push_mono<S: Copy, P: Producer<Item = f32>>(
    prod: &mut P,
    data: &[S],
    channels: usize,
    to_f32: impl Fn(S) -> f32,
) {
    if channels <= 1 {
        for &s in data {
            let _ = prod.try_push(to_f32(s));
        }
        return;
    }
    for frame in data.chunks_exact(channels) {
        let sum: f32 = frame.iter().map(|&s| to_f32(s)).sum();
        let _ = prod.try_push(sum / channels as f32);
    }
}

/// Writes a mono take to a 16-bit WAV, which is what the sidecar reads.
pub fn write_wav(path: &std::path::Path, samples: &[f32], sample_rate: u32) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    for &s in samples {
        let clamped = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer.write_sample(clamped).map_err(|e| e.to_string())?;
    }
    writer.finalize().map_err(|e| e.to_string())
}
