pub mod auth;
pub mod buffered;
pub mod clock;
#[cfg(windows)]
pub mod credentials;
#[cfg(not(windows))]
#[path = "credentials_unix.rs"]
pub mod credentials;
pub mod continuity;
pub mod crypto;
pub mod discovery;
pub mod equalizer;
pub mod events;
pub mod metadata;
pub mod preemption;
pub mod profile;
pub mod qos;
pub mod receiver_capabilities;
pub mod rtp;
pub mod rtsp;
pub mod session;
pub mod servo;
pub mod source;

#[cfg(windows)]
pub mod wasapi;

#[cfg(windows)]
pub mod live;
#[cfg(not(windows))]
#[path = "live_unix.rs"]
pub mod live;

#[cfg(all(unix, feature = "cli"))]
pub mod cli;

pub mod standby;
pub mod telemetry;
pub mod transport;
pub mod volume;

#[cfg(windows)]
pub mod applications;
#[cfg(not(windows))]
#[path = "applications_unix.rs"]
pub mod applications;

#[cfg(test)]
mod test_net;
