//! Print the audio devices this application can see, and which one it would
//! use by default.
//!
//! Diagnostic aid: an operator reporting "no sound" needs to know *where* the
//! audio is being sent, and the app's own device list is the only authority on
//! that. Run with:
//!
//! ```text
//! cargo run -p k4-audio --example list_audio --features device
//! ```
fn main() {
    println!("-- output (speaker) devices --");
    for n in k4_audio::output_device_names() {
        println!("  {n}");
    }
    println!("\n-- input (mic) devices --");
    for n in k4_audio::input_device_names() {
        println!("  {n}");
    }
    println!("\n-- what the app opens when no device is chosen --");
    match k4_audio::AudioOutput::with_device(None) {
        Ok(_) => println!("  default output opened OK"),
        Err(e) => println!("  default output FAILED: {e}"),
    }
}
