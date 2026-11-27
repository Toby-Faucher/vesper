//! Reading the media socket. On Linux one `recvmmsg` call takes every datagram that is queued,
//! up to [`BATCH`], so the SFU thread pays one system call per burst instead of one per packet.
//! Elsewhere it reads one datagram per call, as before. Either way the call waits up to the
//! socket's read timeout for the first datagram and never for the rest.

use std::io;
use std::net::{SocketAddr, UdpSocket};

/// Most datagrams one call returns. A 20 Mbps share sends a burst of about 35 packets per frame.
pub(crate) const BATCH: usize = 32;

/// Larger than any datagram a WebRTC peer sends (str0m's MTU is 1150 bytes).
const DATAGRAM: usize = 2000;

pub(crate) struct Reader {
    bufs: Vec<[u8; DATAGRAM]>,
    /// Length and source of each datagram of the last call, in order.
    got: Vec<(usize, Option<SocketAddr>)>,
    #[cfg(target_os = "linux")]
    headers: nix::sys::socket::MultiHeaders<nix::sys::socket::SockaddrStorage>,
}

impl Reader {
    pub fn new() -> Reader {
        let n = if cfg!(target_os = "linux") { BATCH } else { 1 };
        Reader {
            bufs: vec![[0; DATAGRAM]; n],
            got: Vec::with_capacity(n),
            #[cfg(target_os = "linux")]
            headers: nix::sys::socket::MultiHeaders::preallocate(n, None),
        }
    }

    /// Wait for at least one datagram, up to the socket's read timeout, and read what is queued.
    /// A timeout is an error of kind `WouldBlock` (or `TimedOut`). Then [`packets`](Self::packets)
    /// gives the datagrams.
    pub fn recv(&mut self, socket: &UdpSocket) -> io::Result<usize> {
        self.got.clear();
        self.recv_inner(socket)?;
        Ok(self.got.len())
    }

    /// The datagrams of the last [`recv`](Self::recv), in the order they arrived.
    pub fn packets(&self) -> impl Iterator<Item = (&[u8], SocketAddr)> {
        self.got
            .iter()
            .zip(&self.bufs)
            .filter_map(|(&(n, source), buf)| Some((&buf[..n], source?)))
    }

    #[cfg(target_os = "linux")]
    fn recv_inner(&mut self, socket: &UdpSocket) -> io::Result<()> {
        use std::io::IoSliceMut;
        use std::os::fd::AsRawFd;

        use nix::sys::socket::{MsgFlags, recvmmsg};

        let mut slices: Vec<[IoSliceMut<'_>; 1]> =
            self.bufs.iter_mut().map(|b| [IoSliceMut::new(b)]).collect();
        // MSG_WAITFORONE: block (up to SO_RCVTIMEO) for the first datagram only. The timeout
        // argument of recvmmsg is not used: Linux checks it only after each datagram.
        let results = recvmmsg(
            socket.as_raw_fd(),
            &mut self.headers,
            slices.iter_mut(),
            MsgFlags::MSG_WAITFORONE,
            None,
        )
        .map_err(io::Error::from)?;
        for msg in results {
            // A datagram with no IP source cannot be routed; its slot stays, so the
            // buffers stay in step, but `packets` skips it.
            let source =
                msg.address
                    .and_then(|a| match (a.as_sockaddr_in(), a.as_sockaddr_in6()) {
                        (Some(v4), _) => Some(SocketAddr::V4((*v4).into())),
                        (_, Some(v6)) => Some(SocketAddr::V6((*v6).into())),
                        _ => None,
                    });
            self.got.push((msg.bytes, source));
        }
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    fn recv_inner(&mut self, socket: &UdpSocket) -> io::Result<()> {
        let (n, source) = socket.recv_from(&mut self.bufs[0])?;
        self.got.push((n, Some(source)));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;
    use std::time::Duration;

    use super::*;

    fn pair() -> (UdpSocket, UdpSocket) {
        let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
        rx.set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let tx = UdpSocket::bind("127.0.0.1:0").unwrap();
        tx.connect(rx.local_addr().unwrap()).unwrap();
        (rx, tx)
    }

    /// Everything `recv` returns until the socket is empty, as (payload, source).
    fn drain(reader: &mut Reader, rx: &UdpSocket) -> Vec<Vec<(Vec<u8>, SocketAddr)>> {
        let mut calls = vec![];
        loop {
            match reader.recv(rx) {
                Ok(_) => calls.push(reader.packets().map(|(d, s)| (d.to_vec(), s)).collect()),
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    return calls;
                }
                Err(e) => panic!("{e}"),
            }
        }
    }

    #[test]
    fn every_datagram_arrives_once_in_order_with_its_source() {
        let (rx, tx) = pair();
        let n = BATCH + 8;
        for i in 0..n {
            tx.send(&[i as u8; 3]).unwrap();
        }
        let mut reader = Reader::new();
        let calls = drain(&mut reader, &rx);
        let got: Vec<_> = calls.iter().flatten().cloned().collect();
        let want: Vec<_> = (0..n)
            .map(|i| (vec![i as u8; 3], tx.local_addr().unwrap()))
            .collect();
        assert_eq!(got, want);
        // On Linux the queued datagrams come in full batches; elsewhere one per call.
        let per_call = if cfg!(target_os = "linux") { BATCH } else { 1 };
        assert_eq!(calls[0].len(), per_call);
        assert!(calls.iter().all(|c| c.len() <= per_call));
    }

    #[test]
    fn an_empty_socket_times_out() {
        let (rx, _tx) = pair();
        let mut reader = Reader::new();
        let e = reader.recv(&rx).unwrap_err();
        assert!(
            matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut),
            "{e}"
        );
        assert_eq!(reader.packets().count(), 0);
    }

    #[test]
    fn one_datagram_returns_without_waiting_for_more() {
        let (rx, tx) = pair();
        rx.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        tx.send(b"one").unwrap();
        let mut reader = Reader::new();
        let start = std::time::Instant::now();
        assert_eq!(reader.recv(&rx).unwrap(), 1);
        assert!(start.elapsed() < Duration::from_secs(1));
        assert_eq!(reader.packets().next().unwrap().0, b"one");
    }

    #[test]
    fn a_full_size_datagram_is_not_cut() {
        let (rx, tx) = pair();
        tx.send(&[7; DATAGRAM]).unwrap();
        let mut reader = Reader::new();
        reader.recv(&rx).unwrap();
        assert_eq!(reader.packets().next().unwrap().0.len(), DATAGRAM);
    }
}
