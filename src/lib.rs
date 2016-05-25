use std::net::SocketAddr;
use std::error::Error;
use std::thread;
use std::collections::HashSet;

extern crate mio;
use mio::udp::UdpSocket;

const MAX_PACKET_SIZE: usize = 64000;
const MESSAGE_PREFIX: &'static str = "RNETMSG";

#[derive(PartialEq, Clone)]
pub enum Event {
    Connect(SocketAddr),
    Disconnect(SocketAddr),
    Receive(SocketAddr, Vec<u8>),
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
            socket: try!(UdpSocket::bound(&addr)),
            clients: HashSet::new(),
        })
    }

    fn service(&mut self) -> Result<Vec<Event>, Box<Error>> {
        let mut events = Vec::new();
        let mut buf = Vec::with_capacity(MAX_PACKET_SIZE);

        // TODO handle truncation errors (possibly with a panic)
        while let Some((data_len, src)) = try!(self.socket.recv_from(&mut buf)) {
            let prefix = &buf[0..MESSAGE_PREFIX.len()];
            // Ignore anything that doesn't start with our prefix
            if prefix != MESSAGE_PREFIX.as_bytes() {
                continue;
            }

            let message_type = buf[MESSAGE_PREFIX.len()];
            let message_payload = &buf[MESSAGE_PREFIX.len() + 1..data_len];
            match From::from(message_type) {
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
                        events.push(Event::Receive(src, From::from(message_payload)));
                    }
                },
                _ => ()
            }
        }

        Ok(events)
    }
}

#[test]
fn it_works() {
    thread::spawn(|| {
        do_server();
    });
    thread::spawn(|| {
        do_client();
    }).join();
}
