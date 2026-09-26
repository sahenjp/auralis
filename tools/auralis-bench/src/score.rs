//! Score one immutable output with raw and deterministic-delay-compensated metrics.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use auralis_core::SAMPLE_RATE_HZ;
use serde::{Deserialize, Serialize};

use crate::DynError;
use crate::metrics::{AlignedQualityMetrics, quality_aligned};
use crate::wav::{self, Asset};

#[derive(Debug, Serialize)]
struct ScoreReport {
    schema_id: &'static str,
    system_id: String,
    sample_rate_hz: u32,
    compared_sample_count_raw: usize,
    compared_sample_count_compensated: usize,
    reference: ExternalAsset,
    noise_component: ExternalAsset,
    output: ExternalAsset,
    quality: AlignedQualityMetrics,
}

#[derive(Debug, Serialize)]
struct ExternalAsset {
    path: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct CorpusManifest {
    schema_id: String,
    cases: Vec<CorpusCase>,
}

#[derive(Debug, Deserialize)]
struct CorpusCase {
    case_id: String,
    condition: String,
    requested_snr_db: Option<f64>,
    reference: ManifestAsset,
    noise_component: Option<ManifestAsset>,
}

#[derive(Debug, Deserialize)]
struct OutputManifest {
    schema_id: String,
    candidate_id: String,
    cases: Vec<OutputCase>,
}

#[derive(Debug, Deserialize)]
struct OutputCase {
    case_id: String,
    output: ManifestAsset,
}

#[derive(Debug, Deserialize)]
struct ManifestAsset {
    path: String,
    sha256: String,
}

#[derive(Debug, Serialize)]
struct BatchScoreReport {
    schema_id: &'static str,
    system_id: String,
    candidate_id: String,
    corpus_manifest: ExternalAsset,
    output_manifest: ExternalAsset,
    alignment: AlignmentPolicy,
    case_count: usize,
    cases: Vec<BatchCaseScore>,
}

#[derive(Debug, Serialize)]
struct AlignmentPolicy {
    offset_samples: usize,
    offset_ms: f64,
    offset_source: String,
    method: &'static str,
}

#[derive(Debug, Serialize)]
struct BatchCaseScore {
    case_id: String,
    condition: String,
    requested_snr_db: Option<f64>,
    reference_sha256: String,
    noise_component_sha256: Option<String>,
    output_sha256: String,
    quality: AlignedQualityMetrics,
}

impl ExternalAsset {
    fn from_path(path: &Path) -> Result<Self, DynError> {
        let root = path.parent().unwrap_or_else(|| Path::new("."));
        let file_name = path
            .file_name()
            .ok_or_else(|| format!("path has no file name: {}", path.display()))?;
        let Asset { sha256, .. } = wav::asset(root, &file_name.to_string_lossy())?;
        Ok(Self {
            path: path.display().to_string(),
            sha256,
        })
    }
}

pub(crate) fn run(
    reference_path: &Path,
    noise_path: &Path,
    output_path: &Path,
    report_path: &Path,
    system_id: &str,
    offset_samples: usize,
    offset_source: &str,
) -> Result<(), DynError> {
    if system_id.is_empty() {
        return Err("system-id must not be empty".into());
    }
    if report_path.exists() {
        return Err(format!("refusing to overwrite {}", report_path.display()).into());
    }
    let reference = wav::read(reference_path)?;
    let noise = wav::read(noise_path)?;
    let output = wav::read(output_path)?;
    if reference.len() != noise.len() || reference.len() != output.len() {
        return Err(format!(
            "sample-count mismatch: reference={}, noise={}, output={}",
            reference.len(),
            noise.len(),
            output.len()
        )
        .into());
    }
    if offset_samples >= output.len() {
        return Err("offset-samples must be smaller than the signal".into());
    }
    let quality = quality_aligned(&reference, &noise, &output, offset_samples, offset_source);
    let report = ScoreReport {
        schema_id: "auralis.objective-score.v1",
        system_id: system_id.to_owned(),
        sample_rate_hz: SAMPLE_RATE_HZ,
        compared_sample_count_raw: reference.len(),
        compared_sample_count_compensated: reference.len() - offset_samples,
        reference: ExternalAsset::from_path(reference_path)?,
        noise_component: ExternalAsset::from_path(noise_path)?,
        output: ExternalAsset::from_path(output_path)?,
        quality,
    };
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent)?;
    }
    wav::write_json(report_path, &report)
}

