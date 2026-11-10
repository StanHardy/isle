//! WASAPI loopback spectrum analyzer.
//!
//! Captures whatever is playing on the default render endpoint (loopback),
//! runs a 2048-point FFT and emits 24 log-spaced bands, dB-mapped with a
//! spectral tilt so mastered music doesn't saturate the low end — the same
//! mapping the Electron helper verified live:
//!   db = 20*log10(mag) + band*0.32, mapped [-58,-10]dB → [0,1], ^0.85.
//!
//! Runs on its own MTA thread and re-initializes on device invalidation.

use std::f32::consts::PI;
use std::time::{Duration, Instant};

use windows::Win32::Media::Audio::{
    eConsole, eRender, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_LOOPBACK, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
    MMDeviceEnumerator, WAVEFORMATEXTENSIBLE,
};

use crate::events::{self, ev};

const FFT_SIZE: usize = 2048;
const BANDS: usize = 24;
const F_MIN: f32 = 40.0;
const F_MAX: f32 = 16_000.0;

fn ensure_com() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_MULTITHREADED,
        );
    }
}

pub fn start(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        ensure_com();
        loop {
            if let Err(err) = run(&app) {
                eprintln!("[spectrum] capture failed: {err} — retrying in 2s");
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

/// One capture session; returns on device errors so the outer loop restarts.
fn run(app: &tauri::AppHandle) -> windows::core::Result<()> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            windows::Win32::System::Com::CoCreateInstance(&MMDeviceEnumerator, None, windows::Win32::System::Com::CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let client: IAudioClient = device.Activate(windows::Win32::System::Com::CLSCTX_ALL, None)?;
        let fmt_ptr = client.GetMixFormat()?;
        let fmt = &*fmt_ptr;
        let channels = fmt.nChannels as usize;
        let sample_rate = fmt.nSamplesPerSec as f32;
        let is_float = fmt.wFormatTag == 3
            || (fmt.wFormatTag == 0xFFFE && {
                // WAVEFORMATEXTENSIBLE.SubFormat at offset 24 == IEEE float.
                // The struct is packed — copy the GUID out before comparing.
                let ext = &*(fmt_ptr as *const WAVEFORMATEXTENSIBLE);
                let sub = { ext.SubFormat };
                sub == windows::core::GUID::from_u128(
                    0x00000003_0000_0010_8000_00aa00389b71,
                )
            });

        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK,
            0,
            0,
            fmt_ptr,
            None,
        )?;
        let capture: IAudioCaptureClient = client.GetService()?;
        client.Start()?;

        let mut acc: Vec<f32> = Vec::with_capacity(FFT_SIZE * 8);
        let mut last_packet: Instant = Instant::now();
        let mut silence_emitted = true;

        loop {
            std::thread::sleep(Duration::from_millis(12));

            // Drain available packets.
            loop {
                let frames = capture.GetNextPacketSize()?;
                if frames == 0 {
                    break;
                }
                let mut data = std::ptr::null_mut::<u8>();
                let mut read = 0u32;
                let mut flags = 0u32;
                capture.GetBuffer(&mut data, &mut read, &mut flags, None, None)?;
                last_packet = Instant::now();
                silence_emitted = false;
                if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) == 0 && read > 0 {
                    let count = read as usize * channels;
                    if is_float {
                        let f32s = std::slice::from_raw_parts(data as *const f32, count);
                        push_mono(&mut acc, f32s, channels);
                    } else {
                        let i16s = std::slice::from_raw_parts(data as *const i16, count);
                        acc.extend(
                            i16s.iter()
                                .step_by(channels)
                                .map(|s| *s as f32 / 32768.0),
                        );
                    }
                }
                capture.ReleaseBuffer(read)?;
                if acc.len() > FFT_SIZE * 8 {
                    // Keep the freshest window; avoid unbounded backlog.
                    let keep_from = acc.len() - FFT_SIZE * 4;
                    acc.drain(..keep_from);
                }
            }

            // Emit when we have a full window.
            while acc.len() >= FFT_SIZE {
                let bands = compute_bands(&acc[..FFT_SIZE], sample_rate);
                events::emit(app, ev::SPECTRUM, &bands);
                acc.drain(..FFT_SIZE / 2); // 50% overlap keeps bars smooth
            }

            // Silence: zero the bars once so they settle instead of freezing.
            if !silence_emitted && last_packet.elapsed() > Duration::from_millis(180) {
                events::emit(app, ev::SPECTRUM, &[0f32; BANDS]);
                silence_emitted = true;
                acc.clear();
            }
        }
    }
}

