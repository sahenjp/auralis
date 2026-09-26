//! End-to-end deterministic smoke benchmark orchestration.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::Path;

use auralis_core::{Passthrough, SAMPLE_RATE_HZ};
use serde::Serialize;

use crate::DynError;
use crate::fixtures::{generate_clean, generate_noise, mix_at_snr, snr_case_id};
use crate::metrics::{
    QualityMetrics, RuntimeMetrics, process_offline, quality, snr_db_from_components,
};
use crate::wav::{self, Asset};

const SNR_LEVELS_DB: [f64; 5] = [10.0, 5.0, 0.0, -5.0, -10.0];

#[derive(Debug, Serialize)]
struct MixtureCase {
    case_id: String,
    requested_snr_db: f64,
    measured_snr_db: f64,
    clean_gain: f64,
    noise_gain: f64,
    peak_normalization_gain: f64,
    reference: Asset,
    noise_component: Asset,
    mixture: Asset,
}

#[derive(Debug, Serialize)]
struct SmokeManifest {
    schema_version: u32,
    corpus_id: &'static str,
    purpose: &'static str,
    sample_rate_hz: u32,
    channels: u16,
    duration_samples: usize,
    clean_source: Asset,
    noise_source: Asset,
    cases: Vec<MixtureCase>,
}

#[derive(Debug, Serialize)]
struct SystemCaseResult {
    case_id: String,
    system_id: &'static str,
    output: Asset,
    quality: QualityMetrics,
    runtime: Option<RuntimeMetrics>,
}

#[derive(Debug, Serialize)]
struct SmokeResults {
    schema_version: u32,
    corpus_id: &'static str,
    warning: &'static str,
    systems: [&'static str; 2],
    results: Vec<SystemCaseResult>,
}

pub(crate) fn run(out_dir: &Path, seconds: u32) -> Result<(), DynError> {
    ensure_empty_directory(out_dir)?;
    for relative in [
        "corpus/clean",
        "corpus/noise",
        "corpus/mixtures",
        "outputs/raw",
        "outputs/experimental",
        "results",
    ] {
        fs::create_dir_all(out_dir.join(relative))?;
    }

    let sample_count = usize::try_from(SAMPLE_RATE_HZ)? * usize::try_from(seconds)?;
    let clean = generate_clean(sample_count);
    let noise = generate_noise(sample_count);
    wav::write(
        &out_dir.join("corpus/clean/synthetic-voiced-v1.wav"),
        &clean,
    )?;
    wav::write(
        &out_dir.join("corpus/noise/synthetic-mixed-noise-v1.wav"),
        &noise,
    )?;

    let mut manifest_cases = Vec::with_capacity(SNR_LEVELS_DB.len());
    let mut results = Vec::with_capacity(SNR_LEVELS_DB.len() * 2);
    for snr_db in SNR_LEVELS_DB {
        let case_id = snr_case_id(snr_db);
        let mixed = mix_at_snr(&clean, &noise, snr_db)?;
        let reference_relative = format!("corpus/mixtures/{case_id}-reference.wav");
        let noise_relative = format!("corpus/mixtures/{case_id}-noise.wav");
        let mixture_relative = format!("corpus/mixtures/{case_id}-mixture.wav");
        wav::write(&out_dir.join(&reference_relative), &mixed.reference)?;
        wav::write(&out_dir.join(&noise_relative), &mixed.noise)?;
        wav::write(&out_dir.join(&mixture_relative), &mixed.mixture)?;

        // Measure the exact bytes exposed to every future system under test.
        let disk_mixture = wav::read(&out_dir.join(&mixture_relative))?;
        let raw_relative = format!("outputs/raw/{case_id}.wav");
        wav::write(&out_dir.join(&raw_relative), &disk_mixture)?;
        results.push(make_result(
            out_dir,
            &case_id,
            "raw",
            &raw_relative,
            &mixed.reference,
            &mixed.noise,
            &disk_mixture,
            None,
        )?);

        let (processed, runtime) = process_offline(&disk_mixture, Passthrough);
        let experimental_relative = format!("outputs/experimental/{case_id}.wav");
        wav::write(&out_dir.join(&experimental_relative), &processed)?;
        results.push(make_result(
            out_dir,
            &case_id,
            "auralis-passthrough-dev",
            &experimental_relative,
            &mixed.reference,
            &mixed.noise,
            &processed,
            Some(runtime),
        )?);

        manifest_cases.push(MixtureCase {
            case_id,
            requested_snr_db: snr_db,
            measured_snr_db: snr_db_from_components(&mixed.reference, &mixed.noise),
            clean_gain: mixed.clean_gain,
            noise_gain: mixed.noise_gain,
            peak_normalization_gain: mixed.peak_gain,
            reference: wav::asset(out_dir, &reference_relative)?,
            noise_component: wav::asset(out_dir, &noise_relative)?,
            mixture: wav::asset(out_dir, &mixture_relative)?,
        });
    }

    let manifest = SmokeManifest {
        schema_version: 1,
        corpus_id: "auralis-smoke-v1",
        purpose: "transport and benchmark plumbing only; synthetic fixtures are not speech-quality evidence",
        sample_rate_hz: SAMPLE_RATE_HZ,
        channels: 1,
        duration_samples: sample_count,
        clean_source: wav::asset(out_dir, "corpus/clean/synthetic-voiced-v1.wav")?,
        noise_source: wav::asset(out_dir, "corpus/noise/synthetic-mixed-noise-v1.wav")?,
        cases: manifest_cases,
    };
    let smoke_results = SmokeResults {
        schema_version: 1,
        corpus_id: "auralis-smoke-v1",
        warning: "Synthetic smoke data cannot support audio-quality or competitor claims.",
        systems: ["raw", "auralis-passthrough-dev"],
        results,
    };
    wav::write_json(&out_dir.join("manifest.json"), &manifest)?;
    wav::write_json(&out_dir.join("results/results.json"), &smoke_results)?;
    write_results_csv(&out_dir.join("results/results.csv"), &smoke_results.results)?;
    Ok(())
}

fn ensure_empty_directory(path: &Path) -> io::Result<()> {
    if path.exists() {
        let mut entries = fs::read_dir(path)?;
        if entries.next().transpose()?.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "refusing to overwrite non-empty directory: {}",
                    path.display()
                ),
            ));
        }
    }
    fs::create_dir_all(path)
}

