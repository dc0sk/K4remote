//! Play a short tone through the **application's own** playback path.
//!
//! Diagnostic aid for "no sound": this uses `AudioOutput::submit_stereo_12k`,
//! exactly as received radio audio does, so it separates "the audio path is
//! broken or misrouted" from "no audio is arriving from the radio". If you
//! hear this, the path works.
//!
//! ```text
//! cargo run -p k4-audio --example test_tone --features device -- [device-name]
//! ```
use std::f32::consts::TAU;

fn main() {
    let name = std::env::args().nth(1);
    println!("opening: {}", name.as_deref().unwrap_or("system default"));
    let mut out = match k4_audio::AudioOutput::with_device(name.as_deref()) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("FAILED to open output: {e}");
            std::process::exit(1);
        }
    };
    // 12 kHz stereo, as the radio sends: 1 kHz in the left (Main), 600 Hz in
    // the right (Sub), so the channel split is audible too.
    let secs = 8.0;
    let n = (12_000.0 * secs) as usize;
    let mut pcm = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f32 / 12_000.0;
        pcm.push((TAU * 1000.0 * t).sin() * 0.25); // L = Main
        pcm.push((TAU * 600.0 * t).sin() * 0.25); // R = Sub
    }
    // Feed it in 20 ms blocks, as the decoder does.
    for block in pcm.chunks(240 * 2) {
        out.submit_stereo_12k(block);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    println!("done — 1 kHz left (Main), 600 Hz right (Sub)");
    std::thread::sleep(std::time::Duration::from_millis(500));
}
