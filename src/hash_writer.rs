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
