//! **The `embedded-io` 0.7 traits**, behind the `embedded-io-07` feature: [`FileReader`] is
//! `Read` and `Seek`, [`FileWriter`] is `Write` and `Seek`, and [`FileReadError`] and
//! [`WriteError`] are `embedded_io::Error`, each with the [`ErrorKind`] nearest its meaning.
//!
//! Each trait method calls the inherent method of the same purpose, so a read, a write or a seek
//! through a trait does exactly what the inherent call does. A seek past the end of a file stops at
//! its end, as [`FileReader::set_position`] and [`FileWriter::set_position`] do, and returns that
//! position; a seek before the start is refused with `SeekBeforeStart` and moves nothing.
//! `Write::flush` has nothing to do, since every write rewrites the directory entry before it
//! returns.
//!
//! `embedded_io::Error` requires `core::error::Error`, which this crate's error types implement
//! when the device's error does, so the traits need a device whose error implements
//! `core::error::Error`. A device's own error reports [`ErrorKind::Other`], since the crate cannot
//! know what it means.

use embedded_io::{ErrorKind, ErrorType, Read, Seek, SeekFrom, Write};

use crate::reader::ChainError;
use crate::sector::{SectorDevice, WritableSectorDevice};
use crate::time::Clock;
use crate::{FileReadError, FileReader, FileWriter, WriteError};

/// The kind of a chain error: the device's own error is opaque, and every other variant says the
/// table on the volume is damaged.
fn chain_kind<E>(error: &ChainError<E>) -> ErrorKind {
    match error {
        ChainError::Device(_) => ErrorKind::Other,
        ChainError::NotADataCluster(_) | ChainError::Broken { .. } | ChainError::TooLong => {
            ErrorKind::InvalidData
        }
    }
}

impl<E: core::error::Error + 'static> embedded_io::Error for FileReadError<E> {
    fn kind(&self) -> ErrorKind {
        match self {
            FileReadError::Chain(error) => chain_kind(error),
            FileReadError::ShorterThanItsSize => ErrorKind::InvalidData,
            FileReadError::Failed => ErrorKind::Other,
            FileReadError::SeekBeforeStart => ErrorKind::InvalidInput,
        }
    }
}

impl<E: core::error::Error + 'static> embedded_io::Error for WriteError<E> {
    fn kind(&self) -> ErrorKind {
        match self {
            WriteError::Chain(error) => chain_kind(error),
            // `embedded_io::Write` asks for `WriteZero` when a writer can take no more bytes.
            WriteError::VolumeFull | WriteError::FileTooLarge => ErrorKind::WriteZero,
            WriteError::ShorterThanItsSize | WriteError::TimeOutOfRange => ErrorKind::InvalidData,
            WriteError::EntryChanged => ErrorKind::NotFound,
            WriteError::AlreadyExists => ErrorKind::AlreadyExists,
            WriteError::ReadOnly => ErrorKind::PermissionDenied,
            WriteError::NameRefused
            | WriteError::LabelRefused
            | WriteError::NotAFile
            | WriteError::NotADirectory
            | WriteError::MovedIntoItself
            | WriteError::SeekBeforeStart => ErrorKind::InvalidInput,
            WriteError::Failed | WriteError::DirectoryFull | WriteError::DirectoryNotEmpty => {
                ErrorKind::Other
            }
        }
    }
}

/// **The byte a seek names**, or `None` when it lies before the start of the file. A position
/// past `u32::MAX` becomes `u32::MAX`, which the cursor then brings back to the file's end.
fn seek_target(from: SeekFrom, position: u32, size_bytes: u32) -> Option<u32> {
    let (base, offset) = match from {
        SeekFrom::Start(target) => return Some(u32::try_from(target).unwrap_or(u32::MAX)),
        SeekFrom::End(offset) => (size_bytes, offset),
        SeekFrom::Current(offset) => (position, offset),
    };
    let target = i128::from(base) + i128::from(offset);
    if target < 0 {
        None
    } else {
        Some(u32::try_from(target).unwrap_or(u32::MAX))
    }
}

impl<D: SectorDevice> ErrorType for FileReader<'_, D>
where
    D::Error: core::error::Error + 'static,
{
    type Error = FileReadError<D::Error>;
}

impl<D: SectorDevice> Read for FileReader<'_, D>
where
    D::Error: core::error::Error + 'static,
{
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize, Self::Error> {
        FileReader::read(self, buffer)
    }
}

impl<D: SectorDevice> Seek for FileReader<'_, D>
where
    D::Error: core::error::Error + 'static,
{
    fn seek(&mut self, from: SeekFrom) -> Result<u64, Self::Error> {
        let target = seek_target(from, self.position(), self.size_bytes())
            .ok_or(FileReadError::SeekBeforeStart)?;
        self.set_position(target);
        Ok(u64::from(self.position()))
    }
}

impl<D: WritableSectorDevice, C: Clock> ErrorType for FileWriter<'_, D, C>
where
    D::Error: core::error::Error + 'static,
{
    type Error = WriteError<D::Error>;
}

impl<D: WritableSectorDevice, C: Clock> Write for FileWriter<'_, D, C>
where
    D::Error: core::error::Error + 'static,
{
    fn write(&mut self, data: &[u8]) -> Result<usize, Self::Error> {
        FileWriter::write(self, data)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl<D: WritableSectorDevice, C: Clock> Seek for FileWriter<'_, D, C>
where
    D::Error: core::error::Error + 'static,
{
    fn seek(&mut self, from: SeekFrom) -> Result<u64, Self::Error> {
        let target = seek_target(from, self.position(), self.size_bytes())
            .ok_or(WriteError::SeekBeforeStart)?;
        self.set_position(target);
        Ok(u64::from(self.position()))
    }
}

#[cfg(test)]
mod tests;
