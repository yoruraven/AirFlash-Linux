//! Non-Windows placeholder for per-application capture discovery.
//!
//! Capturing one application's audio is a Windows WASAPI process-loopback
//! feature. On Linux this module exists so the shared session code compiles
//! unchanged and the feature cleanly reports itself as unsupported; the
//! system-wide PipeWire sink monitor is the Linux capture path instead.
use anyhow::Result;

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Application {
    pub path: String,
    pub name: String,
    pub process_id: u32,
    pub started_at: u64,
}

/// Upper bound on application instances captured at once (mirrors Windows).
pub const MAX_CAPTURE_INSTANCES: usize = 4;

pub struct Process;

impl Process {
    pub fn verify(_application: &Application) -> Result<Self> {
        anyhow::bail!("application capture is not supported on this platform")
    }
}

pub fn supported() -> bool {
    false
}

pub fn matching(_path: &str) -> Result<Vec<Application>> {
    Ok(Vec::new())
}

pub fn bind_plan(
    _path: &str,
    _candidates: &[Application],
    _pinned: Option<(u32, u64)>,
    _cap: usize,
) -> Vec<Application> {
    Vec::new()
}

pub fn list() -> Result<Vec<Application>> {
    Ok(Vec::new())
}