pub(crate) fn run_batch(
    corpus_manifest_path: &Path,
    output_manifest_path: &Path,
    report_path: &Path,
    system_id: &str,
    offset_samples: usize,
    offset_source: &str,
) -> Result<(), DynError> {
    if system_id.is_empty() {
        return Err("system-id must not be empty".into());
    }
    if report_path.exists() {
        return Err(format!("refusing to overwrite {}", report_path.display()).into());
    }
    let corpus: CorpusManifest = serde_json::from_slice(&fs::read(corpus_manifest_path)?)?;
    let outputs: OutputManifest = serde_json::from_slice(&fs::read(output_manifest_path)?)?;
    if corpus.schema_id != "auralis.offline-bakeoff-corpus.v1" {
        return Err("unsupported corpus manifest schema".into());
    }
    if outputs.schema_id != "auralis.offline-bakeoff-output.v1" {
        return Err("unsupported output manifest schema".into());
    }
    let output_by_case: HashMap<&str, &OutputCase> = outputs
        .cases
        .iter()
        .map(|case| (case.case_id.as_str(), case))
        .collect();
    if output_by_case.len() != outputs.cases.len() || outputs.cases.len() != corpus.cases.len() {
        return Err("corpus/output case count or ID uniqueness mismatch".into());
    }

    let corpus_root = corpus_manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let output_root = output_manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let mut cases = Vec::with_capacity(corpus.cases.len());
    for case in corpus.cases {
        let output_case = output_by_case
            .get(case.case_id.as_str())
            .ok_or_else(|| format!("output missing corpus case: {}", case.case_id))?;
        verify_manifest_hash(corpus_root, &case.reference)?;
        verify_manifest_hash(output_root, &output_case.output)?;
        let reference = wav::read(&corpus_root.join(&case.reference.path))?;
        let output = wav::read(&output_root.join(&output_case.output.path))?;
        if reference.len() != output.len() || offset_samples >= output.len() {
            return Err(format!("sample-count or offset mismatch: {}", case.case_id).into());
        }
        let (noise, noise_sha256) = match case.noise_component {
            Some(asset) => {
                verify_manifest_hash(corpus_root, &asset)?;
                let samples = wav::read(&corpus_root.join(&asset.path))?;
                (samples, Some(asset.sha256))
            }
            None => (vec![0.0; reference.len()], None),
        };
        if noise.len() != reference.len() {
            return Err(format!("noise sample-count mismatch: {}", case.case_id).into());
        }
        cases.push(BatchCaseScore {
            case_id: case.case_id,
            condition: case.condition,
            requested_snr_db: case.requested_snr_db,
            reference_sha256: case.reference.sha256,
            noise_component_sha256: noise_sha256,
            output_sha256: output_case.output.sha256.clone(),
            quality: quality_aligned(&reference, &noise, &output, offset_samples, offset_source),
        });
    }
    let report = BatchScoreReport {
        schema_id: "auralis.objective-bakeoff-score.v1",
        system_id: system_id.to_owned(),
        candidate_id: outputs.candidate_id,
        corpus_manifest: ExternalAsset::from_path(corpus_manifest_path)?,
        output_manifest: ExternalAsset::from_path(output_manifest_path)?,
        alignment: AlignmentPolicy {
            offset_samples,
            offset_ms: offset_samples as f64 / SAMPLE_RATE_HZ as f64 * 1_000.0,
            offset_source: offset_source.to_owned(),
            method: "trim output prefix by fixed offset; truncate reference/noise tail; no signal-derived search",
        },
        case_count: cases.len(),
        cases,
    };
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent)?;
    }
    wav::write_json(report_path, &report)
}

fn verify_manifest_hash(root: &Path, expected: &ManifestAsset) -> Result<(), DynError> {
    let path = Path::new(&expected.path);
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("asset has no file name: {}", expected.path))?;
    let actual = wav::asset(&root.join(parent), &file_name.to_string_lossy())?;
    if actual.sha256 != expected.sha256 {
        return Err(format!("asset hash mismatch: {}", root.join(path).display()).into());
    }
    Ok(())
}
