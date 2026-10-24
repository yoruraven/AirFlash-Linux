//! Monotonic media clock and a bounded unicast PTPv2 probe master.
//! Implements the AirPlay PTP wire profile; no system clock is changed.
use crate::rtsp::Cancellation;
use anyhow::{Context, Result};
use std::{
    net::{IpAddr, UdpSocket},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[derive(Clone)]
pub struct Clock {
    origin: Instant,
    epoch_ns: u64,
}
impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}
impl Clock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
            epoch_ns: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64,
        }
    }
    pub fn now_ns(&self) -> u64 {
        self.epoch_ns + self.origin.elapsed().as_nanos() as u64
    }
}

/// Wall-clock nanoseconds for capture timestamps. Monotonicity is not required:
/// these values only feed queue-age diagnostics and onset markers.
pub fn unix_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}
fn header(kind: u8, len: usize, clock: u64, seq: u16, flags: u16, interval: i8) -> Vec<u8> {
    let mut p = vec![0; len];
    p[0] = 0x10 | kind;
    p[1] = 2;
    p[2..4].copy_from_slice(&(len as u16).to_be_bytes());
    p[6..8].copy_from_slice(&flags.to_be_bytes());
    p[20..28].copy_from_slice(&clock.to_be_bytes());
    p[28..30].copy_from_slice(&0x8005u16.to_be_bytes());
    p[30..32].copy_from_slice(&seq.to_be_bytes());
    p[33] = interval as u8;
    p
}
fn timestamp(out: &mut [u8], ns: u64) {
    out[..6].copy_from_slice(&(ns / 1_000_000_000).to_be_bytes()[2..]);
    out[6..10].copy_from_slice(&((ns % 1_000_000_000) as u32).to_be_bytes());
}
fn tlv(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0, 3];
    out.extend((data.len() as u16).to_be_bytes());
    out.extend(data);
    out
}
pub fn announce(clock: u64, seq: u16, ns: u64) -> Vec<u8> {
    // IEEE 1588-2008 announce: originTimestamp (10), currentUtcOffset (2),
    // one reserved byte (1), then grandmaster priority/quality/identity.
    // The reserved byte at offset 46 was missing, shifting every later field
    // and preventing HomePods from selecting this clock (they'd light up but
    // stay silent because the clock never locked).
    let mut p = header(11, 76, clock, seq, 0x0408, 0);
    timestamp(&mut p[34..44], ns);
    p[44..46].copy_from_slice(&0u16.to_be_bytes()); // currentUtcOffset
    p[46] = 0; // reserved
    p[47] = 128; // grandmasterPriority1
    p[48] = 248; // clockClass (local oscillator, not GPS)
    p[49] = 0xfe; // clockAccuracy
    p[50..52].copy_from_slice(&0xffffu16.to_be_bytes()); // variance
    p[52] = 128; // grandmasterPriority2
    p[53..61].copy_from_slice(&clock.to_be_bytes()); // grandmasterIdentity
    p[61..63].copy_from_slice(&0u16.to_be_bytes()); // stepsRemoved
    p[63] = 0xa0; // timeSource
    p[64..68].copy_from_slice(&[0, 8, 0, 8]); // PATH_TRACE TLV header
    p[68..].copy_from_slice(&clock.to_be_bytes()); // clock ID in TLV
    p
}
pub fn sync_pair(clock: u64, seq: u16, ns: u64) -> (Vec<u8>, Vec<u8>) {
    let mut sync = header(0, 44, clock, seq, 0x0608, -3);
    timestamp(&mut sync[34..], ns);
    let mut follow = header(8, 96, clock, seq, 0x0408, -3);
    timestamp(&mut follow[34..44], ns);
    let mut rate = vec![0; 28];
    rate[..6].copy_from_slice(&[0, 0x80, 0xc2, 0, 0, 1]);
    let mut cid = vec![0; 16];
    cid[..6].copy_from_slice(&[0, 0x0d, 0x93, 0, 0, 4]);
    cid[6..14].copy_from_slice(&clock.to_be_bytes());
    follow[44..76].copy_from_slice(&tlv(&rate));
    follow[76..].copy_from_slice(&tlv(&cid));
    (sync, follow)
}
pub struct PtpMaster {
    pub clock_id: u64,
    pub received: Arc<AtomicU64>,
    cancel: Cancellation,
    worker: Option<JoinHandle<()>>,
}
impl PtpMaster {
    pub fn start(peers: Vec<IpAddr>, clock: Clock) -> Result<Self> {
        // PTP event/general ports are below the unprivileged port range. Grant
        // them with one of: sudo setcap cap_net_bind_service=+ep on the engine
        // binary, sysctl net.ipv4.ip_unprivileged_port_start<=319 (or ambient
        // capabilities / systemd AmbientCapabilities=CAP_NET_BIND_SERVICE).
        // timing="ntp" avoids the privileged ports but AirPlay 2 receivers
        // such as HomePods require PTP; timing="auto" picks PTP when possible.
        let event = UdpSocket::bind("0.0.0.0:319")
            .context("PTP event port 319 unavailable; grant it with `sudo setcap cap_net_bind_service=+ep` on the engine binary (or sysctl net.ipv4.ip_unprivileged_port_start<=319), or use timing=ntp/auto")?;
        let general = UdpSocket::bind("0.0.0.0:320")
            .context("PTP general port 320 unavailable; grant it with `sudo setcap cap_net_bind_service=+ep` on the engine binary (or sysctl net.ipv4.ip_unprivileged_port_start<=319), or use timing=ntp/auto")?;
        event.set_nonblocking(true)?;
        general.set_nonblocking(true)?;
        let clock_id = rand::random::<u64>() & 0x7fff_ffff_ffff_ffff;
        let received = Arc::new(AtomicU64::new(0));
        let rx = received.clone();
        let cancel = Cancellation::default();
        let stop = cancel.clone();
        let worker = thread::Builder::new()
            .name("airplay-ptp".into())
            .spawn(move || {
                let mut seq = 0u16;
                let mut last_announce = Instant::now() - Duration::from_secs(2);
                let mut next_sync = Instant::now();
                let mut buf = [0u8; 2048];
                while !stop.is_cancelled() {
                    let now = Instant::now();
                    if now >= next_sync {
                        for peer in &peers {
                            let (sync, follow) = sync_pair(clock_id, seq, clock.now_ns());
                            let _ = event.send_to(&sync, (*peer, 319));
                            let _ = general.send_to(&follow, (*peer, 320));
                            if now.duration_since(last_announce) >= Duration::from_secs(1) {
                                let _ = general.send_to(
                                    &announce(clock_id, seq, clock.now_ns()),
                                    (*peer, 320),
                                );
                            }
                        }
                        if now.duration_since(last_announce) >= Duration::from_secs(1) {
                            last_announce = now;
                        }
                        seq = seq.wrapping_add(1);
                        next_sync = now + Duration::from_millis(125);
                    }
                    for socket in [&event, &general] {
                        while let Ok((n, addr)) = socket.recv_from(&mut buf) {
                            if !peers.contains(&addr.ip()) || n < 34 || buf[1] & 0xf != 2 {
                                continue;
                            }
                            let length = u16::from_be_bytes([buf[2], buf[3]]) as usize;
                            if length > n || length < 34 {
                                continue;
                            }
                            rx.fetch_add(1, Ordering::Relaxed);
                            let kind = buf[0] & 0xf;
                            let request_seq = u16::from_be_bytes([buf[30], buf[31]]);
                            if (kind == 1 || kind == 2) && n >= 44 {
                                let response_kind = if kind == 1 { 9 } else { 3 };
                                let mut response = header(
                                    response_kind,
                                    54,
                                    clock_id,
                                    request_seq,
                                    if kind == 1 { 0x0408 } else { 0x0608 },
                                    -3,
                                );
                                timestamp(&mut response[34..44], clock.now_ns());
                                response[44..54].copy_from_slice(&buf[20..30]);
                                if kind == 1 {
                                    let _ = general.send_to(&response, (addr.ip(), 320));
                                } else {
                                    let _ = event.send_to(&response, (addr.ip(), 319));
                                    let mut follow =
                                        header(10, 54, clock_id, request_seq, 0x0408, -3);
                                    timestamp(&mut follow[34..44], clock.now_ns());
                                    follow[44..54].copy_from_slice(&buf[20..30]);
                                    let _ = general.send_to(&follow, (addr.ip(), 320));
                                }
                            }
                        }
                    }
                    thread::sleep(Duration::from_millis(2));
                }
            })?;
        Ok(Self {
            clock_id,
            received,
            cancel,
            worker: Some(worker),
        })
    }
}
impl Drop for PtpMaster {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wire_clock_and_lengths() {
        let id = 0x1234567812345678;
        let (s, f) = sync_pair(id, 65535, 1234567890);
        assert_eq!(&s[20..28], &id.to_be_bytes());
        assert_eq!(s.len(), 44);
        assert_eq!(f.len(), 96);
        assert_eq!(&f[34..44], &[0, 0, 0, 0, 0, 1, 13, 251, 56, 210]);
        let announced = announce(id, 0, 0);
        assert_eq!(announced.len(), 76);
        // IEEE 1588 announce: currentUtcOffset (2), reserved (1), priority1,
        // clockClass, clockAccuracy, variance, priority2, grandmasterIdentity,
        // stepsRemoved, timeSource, then the PATH_TRACE TLV.
        assert_eq!(&announced[44..47], &[0, 0, 0]); // UTC offset + reserved
        assert_eq!(announced[47], 128); // grandmasterPriority1
        assert_eq!(&announced[48..53], &[248, 0xfe, 0xff, 0xff, 128]); // class, accuracy, variance, priority2
        assert_eq!(&announced[53..61], &id.to_be_bytes()); // grandmasterIdentity
        assert_eq!(&announced[61..64], &[0, 0, 0xa0]); // stepsRemoved, timeSource
        assert_eq!(&announced[64..68], &[0, 8, 0, 8]); // TLV type and length
        assert_eq!(&announced[68..], &id.to_be_bytes()); // clock in TLV
    }
}
