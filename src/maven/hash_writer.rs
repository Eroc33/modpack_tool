use sha1::{Digest, Sha1};
use std::io::{Result, Write};

pub struct HashWriter(Sha1);

impl Default for HashWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl HashWriter {
    pub fn new() -> Self {
        Self(Sha1::new())
    }
    pub fn digest(&self) -> Digest {
        self.0.digest()
    }
}

impl Write for HashWriter {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }
    // can't be flushed so just pretend we did
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

use std::{
    task::{Poll,Context},
    pin::Pin,
};

impl tokio::io::AsyncWrite for HashWriter{
    fn poll_write(
        mut self: Pin<&mut Self>, 
        _cx: &mut Context, 
        buf: &[u8]
    ) -> Poll<tokio::io::Result<usize>>{
        self.as_mut().0.update(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(
        self: Pin<&mut Self>, 
        _cx: &mut Context
    ) -> Poll<tokio::io::Result<()>>{
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: Pin<&mut Self>, 
        _cx: &mut Context
    ) -> Poll<tokio::io::Result<()>>{
        Poll::Ready(Ok(()))
    }

}