use std::mem;
use std::slice;
use std::io;

use generic::*;

/// Appears at the beginning of every valid packet.
pub const MAGIC_STRING: [u8; 8] = *b"RUSTYNET";

/// Determines the size of buffers used for receiving packets. The MTU for Ethernet is 1500 so this
/// should be pretty safe.
pub const MAX_PACKET_SIZE: usize = 1500;

/// Stores metadata (some might call this the packet header).
#[repr(C)]
#[derive(Clone, Debug, PartialEq)]
pub struct PacketMetadata {
    dummy: u32,
}

impl PacketMetadata {
    fn bytes(&self) -> &[u8] {
        unsafe {
            let metadata_ptr = self as *const PacketMetadata;
            let bytes_ptr = metadata_ptr as *const u8;
            slice::from_raw_parts(bytes_ptr, mem::size_of::<PacketMetadata>())
        }
    }
}

/// Stores all of the data associated with a packet.
///
/// Note that this struct is supposed to be designed such that it will always be "safe" ie. it will
/// always be safe to call any functions on this struct.
pub struct PacketBuffer {
    buf: [u8; MAX_PACKET_SIZE],
    len: usize,
}

impl PacketBuffer {
    /// Constructs a packet using the given metadata and user data.
    ///
    /// Panics if too much user data is provided. (Eventually this should be impossible once
    /// fragmentation is supported. That's why this is a panic and not an error.)
    pub fn new(metadata: &PacketMetadata, user_data: &[u8]) -> Box<PacketBuffer> {
        let total_len = PacketBuffer::min_packet_length() + user_data.len();
        if PacketBuffer::min_packet_length() + user_data.len() > MAX_PACKET_SIZE {
            panic!("Too much data");
        }
        let mut buf = Box::new(PacketBuffer {
            buf: [0; MAX_PACKET_SIZE],
            len: PacketBuffer::min_packet_length() + user_data.len(),
        });
        buf.buf[0..MAGIC_STRING.len()].clone_from_slice(&MAGIC_STRING);
        buf.buf[MAGIC_STRING.len()..PacketBuffer::min_packet_length()]
            .clone_from_slice(metadata.bytes());
        buf.buf[PacketBuffer::min_packet_length()..total_len].clone_from_slice(user_data);
        buf
    }

    /// Tries to recv a packet from the socket.
    ///
    /// TODO BUFFER_POOL use a buffer pool to avoid excessive heap allocation.
    pub fn recv<S: Socket>(socket: S) -> io::Result<Box<PacketBuffer>> {
        let mut buf = Box::new(PacketBuffer {
            buf: [0; MAX_PACKET_SIZE],
            len: 0,
        });
        let (amt, src) = try!(socket.recv_from(&mut buf.buf));

        // We check these here so users never end up with a partially valid packet.
        if amt < PacketBuffer::min_packet_length() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Packet was too short."));
        } else if buf.buf[0..MAGIC_STRING.len()] != MAGIC_STRING {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Incorrect magic string."));
        }

        buf.len = amt;
        Ok(buf)
    }

    /// Gets the minimum number of bytes needed to store a valid packet (equal to the size of the
    /// PacketMetadata struct plus the size of the MAGIC_STRING".
    ///
    /// TODO CONST_FN make this const fn when that becomes stable.
    pub fn min_packet_length() -> usize {
        mem::size_of::<PacketMetadata>() + MAGIC_STRING.len()
    }

    fn magic_string(&self) -> &[u8] {
        &self.buf[0..MAGIC_STRING.len()]
    }

    /// Gets the packet metadata stored in this buffer.
    ///
    /// This works by reinterpreting the buffer as a PacketMetadata.
    pub fn metadata(&self) -> &PacketMetadata {
        unsafe {
            let bytes_ptr = &self.buf as *const u8;
            let metadata_ptr =
                bytes_ptr.offset(MAGIC_STRING.len() as isize) as *const PacketMetadata;
            &*metadata_ptr
        }
    }

    /// Get the data portion of the buffer (AKA the part provided by the user).
    pub fn data(&self) -> &[u8] {
        unsafe {
            let bytes_ptr = &self.buf as *const u8;
            let data_ptr =
                bytes_ptr.offset(PacketBuffer::min_packet_length() as isize) as *const u8;
            slice::from_raw_parts(data_ptr, self.len - PacketBuffer::min_packet_length())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic]
    fn too_much_data() {
        PacketBuffer::new(&PacketMetadata { dummy: 0 }, &[0; MAX_PACKET_SIZE]);
    }

    #[test]
    fn make_buffer() {
        let metadata = PacketMetadata { dummy: 0 };
        let user_data = [0; 10];
        let buf = PacketBuffer::new(&metadata, &user_data);
        assert_eq!(buf.magic_string(), MAGIC_STRING);
        assert_eq!(buf.metadata(), &metadata);
        assert_eq!(buf.data(), &user_data);
    }
}
