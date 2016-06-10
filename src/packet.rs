use std::net::SocketAddr;
use std::mem;
use std::slice;
use std::io;
use std::collections::VecDeque;
use std::cell::RefCell;
use std::ops::{Deref, DerefMut};

use generic::*;

/// Appears at the beginning of every valid packet.
pub const MAGIC_STRING: [u8; 8] = *b"RUSTYNET";

/// Determines the size of buffers used for receiving packets. The MTU for Ethernet is 1500 so this
/// should be pretty safe.
pub const MAX_PACKET_SIZE: usize = 1500;

#[derive(Copy, Clone, Debug, PartialEq)]
enum MessageType {
    Connect,
}

impl MessageType {
    fn into_u8(self) -> u8 {
        match self {
            MessageType::Connect => 0,
        }
    }

    fn from_u8(v: u8) -> Result<MessageType, ()> {
        match v {
            0 => Ok(MessageType::Connect),
            _ => Err(()),
        }
    }
}

quick_error! {
    #[derive(Debug)]
    pub enum PacketError {
        Io(err: io::Error) {
            from()
        }
        NotEnoughData {}
    }
}

pub struct PacketBufferPool {
    pool: RefCell<VecDeque<Box<PacketBuffer>>>,
}

impl PacketBufferPool {
    fn new() -> PacketBufferPool {
        PacketBufferPool { pool: RefCell::new(VecDeque::new()) }
    }

    fn len(&self) -> usize {
        self.pool.borrow().len()
    }

    fn get_buf(&self) -> PacketBufferHandle {
        let mut mut_pool = self.pool.borrow_mut();
        let buf = match mut_pool.pop_front() {
            Some(buf) => buf,
            None => Box::new(PacketBuffer::new()),
        };
        PacketBufferHandle::new(buf, self)
    }

    fn return_buf(&self, buf: Box<PacketBuffer>) {
        let mut mut_pool = self.pool.borrow_mut();
        mut_pool.push_front(buf);
    }
}

pub struct PacketBufferHandle<'p> {
    pool: &'p PacketBufferPool,
    buf: *mut PacketBuffer,
}

impl<'p> PacketBufferHandle<'p> {
    fn new(buf: Box<PacketBuffer>, pool: &'p PacketBufferPool) -> PacketBufferHandle {
        let ptr = Box::into_raw(buf);
        PacketBufferHandle {
            pool: pool,
            buf: ptr,
        }
    }
}

impl<'p> Deref for PacketBufferHandle<'p> {
    type Target = PacketBuffer;

    fn deref(&self) -> &PacketBuffer {
        unsafe { &*self.buf }
    }
}

impl<'p> DerefMut for PacketBufferHandle<'p> {
    fn deref_mut(&mut self) -> &mut PacketBuffer {
        unsafe { &mut *self.buf }
    }
}

impl<'p> Drop for PacketBufferHandle<'p> {
    fn drop(&mut self) {
        // Clear it now so we don't have to put it in a RefCell inside the pool (because Box is
        // immutable)
        unsafe { (&mut *self.buf).clear() };

        // We do this so buf doesn't need to be an option: forcing self.buf to be a pointer means
        // Rust won't drop it for us.
        let buf = unsafe { Box::from_raw(self.buf) };
        self.pool.return_buf(buf);
    }
}

/// Stores metadata (some might call this the packet header).
#[repr(C)]
#[derive(Clone, Debug, PartialEq)]
pub struct PacketMetadata {
    message_type: u8,
}

impl PacketMetadata {
    fn validate(self) -> Result<(), &'static str> {
        try!(MessageType::from_u8(self.message_type).map_err(|_| "Invalid message type."));
        Ok(())
    }

    fn bytes(&self) -> &[u8] {
        unsafe {
            let metadata_ptr = self as *const PacketMetadata;
            let bytes_ptr = metadata_ptr as *const u8;
            slice::from_raw_parts(bytes_ptr, mem::size_of::<PacketMetadata>())
        }
    }
}

/// A buffer for storing and interpreting data associated with a packet. Accessor functions return
/// errors if there isn't enough data.
pub struct PacketBuffer {
    buf: [u8; MAX_PACKET_SIZE],
    len: usize,
}

impl PacketBuffer {
    fn new() -> PacketBuffer {
        PacketBuffer {
            buf: [0; MAX_PACKET_SIZE],
            len: 0,
        }
    }

    fn clear(&mut self) {
        self.buf.clone_from_slice(&[0; MAX_PACKET_SIZE]);
        self.len = 0;
    }

    /// Uses this buffer to load data from a socket.
    pub fn recv_from<S: Socket>(&mut self, socket: S) -> Result<(usize, SocketAddr), PacketError> {
        let (amt, src) = try!(socket.recv_from(&mut self.buf));
        self.len = amt;
        Ok((amt, src))
    }

    /// Gets the packet metadata stored in this buffer. This works by interpreting the first
    /// mem::size_of::<PacketMetadata>() bytes as a PacketMetadata struct.
    ///
    /// The data returned may be invalid (ie. this might not be a valid packet) but it will all be
    /// actual data recevied from the socket.
    pub fn metadata(&self) -> Result<&PacketMetadata, PacketError> {
        if self.len < mem::size_of::<PacketMetadata>() {
            return Err(PacketError::NotEnoughData);
        }
        unsafe {
            let bytes_ptr = &self.buf as *const u8;
            let metadata_ptr = bytes_ptr as *const PacketMetadata;
            Ok(&*metadata_ptr)
        }
    }

    /// Get the data portion of the buffer (AKA the part provided by the user).
    pub fn data(&self) -> Result<&[u8], PacketError> {
        if self.len < mem::size_of::<PacketMetadata>() {
            return Err(PacketError::NotEnoughData);
        }
        unsafe {
            let bytes_ptr = &self.buf as *const u8;
            let data_ptr = bytes_ptr.offset(mem::size_of::<PacketMetadata>() as isize) as *const u8;
            Ok(slice::from_raw_parts(data_ptr, self.len - mem::size_of::<PacketMetadata>()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_return_buffers() {
        let pool = PacketBufferPool::new();
        fn test(pool: &PacketBufferPool) {
            let buf1 = pool.get_buf();
            let buf2 = pool.get_buf();
            let buf3 = pool.get_buf();
        }
        test(&pool);
        assert_eq!(pool.len(), 3);
    }
}
