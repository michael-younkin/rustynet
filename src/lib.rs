use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::io;
use std::io::{Cursor, Read};
use std::collections::{HashMap, VecDeque};

#[macro_use]
extern crate quick_error;

extern crate byteorder;
use byteorder::{NetworkEndian, ReadBytesExt};

extern crate rand;
use rand::Rng;

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
    ConnectRequestReceived,
    ConnectRequestSent,
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

    /// The next syn numbers expected from the peer.
    peer_syns: Vec<u32>,

    /// The next syn numbers expected from the host.
    local_syns: Vec<u32>,
}

impl Peer {
    fn new(addr: SocketAddr, state: PeerState) -> Peer {
        Peer {
            state: state,
            addr: addr,
            outgoing_packets: Vec::new(),
            zombie_packets: Vec::new(),
            peer_syns: Vec::new(),
            local_syns: Vec::new(),
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
    ConnectComplete,
    Data {
        syn: u32,
        channel: u8,
        data: Vec<u8>,
    },
    Disconnect,
}

impl Message {
    /// Generates a connect handshake message.
    ///
    /// Can panic if num_channels is 0.
    fn gen_connect_handshake(num_channels: u8) -> Message {
        Message::ConnectHandshake {
            num_channels: num_channels,
            syns: gen_syns(num_channels),
        }
    }
}

#[derive(Copy, Clone, Debug)]
enum MessageKind {
    ConnectStart,
    ConnectHandshake,
    ConnectComplete,
    Data,
    Disconnect,
}

impl MessageKind {
    fn to_u8(self) -> u8 {
        match self {
            MessageKind::ConnectStart => 1,
            MessageKind::ConnectHandshake => 2,
            MessageKind::ConnectComplete => 3,
            MessageKind::Data => 4,
            MessageKind::Disconnect => 5,
        }
    }

    fn from_u8(v: u8) -> Option<MessageKind> {
        Some(match v {
            1 => MessageKind::ConnectStart,
            2 => MessageKind::ConnectHandshake,
            3 => MessageKind::ConnectComplete,
            4 => MessageKind::Data,
            5 => MessageKind::Disconnect,
            _ => return None,
        })
    }
}

/// Generates syn numbers.
///
/// Panics if num_channels is 0.
fn gen_syns(num_channels: u8) -> Vec<u32> {
    if num_channels == 0 {
        panic!("Hosts must have at least 1 channel.");
    }
    let mut syns = Vec::with_capacity(num_channels);
    let mut rng = rand::thread_rng();
    for _ in 0..num_channels {
        syns.push(rng.gen());
    }
    syns
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

            match message {
                Message::ConnectStart => {
                    // If we already have the peer registered, then ignore this message
                    if self.peers.contains_key(addr) {
                        continue;
                    }

                    // Initialize a new peer
                    let mut peer = Peer::new(addr, PeerState::ConnectRequestReceived);
                    peer.local_syns.extend(gen_syns(self.num_channels).iter().cloned());

                    // Setup an outgoing packet
                    peer.outgoing_packets.push(Packet {
                        addr: addr,
                        kind: Message::ConnectHandshake {
                            num_channels: self.num_channels,
                            syns: peer.local_syns.clone(),
                        },
                    });
                }
                Message::ConnectHandshake => {
                    // Ignore unknown peers
                    let mut peer = match self.peers.get_mut(addr) {
                        Some(peer) => peer,
                        None => continue,
                    };

                    // Ignore unexpected handshakes
                }
            }
        }
        Ok(())
    }

    fn parse_message(&mut self, buf: &[u8]) -> io::Result<Message> {
        let mut cursor = Cursor::new(buf);

        // Check the magic number
        if try!(cursor.read_u32::<NetworkEndian>()) != MAGIC_NUMBER {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Magic number does not match."));
        }

        // Get the message kind
        let msg_kind = match MessageKind::from_u8(try!(cursor.read_u32::<NetworkEndian>())) {
            Some(kind) => kind,
            None => {
                return Err(io::Error::new(io::ErrorKind::InvalidInput("Unknown message type.")))
            }
        };
        let msg = match msg_kind {
            MessageKind::ConnectStart => Message::ConnectStart,

            MessageKind::ConnectHandshake => {
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

            MessageKind::Data => {
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

            MessageKind::Disconnect => Message::Disconnect,

            MessageKind::ConnectComplete => Message::ConnectComplete,

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
