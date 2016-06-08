use std::net::{SocketAddr, UdpSocket, ToSocketAddrs};
use std::mem;

/// Appears at the beginning of every valid packet.
pub const MAGIC_STRING: [u8; 8] = *b"RUSTYNET";
/// Determines the size of buffers used for receiving packets. The MTU for Ethernet is 1500 so this
/// should be pretty safe.
pub const MAX_PACKET_SIZE: usize = 1500;

#[repr(C)]
struct PacketMetadata {
}

struct PacketBuffer {
    buf: [u8; MAX_PACKET_SIZE],
    len: usize,
}

impl PacketBuffer {
    fn recv<S: Socket>(socket: S) -> std::io::Result<PacketBuffer> {
        let mut buffer = Box::new(PacketBuffer {
            buf: [0; MAX_PACKET_SIZE],
            len: 0,
        });
        let (amt, src) = try!(socket.recv_from(&mut buffer.buf));
        buffer.len = amt;
        buffer
    }
}

pub enum PacketParsingError {
    TooFewBytes,
    InvalidMagicString,
    InvalidMessageType,
}

#[repr(C)]
/// Represents the sized components of a packet. Any multibyte components will have network
/// endianness. This struct is meant to be used inside a Packet.
pub struct PacketSized {
    magic_string: [u8; 8],
    message_type: u8,
    sequence_num: u32,
}

impl PacketSized {
    fn new(message_type: MessageType, sequence_num: u32) -> PacketSized {
        PacketSized {
            magic_string: MAGIC_STRING,
            message_type: message_type.into_u8(),
            sequence_num: sequence_num,
        }
    }

    fn to_bytes(&self) -> &[u8] {
        let sized_ptr = self as *const PacketSized;
        let bytes_ptr = sized_ptr as *const u8;
        unsafe { std::slice::from_raw_parts(bytes_ptr, mem::size_of::<PacketSized>()) }
    }
}

#[repr(C)]
/// Represents a packet. This is an unsized type because the payload portion (application data)
/// could be any length (including 0).
pub struct Packet {
    sized: PacketSized,
    payload: [u8]
}

impl Packet {
    /// Reinterprets a byte slice as a packet. Errors if there are not enough bytes for the sized
    /// portion of the packet.
    fn from_bytes(bytes: &[u8]) -> Result<&Packet, PacketParsingError> {
        if bytes.len() < mem::size_of::<PacketSized>() {
            return Err(PacketParsingError::TooFewBytes)
        }
        let bytes_ptr = bytes as *const [u8];
        let packet_ptr = bytes_ptr as *const Packet;
        let packet = unsafe { &*packet_ptr };
        if packet.sized.magic_string != MAGIC_STRING {
            return Err(PacketParsingError::InvalidMagicString)
        }
        if let Err(_) = MessageType::from_u8(packet.sized.message_type) {
            return Err(PacketParsingError::InvalidMessageType)
        }
        Ok(packet)
    }

    /// Gets the message type of the packet.
    ///
    /// Note: this can panic if somehow the message_type was changed between the packet's
    /// construction and this method being called. In other words, the invariant is checked during
    /// construction, not in this method.
    fn message_type(&self) -> MessageType {
        MessageType::from_u8(self.sized.message_type).unwrap()
    }

    /// Gets the sequence number of the packet.
    fn sequence_num(&self) -> u32 {
        // TODO endianness conversion
        self.sized.sequence_num
    }

    /// Gets the payload of the packet. This may be any length (though one would expect it to be
    /// less than 1500 bytes).
    fn payload(&self) -> &[u8] {
        &self.payload
    }
}

#[derive(Copy, Clone, Debug)]
pub enum MessageType {
    Heartbeat,
    UnreliablePayload,
}

impl MessageType {
    fn from_u8(v: u8) -> std::result::Result<MessageType, ()> {
        match v {
            1 => Ok(MessageType::Heartbeat),
            2 => Ok(MessageType::UnreliablePayload),
            _ => Err(()),
        }
    }

