//! Deterministic smoke-only signal generation and controlled mixing.

use std::f32::consts::TAU;

use auralis_core::SAMPLE_RATE_HZ;

use crate::DynError;
use crate::metrics::rms;

pub(crate) struct MixedSignals {
    pub reference: Vec<f32>,
    pub noise: Vec<f32>,
    pub mixture: Vec<f32>,
    pub clean_gain: f64,
    pub noise_gain: f64,
    pub peak_gain: f64,
}

pub(crate) fn generate_clean(sample_count: usize) -> Vec<f32> {
    let mut phase = 0.0_f32;
    (0..sample_count)
        .map(|index| {
            let time = index as f32 / SAMPLE_RATE_HZ as f32;
            let fundamental_hz = 145.0 + 18.0 * (TAU * 0.7 * time).sin();
            phase += TAU * fundamental_hz / SAMPLE_RATE_HZ as f32;
            let syllable = ((TAU * 2.1 * time).sin() * 0.5 + 0.5).powi(2);
            let envelope = 0.12 + 0.88 * syllable;
            envelope
                * (0.34 * phase.sin() + 0.13 * (2.0 * phase).sin() + 0.07 * (3.0 * phase).sin())
        })
        .collect()
}

pub(crate) fn generate_noise(sample_count: usize) -> Vec<f32> {
    let mut state = 0xA5A5_1234_u32;
    (0..sample_count)
        .map(|index| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let white = (f64::from(state) / f64::from(u32::MAX) * 2.0 - 1.0) as f32;
            let time = index as f32 / SAMPLE_RATE_HZ as f32;
            let fan = 0.35 * (TAU * 120.0 * time).sin() + 0.16 * (TAU * 240.0 * time).sin();
            let click_period = usize::try_from(SAMPLE_RATE_HZ).expect("sample rate fits usize") / 7;
            let click = if index % click_period < 24 {
                let decay = 1.0 - (index % click_period) as f32 / 24.0;
                1.8 * decay
            } else {
                0.0
            };
            0.55 * white + fan + click
        })
        .collect()
}

pub(crate) fn mix_at_snr(
    clean: &[f32],
    noise: &[f32],
    snr_db: f64,
) -> Result<MixedSignals, DynError> {
    if clean.len() != noise.len() || clean.is_empty() {
        return Err("clean and noise must be non-empty and equal length".into());
    }
    let clean_rms = rms(clean);
    let noise_rms = rms(noise);
    if clean_rms == 0.0 || noise_rms == 0.0 {
        return Err("clean and noise RMS must be non-zero".into());
    }
    let clean_gain = 1.0_f64;
    let noise_gain = clean_rms / (noise_rms * 10.0_f64.powf(snr_db / 20.0));
    let unscaled_peak = clean
        .iter()
        .zip(noise)
        .map(|(&speech, &background)| {
            (f64::from(speech) * clean_gain + f64::from(background) * noise_gain).abs()
        })
        .fold(0.0_f64, f64::max);
    let peak_gain = if unscaled_peak > 0.98 {
        0.98 / unscaled_peak
    } else {
        1.0
    };

    let reference: Vec<f32> = clean
        .iter()
        .map(|sample| (f64::from(*sample) * clean_gain * peak_gain) as f32)
        .collect();
    let scaled_noise: Vec<f32> = noise
        .iter()
        .map(|sample| (f64::from(*sample) * noise_gain * peak_gain) as f32)
        .collect();
    let mixture = reference
        .iter()
        .zip(&scaled_noise)
        .map(|(&speech, &background)| speech + background)
        .collect();

    Ok(MixedSignals {
        reference,
        noise: scaled_noise,
        mixture,
        clean_gain,
        noise_gain,
        peak_gain,
    })
}

pub(crate) fn snr_case_id(snr_db: f64) -> String {
    if snr_db < 0.0 {
        format!("snr-minus-{}db", snr_db.abs() as u32)
    } else {
        format!("snr-plus-{}db", snr_db as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::{generate_clean, generate_noise, mix_at_snr};
    use crate::metrics::snr_db_from_components;

    #[test]
    fn mixture_hits_requested_snr_without_clipping() {
        let clean = generate_clean(48_000);
        let noise = generate_noise(48_000);
        let mixed = mix_at_snr(&clean, &noise, -5.0).expect("mix should succeed");
        let measured = snr_db_from_components(&mixed.reference, &mixed.noise);
        assert!((measured - -5.0).abs() < 0.000_01);
        assert!(mixed.mixture.iter().all(|sample| sample.abs() <= 0.980_001));
    }

    #[test]
    fn fixture_generation_is_deterministic() {
        assert_eq!(generate_noise(2_000), generate_noise(2_000));
        assert_eq!(generate_clean(2_000), generate_clean(2_000));
    }
}
