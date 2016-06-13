use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::io;
use std::io::{Cursor, Read};
use std::collections::{HashMap, VecDeque};

#[macro_use]
extern crate quick_error;

extern crate byteorder;
use byteorder::{NetworkEndian, ReadBytesExt};

/// Magic number used to help differentiate packets intended for a different protocol.
const MAGIC_NUMBER: u32 = 412;

/// The largest number of bytes to be stored in a single packet. Most packets should really be much
/// smaller. This number comes from the MTU of Ethernet. There might be a better value, but this is
/// the best I have for now.
const MAX_PACKET_SIZE: usize = 1500;

/// Provides the ability to recv and send on a socket.
///
/// Ideally there would be a trait like this in the standard library, but ATM there isn't. Writing
/// a trait allows for the use of generics, which allow for mock `Socket`s in unit testing.
trait Socket {
    fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)>;
    fn send_to<A: ToSocketAddrs>(&self, buf: &[u8], addr: A) -> io::Result<usize>;
}

impl Socket for UdpSocket {
    #[inline]
    fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        self.recv_from(buf)
    }

    #[inline]
    fn send_to<A: ToSocketAddrs>(&self, buf: &[u8], addr: A) -> io::Result<usize> {
        self.send_to(buf, addr)
    }
}

/// A connection related event (such as a client connecting, disconnecting, or sending or receiving
/// data).
enum Event {
}

/// The connection status of a peer.
enum PeerState {
    Connecting,
    Connected,
}

/// A peer for a `Host`.
struct Peer {
    /// The current state of this peer.
    state: PeerState,

    /// The IP and Port of this peer.
    addr: SocketAddr,

    /// Packets to be sent to this peer at the next available opportunity.
    outgoing_packets: Vec<Packet>,

    /// Packets that may or may not have been received by this peer.
    zombie_packets: Vec<Packet>,

    /// The sync number for each channel.
    channel_syns: Vec<u32>,
}

impl Peer {
    fn new(addr: SocketAddr) -> Peer {
        Peer {
            state: PeerState::Connecting,
            addr: addr,
            outgoing_packets: Vec::new(),
            zombie_packets: Vec::new(),
            channel_syns: Vec::new(),
        }
    }

    /// Queue a packet to be sent.
    fn queue_msg(&mut self, packet: Packet) {
        self.outgoing_packets.push(packet);
    }
}

enum Message {
    ConnectStart,
    ConnectHandshake {
        num_channels: u8,
        syns: Vec<u32>,
    },
    Data {
        syn: u32,
        channel: u8,
        data: Vec<u8>,
    },
    Disconnect,
}

struct Packet {
    addr: SocketAddr,
    kind: Message,
}

struct Host<S: Socket> {
    /// The socket used for network communication.
    socket: S,

    /// Used for storing events.
    events: Vec<Event>,

    /// Peers currently connected to the host.
    peers: HashMap<SocketAddr, Peer>,

    /// The number of communication channels available.
    num_channels: u8,
}

impl<S: Socket> Host<S> {
    /// Constructs a host using a user specified socket. Users should typically use the `bind_host`
    /// function instead of `Host::new`.
    fn new(socket: S, num_channels: u8) -> Host<S> {
        if num_channels == 0 {
            panic!("Hosts must have at least 1 channel.");
        }
        Host {
            socket: socket,
            events: Vec::new(),
            peers: HashMap::new(),
            num_channels: num_channels,
        }
    }

    fn service(&mut self) -> io::Result<&[Event]> {
        self.events.clear();
        try!(self.read_incoming());
        Ok(&self.events)
    }

    fn read_incoming(&mut self) -> io::Result<()> {
        loop {
            // Recv
            let mut buf = [0; MAX_PACKET_SIZE];
            let (amt, addr) = match self.socket.recv_from(&mut buf) {
                Ok(v) => v,
                // TODO verify that WouldBlock works for EAGAIN and EWOULDBLOCK
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
                // Bail out completely if we see a "real" error
                // TODO consider invalidating the struct somehow since we'll probably be in a bad
                // state after getting an IO error.
                Err(e) => return Err(e),
            };

            let message = match self.parse_message(&buf[0..amt]) {
                Ok(m) => m,
                Err(_) => continue,
            };
        }
        Ok(())
    }

    fn parse_message(&mut self, buf: &[u8]) -> io::Result<Message> {
        let mut cursor = Cursor::new(buf);

        // Check the magic number
        if try!(cursor.read_u32::<NetworkEndian>()) != MAGIC_NUMBER {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Magic number does not match."));
        }

        // Get the message type
        let msg = match try!(cursor.read_u32::<NetworkEndian>()) {
            // A "Start Connection" message
            1 => Message::ConnectStart,

            // A "ConnectHandshake" message
            2 => {
                let num_channels = try!(cursor.read_u8());
                if num_channels == 0 {
                    return Err(io::Error::new(io::ErrorKind::InvalidInput,
                                              "Number of channels was 0."));
                }

                let mut syns = Vec::with_capacity(num_channels as usize);
                for _ in 0..num_channels {
                    syns.push(try!(cursor.read_u32::<NetworkEndian>()));
                }
                Message::ConnectHandshake {
                    num_channels: num_channels,
                    syns: syns,
                }
            }

            // A "Data" message
            3 => {
                let syn = try!(cursor.read_u32::<NetworkEndian>());
                let channel = try!(cursor.read_u8());
                let mut data = Vec::new();
                try!(cursor.read_to_end(&mut data));
                Message::Data {
                    syn: syn,
                    channel: channel,
                    data: data,
                }
            }

            // A "Disconnect" message
            4 => Message::Disconnect,

            _ => return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid message type.")),
        };

        // Make sure there's no extra data in the packet (which could indicate corruption)
        if cursor.get_ref().len() as u64 != cursor.position() {
            Err(io::Error::new(io::ErrorKind::InvalidInput, "Trailing data."))
        } else {
            Ok(msg)
        }
    }
}

/// Constructs a host using a UDP Socket from the standard library.
fn bind_host<A: ToSocketAddrs>(addr: A, num_channels: u8) -> io::Result<Host<UdpSocket>> {
    let socket = try!(UdpSocket::bind(addr));
    try!(socket.set_nonblocking(true));
    Ok(Host::new(socket, num_channels))
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
