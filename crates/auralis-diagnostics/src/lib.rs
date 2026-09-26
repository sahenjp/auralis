//! Non-realtime process diagnostics for measured engine runs.

#![forbid(unsafe_code)]

use std::io;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

use serde::Serialize;
use sysinfo::{MINIMUM_CPU_UPDATE_INTERVAL, ProcessesToUpdate, System, get_current_pid};

/// Sampled process CPU and resident-memory measurements.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ProcessResourceSnapshot {
    /// Number of non-realtime samples included in the result.
    pub samples: u64,
    /// Average process CPU percentage; 100% represents one fully used logical CPU.
    pub average_cpu_percent: f64,
    /// Maximum sampled process CPU percentage on the same scale.
    pub max_cpu_percent: f64,
    /// Maximum sampled resident memory in bytes.
    pub peak_resident_memory_bytes: u64,
    /// Resident memory from the final sample in bytes.
    pub final_resident_memory_bytes: u64,
}

/// Latest non-realtime process sample while a monitor is running.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct ProcessResourceLiveSnapshot {
    pub cpu_percent: f64,
    pub resident_memory_bytes: u64,
}

#[derive(Default)]
struct LiveResourceState {
    sample: Mutex<Option<ProcessResourceLiveSnapshot>>,
}

/// Owns a low-frequency diagnostics thread that never runs in an audio callback.
pub struct ProcessResourceMonitor {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<ProcessResourceSnapshot>>,
    live: Arc<LiveResourceState>,
}

impl ProcessResourceMonitor {
    /// Return the host OS version for non-realtime provenance reports.
    pub fn current_os_version() -> Option<String> {
        System::long_os_version()
    }

    /// Read the current process resident memory outside realtime callbacks.
    pub fn current_resident_memory_bytes() -> io::Result<u64> {
        let pid = get_current_pid().map_err(io::Error::other)?;
        let mut system = System::new();
        system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
        system
            .process(pid)
            .map(|process| process.memory())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "current process not found"))
    }

    /// Start sampling the current process.
    pub fn start() -> io::Result<Self> {
        let pid = get_current_pid().map_err(io::Error::other)?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let live = Arc::new(LiveResourceState::default());
        let thread_live = Arc::clone(&live);
        let join = thread::Builder::new()
            .name("auralis-diagnostics".to_owned())
            .spawn(move || {
                let mut system = System::new();
                system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
                let mut snapshot = ProcessResourceSnapshot::default();
                let mut cpu_total = 0.0_f64;

                while !thread_stop.load(Ordering::Relaxed) {
                    thread::sleep(MINIMUM_CPU_UPDATE_INTERVAL);
                    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
                    let Some(process) = system.process(pid) else {
                        continue;
                    };
                    let cpu = f64::from(process.cpu_usage());
                    let memory = process.memory();
                    if let Ok(mut sample) = thread_live.sample.lock() {
                        *sample = Some(ProcessResourceLiveSnapshot {
                            cpu_percent: cpu,
                            resident_memory_bytes: memory,
                        });
                    }
                    snapshot.samples = snapshot.samples.saturating_add(1);
                    cpu_total += cpu;
                    snapshot.max_cpu_percent = snapshot.max_cpu_percent.max(cpu);
                    snapshot.peak_resident_memory_bytes =
                        snapshot.peak_resident_memory_bytes.max(memory);
                    snapshot.final_resident_memory_bytes = memory;
                }

                if snapshot.samples > 0 {
                    snapshot.average_cpu_percent = cpu_total / snapshot.samples as f64;
                }
                snapshot
            })?;
        Ok(Self {
            stop,
            join: Some(join),
            live,
        })
    }

    /// Return the most recent sample without stopping the monitor.
    pub fn latest(&self) -> Option<ProcessResourceLiveSnapshot> {
        self.live.sample.lock().ok().and_then(|sample| *sample)
    }

    /// Stop sampling and return the collected measurements.
    pub fn finish(mut self) -> thread::Result<ProcessResourceSnapshot> {
        self.stop.store(true, Ordering::Relaxed);
        self.join
            .take()
            .map_or_else(|| Ok(ProcessResourceSnapshot::default()), JoinHandle::join)
    }
}

impl Drop for ProcessResourceMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
