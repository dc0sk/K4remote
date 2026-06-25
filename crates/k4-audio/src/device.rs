//! cpal device I/O (`device` feature): default output (RX playback) and input
//! (TX capture), bridged to the worker via [`SampleRing`]s.
//!
//! These wrap real hardware streams, so they are exercised at L4 (manual, with a
//! sound device), not in unit tests. Channel/rate adaptation uses
//! [`LinearResampler`]; only F32 device formats are supported (the common case).

use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;

use crate::resample::LinearResampler;
use crate::ring::SampleRing;

/// K4 audio sample rate (Hz).
pub const K4_RATE: u32 = 12_000;

/// Errors opening an audio device.
#[derive(Debug)]
pub enum AudioError {
    /// No default device of the requested direction.
    NoDevice,
    /// The device's default format is not F32.
    UnsupportedFormat,
    /// Configuration / stream-build failure.
    Config(String),
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::NoDevice => write!(f, "no default audio device"),
            AudioError::UnsupportedFormat => write!(f, "device default format is not f32"),
            AudioError::Config(e) => write!(f, "audio config error: {e}"),
        }
    }
}
impl std::error::Error for AudioError {}

/// Speaker output: accepts decoded 12 kHz stereo PCM and plays it.
pub struct AudioOutput {
    _stream: cpal::Stream,
    ring: Arc<Mutex<SampleRing>>,
    channels: usize,
    rs_left: LinearResampler,
    rs_right: LinearResampler,
}

impl AudioOutput {
    /// Open the default output device and start playback.
    pub fn new() -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(AudioError::NoDevice)?;
        let supported = device
            .default_output_config()
            .map_err(|e| AudioError::Config(e.to_string()))?;
        if supported.sample_format() != SampleFormat::F32 {
            return Err(AudioError::UnsupportedFormat);
        }
        let rate = supported.sample_rate().0;
        let channels = supported.channels() as usize;
        let config = supported.config();

        let ring = Arc::new(Mutex::new(SampleRing::new(rate as usize))); // ~1 s
        let cb = Arc::clone(&ring);
        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    if let Ok(mut r) = cb.lock() {
                        for s in data.iter_mut() {
                            *s = r.pop().unwrap_or(0.0);
                        }
                    }
                },
                |err| eprintln!("audio output stream error: {err}"),
                None,
            )
            .map_err(|e| AudioError::Config(e.to_string()))?;
        stream
            .play()
            .map_err(|e| AudioError::Config(e.to_string()))?;

        Ok(Self {
            _stream: stream,
            ring,
            channels,
            rs_left: LinearResampler::new(K4_RATE, rate),
            rs_right: LinearResampler::new(K4_RATE, rate),
        })
    }

    /// Submit interleaved 12 kHz stereo PCM (L = Main, R = Sub) for playback.
    pub fn submit_stereo_12k(&mut self, pcm: &[f32]) {
        let mut left = Vec::with_capacity(pcm.len() / 2);
        let mut right = Vec::with_capacity(pcm.len() / 2);
        for frame in pcm.chunks_exact(2) {
            left.push(frame[0]);
            right.push(frame[1]);
        }
        let mut l = Vec::new();
        let mut r = Vec::new();
        self.rs_left.process(&left, &mut l);
        self.rs_right.process(&right, &mut r);

        let frames = l.len().min(r.len());
        let mut out = Vec::with_capacity(frames * self.channels);
        for i in 0..frames {
            match self.channels {
                1 => out.push((l[i] + r[i]) * 0.5),
                _ => {
                    out.push(l[i]);
                    out.push(r[i]);
                    // Silence any extra device channels beyond stereo.
                    out.resize(out.len() + self.channels - 2, 0.0);
                }
            }
        }
        if let Ok(mut rb) = self.ring.lock() {
            rb.push_slice(&out);
        }
    }
}

/// Microphone input: captures audio and exposes 12 kHz mono frames.
pub struct AudioInput {
    _stream: cpal::Stream,
    ring: Arc<Mutex<SampleRing>>,
}

impl AudioInput {
    /// Open the default input device and start capture.
    pub fn new() -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or(AudioError::NoDevice)?;
        let supported = device
            .default_input_config()
            .map_err(|e| AudioError::Config(e.to_string()))?;
        if supported.sample_format() != SampleFormat::F32 {
            return Err(AudioError::UnsupportedFormat);
        }
        let rate = supported.sample_rate().0;
        let channels = supported.channels().max(1) as usize;
        let config = supported.config();

        let ring = Arc::new(Mutex::new(SampleRing::new(K4_RATE as usize))); // ~1 s of 12 k
        let cb = Arc::clone(&ring);
        let mut resampler = LinearResampler::new(rate, K4_RATE);
        let stream = device
            .build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    // Down-mix interleaved frames to mono, then resample to 12 kHz.
                    let mut mono = Vec::with_capacity(data.len() / channels);
                    for frame in data.chunks(channels) {
                        mono.push(frame.iter().sum::<f32>() / channels as f32);
                    }
                    let mut out = Vec::new();
                    resampler.process(&mono, &mut out);
                    if let Ok(mut rb) = cb.lock() {
                        rb.push_slice(&out);
                    }
                },
                |err| eprintln!("audio input stream error: {err}"),
                None,
            )
            .map_err(|e| AudioError::Config(e.to_string()))?;
        stream
            .play()
            .map_err(|e| AudioError::Config(e.to_string()))?;

        Ok(Self {
            _stream: stream,
            ring,
        })
    }

    /// Take exactly `n` mono 12 kHz samples (one TX frame) if enough are buffered.
    pub fn take_frame(&self, n: usize) -> Option<Vec<f32>> {
        let mut r = self.ring.lock().ok()?;
        if r.len() < n {
            return None;
        }
        let mut frame = Vec::with_capacity(n);
        for _ in 0..n {
            frame.push(r.pop()?);
        }
        Some(frame)
    }
}
