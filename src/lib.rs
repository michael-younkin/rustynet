use std::net::{ToSocketAddrs, SocketAddr, UdpSocket};
use std::time::{Instant, UNIX_EPOCH, Duration};
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::mem::{size_of, transmute};
use std::iter::repeat;
use std::cmp::min;

extern crate byteorder;
use byteorder::{ReadBytesExt, WriteBytesExt, NetworkEndian};

#[macro_use]
extern crate lazy_static;

const MAX_PACKET_SIZE: usize = 64000;
static MESSAGE_PREFIX: &'static str = "RNETMSG";

lazy_static! {
    static ref PROGRAM_START: Instant = Instant::now();
    static ref HEARTBEAT_TIMEOUT: Duration = Duration::new(5, 0);
    static ref HEARTBEAT_INTERVAL: Duration = Duration::new(0, 500000);
}

#[derive(Debug)]
enum Error {
    UnableToBindSocket(std::io::Error),
    UnableToSetNonblocking(std::io::Error),
    UnableToParsePacket,
    UnableToReadFromSocket(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::result::Result<(), std::fmt::Error> {
        match std::error::Error::cause(self) {
            Some(cause) => write!(f, "{}({})", std::error::Error::description(self), cause),
            None => write!(f, "{}", std::error::Error::description(self)),
        }
    }
}

impl std::error::Error for Error {
    fn description(&self) -> &str {
        match *self {
            Error::UnableToBindSocket(_) => "Unable to bind socket",
            Error::UnableToSetNonblocking(_) => "Unable to make socket nonblocking",
            Error::UnableToParsePacket => "Unable to parse packet",
            Error::UnableToReadFromSocket(_) => "Unable to read from socket",
        }
    }

    fn cause(&self) -> Option<&std::error::Error> {
        match *self {
            Error::UnableToBindSocket(ref e) => Some(e),
            Error::UnableToSetNonblocking(ref e) => Some(e),
            Error::UnableToReadFromSocket(ref e) => Some(e),
            _ => None,
        }
    }
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug)]
enum ChannelReliability {
    Unreliable,
}

struct Host {
    socket: UdpSocket,
    channels: Box<[ChannelReliability]>,
}

impl Host {
    fn new<A: ToSocketAddrs>(addr: A, channels: &[ChannelReliability]) -> Result<Host> {
        if channels.len() == 0 {
            panic!("No channels provided.");
        } else if channels.len() > std::u8::MAX as usize {
            panic!("Too many channels provided.");
        }
        let socket = try!(UdpSocket::bind(addr).map_err(Error::UnableToBindSocket));
        try!(socket.set_nonblocking(true).map_err(Error::UnableToSetNonblocking));
        Ok(Host {
            socket: socket,
            channels: channels.to_vec().into_boxed_slice(),
        })
    }

    fn to_server(self) -> Server {
        Server {
            host: self,
            clients: HashMap::new(),
        }
    }
}

#[derive(Copy, Clone, Debug)]
enum ConnectionState {
    Disconnected,
    Connected,
    Connecting,
}

#[derive(Copy, Clone, Debug)]
struct ClientData {
    addr: SocketAddr,
    last_recv_time: Instant,
    last_send_time: Instant,
    status: ConnectionState,
}

impl ClientData {
    fn new(addr: SocketAddr) -> ClientData {
        ClientData {
            addr: addr,
            last_recv_time: *PROGRAM_START,
            last_send_time: *PROGRAM_START,
            status: ConnectionState::Connecting,
        }
    }
}

struct NoCopyCursor<'d> {
    data: &'d [u8],
    pos: usize,
}

impl<'d> NoCopyCursor<'d> {
    fn new(data: &'d [u8]) -> NoCopyCursor {
        NoCopyCursor {
            data: data,
            pos: 0,
        }
    }

    fn read_as_slice(&mut self, amt: usize) -> std::io::Result<&'d [u8]> {
        if self.data.len() == 0 || self.data.len() - self.pos < amt {
            Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof, "Not enough bytes remaining."))
        } else {
            let out = &self.data[self.pos..self.pos + amt];
            self.pos += amt;
            Ok(out)
        }
    }
}

impl<'d> Read for NoCopyCursor<'d> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let amt = min(buf.len(), self.data.len() - self.pos);
        (buf[0..amt]).clone_from_slice(&self.data[0..amt]);
        Ok(amt)
    }
}

