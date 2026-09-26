use std::cell::RefCell;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Threading::{
    AVRT_PRIORITY_NORMAL, AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsA,
    AvSetMmThreadPriority,
};
use windows::core::s;

/// RAII registration for the calling thread's MMCSS task.
struct MmcssRegistration {
    handle: HANDLE,
}

thread_local! {
    static MMCSS_REGISTRATION: RefCell<Option<MmcssRegistration>> = const { RefCell::new(None) };
}

impl Drop for MmcssRegistration {
    fn drop(&mut self) {
        // SAFETY: The handle was returned by AvSetMmThreadCharacteristicsA on
        // this worker thread, is owned by this guard, and is reverted once.
        let _ = unsafe { AvRevertMmThreadCharacteristics(self.handle) };
    }
}

pub(super) fn register_current_thread_for_pro_audio() -> Result<(), String> {
    MMCSS_REGISTRATION.with(|registration| {
        if registration.borrow().is_some() {
            return Ok(());
        }
        let new_registration = create_registration()?;
        *registration.borrow_mut() = Some(new_registration);
        Ok(())
    })
}

fn create_registration() -> Result<MmcssRegistration, String> {
    let mut task_index = 0;
    // SAFETY: `Pro Audio` is a static NUL-terminated task name and the output
    // task-index pointer is valid for this call.
    let handle = unsafe { AvSetMmThreadCharacteristicsA(s!("Pro Audio"), &mut task_index) }
        .map_err(|error| format!("AvSetMmThreadCharacteristicsA(Pro Audio): {error}"))?;
    // SAFETY: The handle belongs to this calling thread and remains valid.
    if let Err(error) = unsafe { AvSetMmThreadPriority(handle, AVRT_PRIORITY_NORMAL) } {
        // SAFETY: Priority setup failed after registration, so revert the
        // still-valid handle before returning the error.
        let _ = unsafe { AvRevertMmThreadCharacteristics(handle) };
        return Err(format!("AvSetMmThreadPriority(NORMAL): {error}"));
    }
    Ok(MmcssRegistration { handle })
}
