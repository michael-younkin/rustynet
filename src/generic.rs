use std::io;
use std::net::{UdpSocket, SocketAddr};

/// Represents a thing that acts like a networking socket. Arguably this should be provided by the
/// standard library (but it isn't for now). This is necessary to let us mock socket's in unit
/// tests.
pub trait Socket {
    fn recv_from(&self, &mut [u8]) -> io::Result<(usize, SocketAddr)>;
    fn send_to(&self, &[u8], addr: SocketAddr) -> io::Result<usize>;
}

/// UdpSocket's are also Socket's. Methods are declared as inline to avoid any overhead from the
/// indirection.
impl Socket for UdpSocket {
    #[inline]
    fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        self.recv_from(buf)
    }

    #[inline]
    fn send_to(&self, buf: &[u8], addr: SocketAddr) -> io::Result<(usize)> {
        self.send_to(buf, addr)
    }
}
