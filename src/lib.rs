use std::net::{UdpSocket, ToSocketAddrs};

#[macro_use]
extern crate quick_error;

mod packet;
use packet::*;
mod generic;
use generic::*;
mod error;
use error::*;

/// A RustyNet server.
pub struct Host<S: Socket> {
    socket: S,
}

impl<S: Socket> Host<S> {
    /// Constructs a new Host instance using an already constructed Socket. This is helpful for
    /// unit testing.
    fn new(socket: S) -> Host<S> {
        Host { socket: socket }
    }
}

/// Constructs a host using a UdpSocket. This is what most people should use to create a Host. If
/// you want to make a Host with your own socket impl (not sure why you'd want to do that) you can
/// use Host::new.
pub fn bind_host<A: ToSocketAddrs>(addr: A) -> Result<Host<UdpSocket>, Error> {
    let socket = try!(UdpSocket::bind(addr));
    try!(socket.set_nonblocking(true));
    Ok(Host::new(socket))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANY_IP_AND_PORT: &'static str = "0.0.0.0:0";

    #[test]
    fn can_create_host() {
        bind_host(ANY_IP_AND_PORT).unwrap();
    }
}