    fn into_u8(self) -> u8 {
        match self {
            MessageType::Heartbeat => 1,
            MessageType::UnreliablePayload => 2,
        }
    }
}

/// Represents a thing that acts like a networking socket. Arguably this should be provided by the
/// standard library (but it isn't for now). This is necessary to let us mock socket's in unit
/// tests.
pub trait Socket {
    fn recv(&self, &mut [u8]) -> std::io::Result<(usize, SocketAddr)>;
    fn send(&self, &[u8], addr: SocketAddr) -> std::io::Result<usize>;
}

/// UdpSocket's are also Socket's. Methods are declared as inline to avoid any overhead from the
/// indirection.
impl Socket for UdpSocket {
    #[inline]
    fn recv(&self, buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr)> {
        self.recv_from(buf)
    }

    #[inline]
    fn send(&self, buf: &[u8], addr: SocketAddr) -> std::io::Result<(usize)> {
        self.send_to(buf, addr)
    }
}

/// A RustyNet server.
pub struct Host<S: Socket> {
    socket: S,
}

impl<S: Socket> Host<S> {
    /// Constructs a new Host instance using an already constructed Socket. This is helpful for
    /// unit testing.
    fn new(socket: S) -> Host<S> {
        Host {
            socket: socket,
        }
    }

    fn service(&self) -> Vec<SocketAddr> {
        let mut sources = Vec::new();

        let mut buf = [0; MAX_PACKET_SIZE];
        while let Ok((_, src)) = self.socket.recv(&mut buf) {
            sources.push(src);
        }

        sources
    }
}

/// Constructs a Host using a UdpSocket. This is what most people should use to make a server.
/// Uses the first valid SocketAddr produced by the ToSocketAddrs object.
pub fn bind_server<A: ToSocketAddrs>(addr: A) -> Result<Host<UdpSocket>, Box<std::error::Error>> {
    let socket = try!(UdpSocket::bind(addr));
    try!(socket.set_nonblocking(true));
    Ok(Host::new(socket))
}

#[cfg(test)]
mod tests {

    use std;
    use std::net::SocketAddr;
    use std::cell::Cell;
    use std::cmp;

    use super::*;

    struct MockSock<'m> {
        messages: &'m [(SocketAddr, &'m [u8])],
        i: Cell<usize>,
    }

    impl<'m> MockSock<'m> {
        fn new(messages: &'m [(SocketAddr, &'m [u8])]) -> MockSock {
            MockSock {
                messages: messages,
                i: Cell::new(0),
            }
        }
    }

    impl<'m> Socket for MockSock<'m> {
        fn recv(&self, buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr)> {
            let i = self.i.get();
            if i == self.messages.len() {
                return Err(std::io::Error::new(
                        std::io::ErrorKind::WouldBlock, "No more packets to read."))
            }
            let (addr, src_buf) = self.messages[i];
            self.i.set(i + 1);
            let amt_read = cmp::min(buf.len(), src_buf.len());
            buf[0..amt_read].clone_from_slice(&src_buf[0..amt_read]);
            Ok((src_buf.len(), addr))
        }

        fn send(&self, buf: &[u8], addr: SocketAddr) -> std::io::Result<(usize)> {
            Ok(0)
        }
    }

    #[test]
    fn can_setup_nonblocking_socket() {
        let server = bind_server("127.0.0.1:10101");
    }

    #[test]
    fn can_read_from_socket() {
        let addr = "178.32.3.2:12333".parse().unwrap();
        let message_1 = PacketSized::new(MessageType::Heartbeat, 0);
        let message_2 = PacketSized::new(MessageType::Heartbeat, 123);
        let messages = &[(addr, message_1.to_bytes()), (addr, message_2.to_bytes())];
        let mock_sock = MockSock::new(messages);
        let server = Host::new(mock_sock);
        let events = server.service();
        assert_eq!(events[0], addr);
        assert_eq!(events[1], addr);
    }
}
