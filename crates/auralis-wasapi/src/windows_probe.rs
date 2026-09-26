use std::ffi::c_void;
use std::ptr;

use cpal::Device;
use windows::Win32::Media::Audio::{IAudioClient3, WAVEFORMATEX};
use windows::Win32::System::Com::{CLSCTX_ALL, CoTaskMemFree};

use crate::{WasapiEngineFormat, WasapiEnginePeriod};

struct CoTaskMemWaveFormat(*mut WAVEFORMATEX);

impl CoTaskMemWaveFormat {
    fn as_ptr(&self) -> *const WAVEFORMATEX {
        self.0.cast_const()
    }

    fn value(&self) -> Result<&WAVEFORMATEX, String> {
        // SAFETY: IAudioClient3 returned a non-null CoTaskMem-allocated
        // WAVEFORMATEX pointer and this guard owns it until drop.
        unsafe { self.0.as_ref() }.ok_or_else(|| "IAudioClient3 returned a null format".to_owned())
    }
}

impl Drop for CoTaskMemWaveFormat {
    fn drop(&mut self) {
        // SAFETY: The pointer was allocated by the COM method documented to
        // require CoTaskMemFree, and this guard frees it exactly once.
        unsafe { CoTaskMemFree(Some(self.0.cast::<c_void>())) };
    }
}

pub(super) fn query_engine_period(device: &Device) -> Result<WasapiEnginePeriod, String> {
    let cpal::platform::DeviceInner::Wasapi(wasapi_device) = device.as_inner();
    let endpoint = wasapi_device
        .immdevice()
        .ok_or_else(|| "WASAPI endpoint is no longer available".to_owned())?;

    // SAFETY: CPAL resolves a live IMMDevice on the current COM-initialized
    // thread. Activate is used only to obtain an uninitialized IAudioClient3.
    let client: IAudioClient3 = unsafe { endpoint.Activate(CLSCTX_ALL, None) }
        .map_err(|error| format!("IMMDevice::Activate(IAudioClient3): {error}"))?;

    let mut current_format = ptr::null_mut();
    let mut current_frames = 0;
    // SAFETY: Both out-pointers are valid. The returned format is immediately
    // placed under an owning CoTaskMem guard.
    unsafe { client.GetCurrentSharedModeEnginePeriod(&mut current_format, &mut current_frames) }
        .map_err(|error| format!("GetCurrentSharedModeEnginePeriod: {error}"))?;
    let current_format = CoTaskMemWaveFormat(current_format);
    let format = current_format.value()?;

    let mut default_frames = 0;
    let mut fundamental_frames = 0;
    let mut minimum_frames = 0;
    let mut maximum_frames = 0;
    // SAFETY: The format pointer remains valid for this call and all output
    // pointers refer to initialized local u32 values.
    unsafe {
        client.GetSharedModeEnginePeriod(
            current_format.as_ptr(),
            &mut default_frames,
            &mut fundamental_frames,
            &mut minimum_frames,
            &mut maximum_frames,
        )
    }
    .map_err(|error| format!("GetSharedModeEnginePeriod: {error}"))?;

    Ok(WasapiEnginePeriod {
        format: WasapiEngineFormat {
            sample_rate_hz: format.nSamplesPerSec,
            channels: format.nChannels,
            bits_per_sample: format.wBitsPerSample,
            block_align_bytes: format.nBlockAlign,
            average_bytes_per_second: format.nAvgBytesPerSec,
            format_tag: format.wFormatTag,
        },
        default_frames,
        fundamental_frames,
        minimum_frames,
        maximum_frames,
        current_frames,
    })
}