/// Downmix one packet to mono.
fn push_mono(acc: &mut Vec<f32>, samples: &[f32], channels: usize) {
    if channels <= 1 {
        acc.extend_from_slice(samples);
        return;
    }
    let mut i = 0;
    while i + channels <= samples.len() {
        let mut sum = 0.0f32;
        for c in 0..channels {
            sum += samples[i + c];
        }
        acc.push(sum / channels as f32);
        i += channels;
    }
}

/// Hann window + iterative radix-2 FFT, then log-spaced band magnitudes.
fn compute_bands(samples: &[f32], sample_rate: f32) -> [f32; BANDS] {
    let mut re = [0f32; FFT_SIZE];
    let mut im = [0f32; FFT_SIZE];
    for i in 0..FFT_SIZE {
        let w = 0.5 * (1.0 - (2.0 * PI * i as f32 / (FFT_SIZE - 1) as f32).cos());
        re[i] = samples[i] * w;
    }

    // Bit-reverse permutation.
    let mut j = 0usize;
    for i in 0..FFT_SIZE {
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
        let mut m = FFT_SIZE >> 1;
        while m >= 1 && j & m != 0 {
            j ^= m;
            m >>= 1;
        }
        j |= m;
    }

    // Butterflies.
    let mut len = 2usize;
    while len <= FFT_SIZE {
        let ang = -2.0 * PI / len as f32;
        let (wr0, wi0) = (ang.cos(), ang.sin());
        let step = len / 2;
        let mut start = 0usize;
        while start < FFT_SIZE {
            let (mut wr, mut wi) = (1.0f32, 0.0f32);
            for k in 0..step {
                let i1 = start + k;
                let i2 = i1 + step;
                let tr = re[i2] * wr - im[i2] * wi;
                let ti = re[i2] * wi + im[i2] * wr;
                re[i2] = re[i1] - tr;
                im[i2] = im[i1] - ti;
                re[i1] += tr;
                im[i1] += ti;
                let nwr = wr * wr0 - wi * wi0;
                wi = wr * wi0 + wi * wr0;
                wr = nwr;
            }
            start += len;
        }
        len <<= 1;
    }

    // Log-spaced band edges 40 Hz → 16 kHz.
    let nyquist = sample_rate / 2.0;
    let bins = FFT_SIZE / 2;
    let mut out = [0f32; BANDS];
    let mut lo = 0usize;
    for b in 0..BANDS {
        let hi_f = F_MIN * (F_MAX / F_MIN).powf((b + 1) as f32 / BANDS as f32);
        let mut hi = ((hi_f / nyquist) * bins as f32).round() as usize;
        if hi <= lo {
            hi = lo + 1;
        }
        if hi > bins {
            hi = bins;
        }
        let mut sum = 0f32;
        for k in lo..hi {
            sum += (re[k] * re[k] + im[k] * im[k]).sqrt();
        }
        let mag = sum / (hi - lo) as f32;
        // Proven mapping from the Electron helper: tilt + dB window + curve.
        let db = 20.0 * mag.max(1e-9).log10() + b as f32 * 0.32;
        out[b] = (((db + 58.0) / 48.0).clamp(0.0, 1.0)).powf(0.85);
        lo = hi;
        if lo >= bins {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_gives_zero_bands() {
        let out = compute_bands(&[0f32; FFT_SIZE], 48000.0);
        assert!(out.iter().all(|v| *v < 0.01));
    }

    #[test]
    fn loud_low_tone_lights_low_bands() {
        // 100 Hz sine at full scale.
        let mut s = [0f32; FFT_SIZE];
        for (i, v) in s.iter_mut().enumerate() {
            *v = (2.0 * PI * 100.0 * i as f32 / 48000.0).sin();
        }
        let out = compute_bands(&s, 48000.0);
        let low = out[..4].iter().sum::<f32>() / 4.0;
        let high = out[20..].iter().sum::<f32>() / 4.0;
        assert!(low > 0.2, "low bands should react, low={low}");
        assert!(low > high, "energy should concentrate in low bands");
    }

    #[test]
    fn bands_are_normalized() {
        let mut s = [0f32; FFT_SIZE];
        for (i, v) in s.iter_mut().enumerate() {
            *v = (2.0 * PI * 440.0 * i as f32 / 48000.0).sin();
        }
        let out = compute_bands(&s, 48000.0);
        assert!(out.iter().all(|v| (0.0..=1.0).contains(v)));
    }
}