enum MessageParseError {
    Io(std::io::Error),
    InvalidPrefix,
    InvalidMessageType,
}

impl From<std::io::Error> for MessageParseError {
    fn from(e: std::io::Error) -> MessageParseError {
        MessageParseError::Io(e)
    }
}

#[derive(Copy, Clone, Debug)]
enum MessageType {
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

enum Message<'p> {
    Heartbeat,
    UnreliablePayload {
        sequence_i: u64,
        data: &'p [u8],
    },
}

impl<'p> Message<'p> {
    fn parse(data: &'p [u8]) -> std::result::Result<Message<'p>, MessageParseError> {
        let mut reader = NoCopyCursor::new(data);

        // Check the prefix
        let prefix = match reader.read_as_slice(MESSAGE_PREFIX.as_bytes().len()) {
            Ok(p) => p,
            Err(_) => return Err(MessageParseError::InvalidPrefix),
        };
        if prefix != MESSAGE_PREFIX.as_bytes() {
            return Err(MessageParseError::InvalidPrefix)
        }

        // Read the message type
        // TODO figure out if there's a nicer way to write this
        let message_type_v = try!(reader.read_u8()
                                  .map_err(|_| MessageParseError::InvalidMessageType));
        let message_type = try!(MessageType::from_u8(message_type_v)
                                .map_err(|_| MessageParseError::InvalidMessageType));

        match message_type {
            MessageType::Heartbeat => Ok(Message::Heartbeat),
            _ => Err(MessageParseError::InvalidMessageType),
        }
    }
}

#[derive(Clone, Debug)]
enum EventType {
    Heartbeat,
}

#[derive(Clone, Debug)]
struct Event {
    client: ClientData,
    variant: EventType,
}

struct Server {
    host: Host,
    clients: HashMap<SocketAddr, ClientData>,
}

impl Server {
    fn service(&mut self) -> Vec<Event> {
        let mut events = Vec::new();

        // Read incoming packets
        loop {
            let mut buf = Vec::new();
            buf.resize(MAX_PACKET_SIZE, 0);

            // TODO handle the case where the socket is messed up separate from the case where
            // there are no messages
            let (len, addr) = match self.host.socket.recv_from(&mut buf) {
                Ok(v) => v,
                Err(v) => break,
            };
            buf.resize(len, 0);

            // Try to parse the payload
            let message = match Message::parse(&buf) {
                Ok(v) => v,
                // TODO log errors (not super important, but they might be corrupted packets - it'd
                // probably be useful to know for debugging though
                Err(_) => break,
            };

            // Get the client data (or create if necessary)
            let mut client_data = self.clients.entry(addr).or_insert_with(|| ClientData::new(addr));
            client_data.last_recv_time = Instant::now();

            // Produce an appropriate event type
            events.push(Event { client: *client_data, variant: EventType::Heartbeat });
        }

        let now = Instant::now();

        // Remove dead clients
        let dead_clients: Vec<_> = self.clients.values()
            .filter(|client| now - client.last_recv_time < *HEARTBEAT_TIMEOUT)
            .map(|client| client.clone())
            .collect();
        for client in dead_clients {
            self.clients.remove(&client.addr);
        }

        // Send a heartbeat message to any quiet clients
        for (addr, client) in self.clients.iter_mut() {
            if now - client.last_send_time >= *HEARTBEAT_INTERVAL {
                client.last_send_time = now;
                // TODO send heartbeat message
            }
        }

        events
    }
}

// TODO add the correct should_panic text
#[test]
#[should_panic]
fn panics_on_0_channels() {
    Host::new("localhost:10100", &[]).unwrap();
}

// TODO add the correct should_panic text
#[test]
#[should_panic]
fn panics_on_too_many_channels() {
    let channels: Vec<ChannelReliability> =
        repeat(ChannelReliability::Unreliable).take(500).collect();
    Host::new("localhost:10101", &channels).unwrap();
}

#[test]
fn accepts_at_least_1_channel() {
    Host::new("localhost:10101", &[ChannelReliability::Unreliable]).unwrap();
    Host::new(
        "localhost:10102",
        &[ChannelReliability::Unreliable, ChannelReliability::Unreliable]
    ).unwrap();
}
