use std::net::SocketAddr;
use std::net::UdpSocket;
use std::error::Error;
use std::thread;
use std::thread::sleep;
use std::time::Duration;
use std::collections::HashSet;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fmt;
use std::str::from_utf8;
use std::io;
use std::io::Cursor;
use std::io::Read;

extern crate byteorder;
use byteorder::{ReadBytesExt, WriteBytesExt, NetworkEndian};

const MAX_PACKET_SIZE: usize = 64000;
static MESSAGE_PREFIX: &'static str = "RNETMSG";

#[derive(PartialEq, Clone)]
pub enum Event {
    Connect(SocketAddr),
    Disconnect(SocketAddr),
    Receive(SocketAddr, Vec<u8>),
}

impl Display for Event {
    fn fmt(&self, f: &mut Formatter) -> Result<(), fmt::Error> {
        match *self {
            Event::Connect(addr) => write!(f, "{} connected.", addr),
            Event::Disconnect(addr) => write!(f, "{} disconnected.", addr),
            Event::Receive(addr, ref data) => {
                let s = from_utf8(data).unwrap_or_else(|e| "Error(Invalid UTF8 received)");
                write!(f, "{} sent us \"{}\"", addr, s)
            },
        }
    }
}

#[derive(PartialEq, Copy, Clone)]
enum MessageType {
    Connect,
    Disconnect,
    Receive,
    Unknown
}

impl From<u8> for MessageType {
    fn from(v: u8) -> MessageType {
        match v {
            0 => MessageType::Connect,
            1 => MessageType::Disconnect,
            2 => MessageType::Receive,
            _ => MessageType::Unknown,
        }
    }
}

struct Host {
    addr: SocketAddr,
    socket: UdpSocket,
    clients: HashSet<SocketAddr>,
}

impl Host {
    fn new(addr: SocketAddr) -> Result<Host, Box<Error>> {
        Ok(Host {
            addr: addr,
            socket: {
                let socket = try!(UdpSocket::bind(addr));
                socket.set_nonblocking(true);
                socket
            },
            clients: HashSet::new(),
        })
    }

    fn service(&mut self) -> Result<Vec<Event>, Box<Error>> {
        let mut events = Vec::new();
        let mut buf = Vec::with_capacity(MAX_PACKET_SIZE);

        loop {
            buf.clear();
            let (data_len, src) = match self.socket.recv_from(&mut buf) {
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => continue,
                e @ Err(_) => try!(e),
                Ok(v) => v,
            };
            let mut packet = Cursor::new(&buf[0..data_len]);
            
            // Check for the proper prefix
            // TODO use an array instead of a Vec somehow
            let mut prefix = Vec::with_capacity(MESSAGE_PREFIX.as_bytes().len());
            try!(packet.read_exact(&mut prefix));
            if &prefix != &MESSAGE_PREFIX.as_bytes() {
                continue;
            }

            // Read the message type
            let message_type = From::from(try!(packet.read_u8()));
            match message_type {
                MessageType::Connect => {
                    if self.clients.contains(&src) {
                        events.push(Event::Disconnect(src));
                        events.push(Event::Connect(src));
                    } else {
                        self.clients.insert(src);
                        events.push(Event::Connect(src));
                    }
                },
                MessageType::Disconnect => {
                    if self.clients.contains(&src) {
                        self.clients.remove(&src);
                        events.push(Event::Disconnect(src));
                    } else {
                        events.push(Event::Connect(src));
                        events.push(Event::Disconnect(src));
                    }
                },
                MessageType::Receive => {
                    if self.clients.contains(&src) {
                        let mut data = Vec::new();
                        try!(packet.read_to_end(&mut data));
                        events.push(Event::Receive(src, data));
                    }
                },
                _ => ()
            }
        };

        Ok(events)
    }
}

#[test]
fn it_works() {
    thread::spawn(|| {
        let mut host = Host::new("127.0.0.1:10101".parse().unwrap()).unwrap();
        loop {
            for event in host.service().unwrap() {
                println!("Server: {}", event);
            }
            sleep(Duration::new(0, 1000));
        }
    });
    thread::spawn(|| {
        let addr = "127.0.0.1:10102".parse::<SocketAddr>().unwrap();
        let socket = UdpSocket::bind(addr).unwrap();

        let mut payload = Vec::new();
        payload.extend_from_slice(MESSAGE_PREFIX.as_bytes());
        payload.push(MessageType::Connect as u8);
        socket.send_to(&payload, &addr);
    }).join();
}
