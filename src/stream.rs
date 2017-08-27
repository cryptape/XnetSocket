use bytes::{Buf, BufMut};
use mio::tcp::TcpStream;
use result::{Result, Error, Kind};
use std::io;
use std::io::ErrorKind::WouldBlock;
use std::net::SocketAddr;

fn map_non_block<T>(res: io::Result<T>) -> io::Result<Option<T>> {
    match res {
        Ok(value) => Ok(Some(value)),
        Err(err) => {
            if let WouldBlock = err.kind() { Ok(None) } else { Err(err) }
        }
    }
}

pub trait TryReadBuf: io::Read {
    fn try_read_buf<B: BufMut>(&mut self, buf: &mut B) -> io::Result<Option<usize>>
    where
        Self: Sized,
    {
        let res = map_non_block(self.read(unsafe { buf.bytes_mut() }));
        if let Ok(Some(cnt)) = res {
            unsafe {
                //set_position()
                buf.advance_mut(cnt);
            }
        }

        res
    }
}

pub trait TryWriteBuf: io::Write {
    fn try_write_buf<B: Buf>(&mut self, buf: &mut B) -> io::Result<Option<usize>>
    where
        Self: Sized,
    {
        let res = map_non_block(self.write(buf.bytes()));

        if let Ok(Some(cnt)) = res {
            //set_position()
            buf.advance(cnt);
        }
        res
    }
}

impl<T: io::Read> TryReadBuf for T {}

impl<T: io::Write> TryWriteBuf for T {}

use self::Stream::*;

pub enum Stream {
    Tcp(TcpStream),
}

impl Stream {
    pub fn tcp(stream: TcpStream) -> Stream {
        Tcp(stream)
    }


    pub fn evented(&self) -> &TcpStream {
        match *self {
            Tcp(ref sock) => sock,
        }
    }

    pub fn is_negotiating(&self) -> bool {
        match *self {
            Tcp(_) => false,
        }
    }

    pub fn clear_negotiating(&mut self) -> Result<()> {
        match *self {
            Tcp(_) => Err(Error::new(Kind::Internal, "Attempted to clear negotiating flag on non ssl connection.")),
        }
    }

    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        match *self {
            Tcp(ref sock) => sock.peer_addr(),
        }
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        match *self {
            Tcp(ref sock) => sock.local_addr(),
        }
    }
}

impl io::Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match *self {
            Tcp(ref mut sock) => sock.read(buf),
        }
    }
}

impl io::Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match *self {
            Tcp(ref mut sock) => sock.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match *self {
            Tcp(ref mut sock) => sock.flush(),
        }
    }
}
