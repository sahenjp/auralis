//! Float-WAV and content-hash utilities for offline artifacts.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use auralis_core::SAMPLE_RATE_HZ;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::DynError;

#[derive(Debug, Serialize)]
pub(crate) struct Asset {
    pub path: String,
    pub sha256: String,
}

pub(crate) fn write(path: &Path, samples: &[f32]) -> Result<(), DynError> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE_HZ,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for &sample in samples {
        writer.write_sample(sample)?;
    }
    writer.finalize()?;
    Ok(())
}

pub(crate) fn read(path: &Path) -> Result<Vec<f32>, DynError> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    if spec.channels != 1
        || spec.sample_rate != SAMPLE_RATE_HZ
        || spec.bits_per_sample != 32
        || spec.sample_format != hound::SampleFormat::Float
    {
        return Err(format!("unsupported smoke WAV format: {}", path.display()).into());
    }
    reader
        .samples::<f32>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

pub(crate) fn asset(root: &Path, relative: &str) -> Result<Asset, DynError> {
    Ok(Asset {
        path: relative.to_owned(),
        sha256: sha256_file(&root.join(relative))?,
    })
}

pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), DynError> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(path, bytes)?;
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, DynError> {
    let bytes = fs::read(path)?;
    let digest = Sha256::digest(bytes);
    let mut hexadecimal = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut hexadecimal, "{byte:02x}")?;
    }
    Ok(hexadecimal)
}
