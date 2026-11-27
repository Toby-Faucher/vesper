//! The media socket's kernel receive buffer. Under heavy video the SFU thread falls behind in
//! bursts, and packets that arrive while the buffer is full are dropped, voice with them
//! (docs/known-issues.md). A bigger buffer holds the bursts.

use std::net::UdpSocket;

use socket2::SockRef;
use tracing::{info, warn};

/// What the kernel gave when the receive buffer was set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Granted {
    pub asked: usize,
    /// `SO_RCVBUF` read back. Linux doubles the value it stores (the extra is for its own
    /// bookkeeping), so a full grant reads back as twice `asked`.
    pub granted: usize,
}

impl Granted {
    /// The kernel stored less than asked. On Linux that means `net.core.rmem_max` capped it.
    pub fn capped(&self) -> bool {
        self.granted < self.asked * GRANT_FACTOR
    }
}

/// How much bigger Linux reports `SO_RCVBUF` than the value set.
const GRANT_FACTOR: usize = if cfg!(target_os = "linux") { 2 } else { 1 };

/// Ask for a receive buffer of `bytes` and log what the kernel granted, with a warning and the
/// sysctl to raise when it gave less. `bytes` = 0 keeps the system default. A failure is logged,
/// not returned: a small buffer is a quality problem, not a reason to refuse to start.
pub(crate) fn set_recv_buffer(socket: &UdpSocket, bytes: usize) -> Option<Granted> {
    let sock = SockRef::from(socket);
    if bytes == 0 {
        match sock.recv_buffer_size() {
            Ok(n) => info!("udp receive buffer: system default, {n} bytes"),
            Err(e) => warn!("udp receive buffer: cannot read it: {e}"),
        }
        return None;
    }
    if let Err(e) = sock.set_recv_buffer_size(bytes) {
        warn!("udp receive buffer: cannot set {bytes} bytes: {e}");
        return None;
    }
    let granted = match sock.recv_buffer_size() {
        Ok(n) => Granted {
            asked: bytes,
            granted: n,
        },
        Err(e) => {
            warn!("udp receive buffer: set {bytes} bytes but cannot read it back: {e}");
            return None;
        }
    };
    if granted.capped() {
        let max = rmem_max().map_or_else(|| "unknown".into(), |m| m.to_string());
        warn!(
            "udp receive buffer: asked for {bytes} bytes, the kernel granted {} (net.core.rmem_max \
             is {max}); heavy video can drop voice. Raise it on the host: \
             sysctl -w net.core.rmem_max={bytes}",
            granted.granted
        );
    } else {
        info!(
            "udp receive buffer: asked for {bytes} bytes, the kernel granted {}",
            granted.granted
        );
    }
    Some(granted)
}

/// `net.core.rmem_max`, the most a socket may ask for without `CAP_NET_ADMIN`. Linux only.
fn rmem_max() -> Option<u64> {
    std::fs::read_to_string("/proc/sys/net/core/rmem_max")
        .ok()?
        .trim()
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_buffer_is_granted_in_full() {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        // 64 KiB is under every default rmem_max.
        let g = set_recv_buffer(&socket, 64 * 1024).unwrap();
        assert_eq!(g.asked, 64 * 1024);
        assert!(!g.capped(), "{g:?}");
    }

    #[test]
    fn zero_keeps_the_default() {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let before = SockRef::from(&socket).recv_buffer_size().unwrap();
        assert_eq!(set_recv_buffer(&socket, 0), None);
        assert_eq!(SockRef::from(&socket).recv_buffer_size().unwrap(), before);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_buffer_over_rmem_max_is_capped() {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let max = usize::try_from(rmem_max().unwrap()).unwrap();
        let g = set_recv_buffer(&socket, max + 1024 * 1024).unwrap();
        assert!(g.capped(), "{g:?}");
        assert_eq!(g.granted, max * 2);
    }

    #[test]
    fn capped_compares_with_the_doubled_value_on_linux() {
        let full = Granted {
            asked: 1000,
            granted: 1000 * GRANT_FACTOR,
        };
        assert!(!full.capped());
        let short = Granted {
            asked: 1000,
            granted: 1000 * GRANT_FACTOR - 1,
        };
        assert!(short.capped());
    }
}
