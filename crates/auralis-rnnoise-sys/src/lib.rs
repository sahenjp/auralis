//! Narrow ownership-safe wrapper around a locally supplied RNNoise library.

use std::error::Error;
use std::ffi::{c_float, c_void};
use std::fmt;
use std::path::{Path, PathBuf};
use std::ptr::{self, NonNull};
use std::sync::Arc;

use libloading::Library;

pub const RNNOISE_FRAME_SAMPLES: usize = 480;

type CreateFn = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
type DestroyFn = unsafe extern "C" fn(*mut c_void);
type FrameSizeFn = unsafe extern "C" fn() -> i32;
type ProcessFn = unsafe extern "C" fn(*mut c_void, *mut c_float, *const c_float) -> c_float;

struct Api {
    _library: Library,
    create: CreateFn,
    destroy: DestroyFn,
    process: ProcessFn,
}

// The loaded module remains alive in `Api`; its function table is immutable.
// RNNoise state itself is never shared and is moved exclusively with `State`.
unsafe impl Send for Api {}
unsafe impl Sync for Api {}

#[derive(Debug)]
pub enum LoadError {
    Library { path: PathBuf, message: String },
    Symbol { name: &'static str, message: String },
    UnexpectedFrameSize(i32),
    StateCreationFailed,
}

impl fmt::Display for LoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Library { path, message } => {
                write!(
                    formatter,
                    "failed to load RNNoise library {}: {message}",
                    path.display()
                )
            }
            Self::Symbol { name, message } => {
                write!(formatter, "failed to load RNNoise symbol {name}: {message}")
            }
            Self::UnexpectedFrameSize(size) => {
                write!(
                    formatter,
                    "RNNoise frame size is {size}, expected {RNNOISE_FRAME_SAMPLES}"
                )
            }
            Self::StateCreationFailed => formatter.write_str("RNNoise state creation failed"),
        }
    }
}

impl Error for LoadError {}

pub struct State {
    api: Arc<Api>,
    state: NonNull<c_void>,
}

// `State` has exclusive ownership of the native state and processing requires
// `&mut self`; moving that ownership to the processing worker is safe.
unsafe impl Send for State {}

impl State {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, LoadError> {
        let path = path.as_ref();
        // SAFETY: the library stays owned by `Api` until after every copied
        // function pointer and native state have been destroyed.
        let library = unsafe { Library::new(path) }.map_err(|error| LoadError::Library {
            path: path.to_owned(),
            message: error.to_string(),
        })?;
        // SAFETY: each symbol name and function signature matches rnnoise.h at
        // the pinned upstream revision. Function pointers are copied while the
        // owning library remains stored in `Api`.
        let create =
            unsafe { load_symbol::<CreateFn>(&library, b"rnnoise_create\0", "rnnoise_create")? };
        let destroy =
            unsafe { load_symbol::<DestroyFn>(&library, b"rnnoise_destroy\0", "rnnoise_destroy")? };
        let frame_size = unsafe {
            load_symbol::<FrameSizeFn>(
                &library,
                b"rnnoise_get_frame_size\0",
                "rnnoise_get_frame_size",
            )?
        };
        let process = unsafe {
            load_symbol::<ProcessFn>(
                &library,
                b"rnnoise_process_frame\0",
                "rnnoise_process_frame",
            )?
        };
        // SAFETY: `frame_size` takes no arguments and the library is alive.
        let actual_frame_size = unsafe { frame_size() };
        if actual_frame_size != RNNOISE_FRAME_SAMPLES as i32 {
            return Err(LoadError::UnexpectedFrameSize(actual_frame_size));
        }
        let api = Arc::new(Api {
            _library: library,
            create,
            destroy,
            process,
        });
        let state = create_state(&api)?;
        Ok(Self { api, state })
    }

    pub fn reset(&mut self) -> Result<(), LoadError> {
        // Create first so a failure leaves the existing state usable.
        let replacement = create_state(&self.api)?;
        // SAFETY: `self.state` was returned by this API's `create`, is uniquely
        // owned here, and has not previously been destroyed.
        unsafe { (self.api.destroy)(self.state.as_ptr()) };
        self.state = replacement;
        Ok(())
    }

    pub fn process(
        &mut self,
        input: &[f32; RNNOISE_FRAME_SAMPLES],
        output: &mut [f32; RNNOISE_FRAME_SAMPLES],
    ) {
        // SAFETY: state ownership is exclusive, buffers are exactly the frame
        // size reported by the library, and both remain valid for the call.
        unsafe {
            (self.api.process)(self.state.as_ptr(), output.as_mut_ptr(), input.as_ptr());
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        // SAFETY: this is the final use of the uniquely owned state and `api`
        // (therefore its library) outlives this call.
        unsafe { (self.api.destroy)(self.state.as_ptr()) };
    }
}

fn create_state(api: &Api) -> Result<NonNull<c_void>, LoadError> {
    // SAFETY: a null model selects the default model embedded by the verified
    // local RNNoise build, per rnnoise.h.
    NonNull::new(unsafe { (api.create)(ptr::null_mut()) }).ok_or(LoadError::StateCreationFailed)
}

unsafe fn load_symbol<T: Copy>(
    library: &Library,
    bytes: &[u8],
    name: &'static str,
) -> Result<T, LoadError> {
    // SAFETY: the caller supplies a NUL-terminated symbol and the exact pinned
    // C signature; the returned pointer is copied while `library` stays alive.
    unsafe { library.get::<T>(bytes) }
        .map(|symbol| *symbol)
        .map_err(|error| LoadError::Symbol {
            name,
            message: error.to_string(),
        })
}
