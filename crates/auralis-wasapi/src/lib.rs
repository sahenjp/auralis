//! Narrow Windows realtime-audio platform adapter.
//!
//! Audio transport remains owned by CPAL. This crate reads the shared-mode
//! engine period and registers the processing worker with Windows MMCSS.

#![deny(unsafe_code)]

use cpal::Device;
use serde::Serialize;

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
mod windows_probe;

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
mod windows_scheduling;

/// Shared-mode engine format returned by `IAudioClient3`.
#[derive(Clone, Debug, Serialize)]
pub struct WasapiEngineFormat {
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub block_align_bytes: u16,
    pub average_bytes_per_second: u32,
    pub format_tag: u16,
}

/// Period limits and instantaneous current period for one endpoint.
#[derive(Clone, Debug, Serialize)]
pub struct WasapiEnginePeriod {
    pub format: WasapiEngineFormat,
    pub default_frames: u32,
    pub fundamental_frames: u32,
    pub minimum_frames: u32,
    pub maximum_frames: u32,
    pub current_frames: u32,
}

/// Query `IAudioClient3` without initializing or changing the audio stream.
pub fn query_engine_period(device: &Device) -> Result<Option<WasapiEnginePeriod>, String> {
    #[cfg(target_os = "windows")]
    {
        windows_probe::query_engine_period(device).map(Some)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = device;
        Ok(None)
    }
}

/// Register the calling processing thread for the Windows `Pro Audio` MMCSS task.
#[cfg(target_os = "windows")]
pub fn register_current_thread_for_pro_audio() -> Result<(), String> {
    windows_scheduling::register_current_thread_for_pro_audio()
}