#[allow(clippy::too_many_arguments)]
fn make_result(
    root: &Path,
    case_id: &str,
    system_id: &'static str,
    output_relative: &str,
    reference: &[f32],
    noise: &[f32],
    output: &[f32],
    runtime: Option<RuntimeMetrics>,
) -> Result<SystemCaseResult, DynError> {
    Ok(SystemCaseResult {
        case_id: case_id.to_owned(),
        system_id,
        output: wav::asset(root, output_relative)?,
        quality: quality(reference, noise, output),
        runtime,
    })
}

fn write_results_csv(path: &Path, results: &[SystemCaseResult]) -> Result<(), DynError> {
    let mut csv = String::from(
        "case_id,system_id,si_sdr_db,sdr_db,input_snr_db,noise_attenuation_db,speech_projection_gain_db,peak_absolute,clipped_samples,realtime_factor,output_sha256\n",
    );
    for result in results {
        let realtime_factor = result
            .runtime
            .as_ref()
            .map_or_else(String::new, |runtime| runtime.realtime_factor.to_string());
        writeln!(
            csv,
            "{},{},{},{},{},{},{},{},{},{},{}",
            result.case_id,
            result.system_id,
            result.quality.si_sdr_db,
            result.quality.sdr_db,
            result.quality.measured_input_snr_db,
            result.quality.noise_attenuation_db,
            result.quality.speech_projection_gain_db,
            result.quality.peak_absolute,
            result.quality.clipped_samples,
            realtime_factor,
            result.output.sha256,
        )?;
    }
    fs::write(path, csv)?;
    Ok(())
}
