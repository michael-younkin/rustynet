use std::net::SocketAddr;
use std::mem;
use std::slice;
use std::io;
use std::collections::VecDeque;
use std::cell::RefCell;
use std::ops::{Deref, DerefMut};

use generic::*;

type SeqNum = u32;

/// Has to match in the same place in every packet.
pub const MAGIC_NUMBER: u8 = 8;

/// Determines the size of buffers used for receiving packets. The MTU for Ethernet is 1500 so this
/// should be pretty safe.
pub const MAX_PACKET_SIZE: usize = 1500;

quick_error! {
    #[derive(Debug)]
    pub enum MessageError {
        Io(err: io::Error) {
            from()
        }
        InvalidHeader {}
        NotEnoughData {}
        UnknownMessageType {}
    }
}

enum Message<'p> {
    /// Sent at the start of the connection handshake.
    ConnectStart {
        src: SocketAddr,
    },
    /// Sent in response to handshake start.
    ConnectConfirm {
        num_channels: u8,
        seq_nums: &'p [SeqNum],
    },
    /// Sent to confirm complete connection
    ConnectComplete {
        num_channels: u8,
        seq_nums: &'p [SeqNum],
    },
}

impl<'p> Message<'p> {
    fn from_packet(packet: &'p Packet) -> Result<Message<'p>, MessageError> {
        let header = try!(packet.header());

        // Verify the magic number
        if header.magic_number != MAGIC_NUMBER {
            return Err(MessageError::InvalidHeader);
        }

        Ok(match header.message_type {
            0 => Message::ConnectStart { src: packet.addr() },
            1 => {
                let channels = try!(packet.channels());
                Message::ConnectConfirm {
                    num_channels: channels.num_channels,
                    seq_nums: channels.seq_nums,
                }
            }
            2 => {
                let channels = try!(packet.channels());
                Message::ConnectComplete {
                    num_channels: channels.num_channels,
                    seq_nums: channels.seq_nums,
                }
            }
            _ => return Err(MessageError::UnknownMessageType),
        })
    }
}

struct Packet {
    buf: PacketBuffer,
    addr: SocketAddr,
}

impl Packet {
    fn new<S: Socket>(socket: S) -> Result<Packet, MessageError> {
        let mut buf = PacketBuffer {
            buf: [0; MAX_PACKET_SIZE],
            len: 0,
        };

        let (amt, addr) = try!(socket.recv_from(&mut buf.buf));
        buf.len = amt;

        Ok(Packet {
            buf: buf,
            addr: addr,
        })
    }

    fn addr(&self) -> SocketAddr {
        self.addr
    }

    fn header(&self) -> Result<&PacketHeader, MessageError> {
        self.buf.metadata()
    }

    fn channels(&self) -> Result<Channels, MessageError> {
        self.buf.channels()
    }
}

#[repr(C)]
struct PacketHeader {
    seq_num: SeqNum,
    magic_number: u8,
    channel: u8,
    message_type: u8,
}

struct Channels<'p> {
    num_channels: u8,
    seq_nums: &'p [SeqNum],
}

struct PacketBuffer {
    buf: [u8; MAX_PACKET_SIZE],
    len: usize,
}

impl PacketBuffer {
    fn metadata(&self) -> Result<&PacketHeader, MessageError> {
        if self.len < mem::size_of::<PacketHeader>() {
            return Err(MessageError::NotEnoughData);
        }
        unsafe {
            let bytes_ptr = &self.buf as *const u8;
            let metadata_ptr = bytes_ptr as *const PacketHeader;
            Ok(&*metadata_ptr)
        }
    }

    fn channels<'p>(&'p self) -> Result<Channels<'p>, MessageError> {
        let offset = mem::size_of::<PacketHeader>();

        // Make sure we can read the number of channels
        if self.len < offset + 1 {
            return Err(MessageError::NotEnoughData);
        }
        let num_channels = self.buf[offset];

        // Make sure we can read all the channel sequence numbers
        if self.len < offset + 1 + (num_channels as usize) * mem::size_of::<SeqNum>() {
            return Err(MessageError::NotEnoughData);
        }

        // Reinterpret as u32 and create a slice
        let seq_nums = unsafe {
            let bytes_ptr = self.buf.as_ptr();
            let seq_nums_ptr = bytes_ptr.offset((offset + 1) as isize) as *const u32;
            slice::from_raw_parts(seq_nums_ptr, num_channels as usize)
        };
        Ok(Channels {
            num_channels: self.buf[offset],
            seq_nums: seq_nums,
        })
    }
}
