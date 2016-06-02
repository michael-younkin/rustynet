use std::net::{ToSocketAddrs, UdpSocket};

extern crate byteorder;
use byteorder::{ReadBytesExt, WriteBytesExt, NetworkEndian};

const MAX_PACKET_SIZE: usize = 64000;
static MESSAGE_PREFIX: &'static str = "RNETMSG";

#[derive(Debug)]
enum Error {
    UnableToBindSocket(std::io::Error),
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
        }
    }

    fn cause(&self) -> Option<&std::error::Error> {
        match *self {
            Error::UnableToBindSocket(ref e) => Some(e),
        }
    }
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug)]
enum ChannelReliability {
    Unreliable,
}

struct Server {
    socket: UdpSocket,
    channels: Box<[ChannelReliability]>,
}

impl Server {
    fn new<A: ToSocketAddrs>(addr: A, channels: &[ChannelReliability]) -> Result<Server> {
        if channels.len() == 0 {
            panic!("No channels provided.");
        }
        let socket = try!(UdpSocket::bind(addr).map_err(Error::UnableToBindSocket));
        Ok(Server {
            socket: socket,
            channels: channels.to_vec().into_boxed_slice(),
        })
    }
}

#[test]
#[should_panic]
fn panics_on_0_channels() {
    Server::new("localhost:10100", &[]);
}

#[test]
fn accepts_at_least_1_channel() {
    Server::new("localhost:10101", &[ChannelReliability::Unreliable]);
    Server::new(
        "localhost:10102",
        &[ChannelReliability::Unreliable, ChannelReliability::Unreliable]
    );
}
