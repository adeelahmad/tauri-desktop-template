//! `audio::read_wav`: WAV bytes from a file the user picked or the web view sent.
#![no_main]

use desktop_core::audio::{read_wav, resample, MAX_RATE, MIN_RATE};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok((samples, rate)) = read_wav(data) {
        assert!((MIN_RATE..=MAX_RATE).contains(&rate));
        assert!(samples.len() <= data.len());
        // What the app does next: resample to Whisper's 16 kHz. Kept short so a run stays fast.
        let n = samples.len().min(4096);
        let out = resample(&samples[..n], rate, 16_000);
        assert!(out.len() <= n * 16 + 1);
    }
});
