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
        let mut agc_peak = 5e-4f32;

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
                let bands = compute_bands(&acc[..FFT_SIZE], sample_rate, &mut agc_peak);
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

/// Hann window + iterative radix-2 FFT, then log-spaced band magnitudes,
/// normalized by an auto-gain peak (instant attack, ~2.5 s release) so the
/// bars show relative dynamics and can never pin flat at maximum — the
/// absolute-dB window tried first saturated on ordinary music.
fn compute_bands(samples: &[f32], sample_rate: f32, agc_peak: &mut f32) -> [f32; BANDS] {
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
    let mut raw = [0f32; BANDS];
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
        raw[b] = sum / (hi - lo) as f32;
        lo = hi;
        if lo >= bins {
            break;
        }
    }

    // Tilt lifts the highs (music has far less high-frequency energy), then
    // normalize against the running peak.
    let tilted = |b: usize| raw[b] * (1.0 + b as f32 * 0.045);
    let frame_peak = (0..BANDS).map(tilted).fold(0f32, f32::max);
    if frame_peak < 5e-4 {
        *agc_peak = (*agc_peak * 0.9).max(5e-4);
        return out; // silence
    }
    if frame_peak > *agc_peak {
        *agc_peak = frame_peak; // instant attack
    } else {
        *agc_peak *= 0.99; // ~2.8 s half-life at ~25 emits/s
    }
    let peak = *agc_peak;
    for b in 0..BANDS {
        out[b] = (tilted(b) / peak).clamp(0.0, 1.0).powf(0.85);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f32) -> [f32; FFT_SIZE] {
        let mut s = [0f32; FFT_SIZE];
        for (i, v) in s.iter_mut().enumerate() {
            *v = (2.0 * PI * freq * i as f32 / 48000.0).sin();
        }
        s
    }

    #[test]
    fn silence_gives_zero_bands() {
        let mut agc = 5e-4f32;
        let out = compute_bands(&[0f32; FFT_SIZE], 48000.0, &mut agc);
        assert!(out.iter().all(|v| *v < 0.01));
    }

    #[test]
    fn tone_concentrates_in_low_bands() {
        let mut agc = 5e-4f32;
        let out = compute_bands(&tone(100.0), 48000.0, &mut agc);
        let peak = out.iter().cloned().fold(0f32, f32::max);
        assert!(peak > 0.5, "peak band should react, peak={peak}");
        let low: f32 = out[..12].iter().sum();
        let high: f32 = out[12..].iter().sum();
        assert!(low > high, "energy should sit in the low half: low={low} high={high}");
    }

    #[test]
    fn bands_are_normalized() {
        let mut agc = 5e-4f32;
        let out = compute_bands(&tone(440.0), 48000.0, &mut agc);
        assert!(out.iter().all(|v| (0.0..=1.0).contains(v)));
    }

    /// The bug the user reported: every bar pinned at max. A single tone must
    /// light only its neighborhood, never the whole spectrum.
    #[test]
    fn never_saturates_everything() {
        let mut agc = 5e-4f32;
        let out = compute_bands(&tone(220.0), 48000.0, &mut agc);
        let maxed = out.iter().filter(|v| **v > 0.9).count();
        assert!(maxed < 8, "too many saturated bands: {maxed}");
    }

    /// AGC dynamics: a quiet passage right after a loud one must not stay
    /// pinned (no saturation) and must visibly recover within a few seconds.
    #[test]
    fn agc_recovers_after_quiet() {
        let mut agc = 5e-4f32;
        let _ = compute_bands(&tone(220.0), 48000.0, &mut agc); // loud → peak set
        let mut quiet = tone(220.0);
        for v in quiet.iter_mut() {
            *v *= 0.05;
        }
        let first = compute_bands(&quiet, 48000.0, &mut agc);
        let first_peak = first.iter().cloned().fold(0f32, f32::max);
        assert!(first_peak < 0.2, "quiet must not instantly max: {first_peak}");
        let mut out = first;
        for _ in 0..200 {
            out = compute_bands(&quiet, 48000.0, &mut agc);
        }
        let recovered = out.iter().cloned().fold(0f32, f32::max);
        assert!(
            recovered > 0.3,
            "AGC should recover within ~8 s: {recovered}"
        );
        assert!(recovered <= 1.0);
    }
}
