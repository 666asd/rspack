use std::fmt::Debug;

use rspack_paths::Utf8Path;

use super::Result;
use crate::{Error, WritableFileSystem};

pub trait IntermediateFileSystem:
  WritableFileSystem + IntermediateFileSystemExtras + Debug + Send + Sync
{
}

#[async_trait::async_trait]
pub trait IntermediateFileSystemExtras: Debug + Send + Sync {
  async fn rename(&self, from: &Utf8Path, to: &Utf8Path) -> Result<()>;
  async fn create_read_stream(&self, file: &Utf8Path) -> Result<Box<dyn ReadStream>>;
  async fn create_write_stream(&self, file: &Utf8Path) -> Result<Box<dyn WriteStream>>;
}

#[async_trait::async_trait]
pub trait ReadStream: Debug + Sync + Send {
  async fn read_line(&mut self) -> Result<String> {
    let data = self.read_until(b'\n').await?;
    if simdutf8::basic::from_utf8(&data).is_ok() {
      #[allow(unsafe_code)]
      // SAFETY: simdutf8 validated the buffer as UTF-8 above.
      Ok(unsafe { String::from_utf8_unchecked(data) })
    } else {
      Err(Error::Io(std::io::Error::other("invalid utf8 line")))
    }
  }
  async fn read(&mut self, length: usize) -> Result<Vec<u8>>;
  async fn read_until(&mut self, byte: u8) -> Result<Vec<u8>>;
  async fn read_to_end(&mut self) -> Result<Vec<u8>>;
  async fn skip(&mut self, offset: usize) -> Result<()>;
  async fn close(&mut self) -> Result<()>;
}

#[async_trait::async_trait]
pub trait WriteStream: Debug + Sync + Send {
  async fn write_line(&mut self, line: &str) -> Result<()> {
    self.write(line.as_bytes()).await?;
    self.write(b"\n").await?;
    Ok(())
  }
  async fn write(&mut self, buf: &[u8]) -> Result<usize>;
  async fn write_all(&mut self, buf: &[u8]) -> Result<()>;
  async fn flush(&mut self) -> Result<()>;
  async fn close(&mut self) -> Result<()>;
}
