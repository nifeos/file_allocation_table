//! **A file's bytes**, read through its cluster chain and bounded by the size its directory entry
//! states and by the volume's cluster count.
//!
//! [`FileReader`] reads and seeks. Under it, a cursor turns a position into the sector that holds
//! it and the bytes of that sector the next read may use. That arithmetic is the part
//! `verification.rs` proves: for any size and any table contents, no span leaves the file or the
//! data clusters, and the spans run out. A chain shorter than the size is an error, a chain longer
//! than the size is read only as far as the size, and a chain that loops ends at the volume's
//! cluster count with `TooLong`.

use crate::boot_sector::BootSector;
use crate::named_entry::NamedEntry;
use crate::reader::{ChainError, Links, Volume, Walk};
use crate::sector::{SectorDevice, SectorSize};

/// **Why an entry could not be opened as a file**, for reading
/// ([`Volume::open_file_reader`]) or for writing
/// ([`WritableVolume::open_file_writer`](crate::WritableVolume::open_file_writer)). Each variant
/// means the same for both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OpenError<E> {
    /// The entry is a directory, whose slots [`Volume::named_entries`] reads.
    IsADirectory,
    /// The entry is the volume label, which names no file and owns no cluster.
    IsAVolumeLabel,
    /// The entry's directory slot no longer holds the entry the caller passed: a write, a resize,
    /// a rename, a removal or an attribute change since it was read changed the slot. Look the
    /// entry up again.
    EntryChanged,
    /// The entry's directory slot could not be read.
    Chain(ChainError<E>),
}

impl<E> From<ChainError<E>> for OpenError<E> {
    fn from(error: ChainError<E>) -> OpenError<E> {
        OpenError::Chain(error)
    }
}

impl<E> OpenError<E> {
    /// Why `entry` cannot be opened as a file on `volume`, or `Ok` when it can: it is a file, and
    /// its slot still holds it.
    pub(crate) fn check<D: SectorDevice<Error = E>>(
        volume: &mut Volume<D>,
        entry: &NamedEntry,
    ) -> Result<(), OpenError<E>> {
        if entry.is_directory() {
            Err(OpenError::IsADirectory)
        } else if entry.is_volume_label() {
            Err(OpenError::IsAVolumeLabel)
        } else if volume.slot_holds(entry.place(), entry.short_entry())? {
            Ok(())
        } else {
            Err(OpenError::EntryChanged)
        }
    }
}

/// **Why a file read stopped.**
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FileReadError<E> {
    /// The chain could not be followed, or a sector could not be read.
    Chain(ChainError<E>),
    /// The chain ends before the file's size is covered: the entry promises more bytes than the
    /// volume has clusters for.
    ShorterThanItsSize,
    /// An earlier read failed, so this reader returns nothing more rather than skip bytes.
    Failed,
    /// A seek named a position before the start of the file. Only the `embedded-io-07` feature's
    /// `Seek` implementation returns it, and the reader stays where it was and usable.
    SeekBeforeStart,
}

/// **A run of bytes inside one sector** that holds the next bytes of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Span {
    /// The volume-relative sector.
    pub(crate) sector: u32,
    /// The first byte of the span in that sector.
    pub(crate) offset: usize,
    /// How many bytes, at least 1 and not past the sector's end.
    pub(crate) length_bytes: usize,
}

/// **Where a file read stands**, apart from the volume, so the arithmetic can be proved over a
/// hostile table without a device.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FileCursor {
    first_cluster: u32,
    size_bytes: u32,
    position: u32,
    walk: Walk,
    /// The cluster at index `clusters_walked - 1` of the chain.
    current: Option<u32>,
    clusters_walked: u32,
    failed: bool,
}

impl FileCursor {
    /// A cursor at the start of a file of `size` bytes whose chain starts at `first_cluster`.
    pub(crate) const fn new(first_cluster: u32, size: u32) -> FileCursor {
        FileCursor {
            first_cluster,
            size_bytes: size,
            position: 0,
            walk: Walk::new(first_cluster),
            current: None,
            clusters_walked: 0,
            failed: false,
        }
    }

    /// The position, in bytes from the start.
    pub(crate) const fn position(&self) -> u32 {
        self.position
    }

    /// Move to `position`, or to the end of the file when it is past it. A move backwards past the
    /// cluster at hand starts the chain walk again.
    pub(crate) fn seek(&mut self, boot: &BootSector, position: u32) {
        let position = position.min(self.size_bytes);
        let needed = position / boot.bytes_per_cluster();
        if self.clusters_walked > needed + 1 {
            self.walk = Walk::new(self.first_cluster);
            self.current = None;
            self.clusters_walked = 0;
        }
        self.position = position;
    }

    /// **The next span of at most `want` bytes**, `Ok(None)` at the end of the file or when `want`
    /// is 0, and an error when the chain cannot supply the bytes the size promises. After an error
    /// every call returns `Failed`.
    pub(crate) fn next_span<L: Links>(
        &mut self,
        links: &mut L,
        boot: &BootSector,
        want: usize,
    ) -> Result<Option<Span>, FileReadError<L::Error>> {
        if self.failed {
            return Err(FileReadError::Failed);
        }
        match self.span(links, boot, want) {
            Ok(span) => Ok(span),
            Err(error) => {
                self.failed = true;
                Err(error)
            }
        }
    }

    fn span<L: Links>(
        &mut self,
        links: &mut L,
        boot: &BootSector,
        want: usize,
    ) -> Result<Option<Span>, FileReadError<L::Error>> {
        if self.position >= self.size_bytes || want == 0 {
            return Ok(None);
        }
        let per_cluster = boot.bytes_per_cluster();
        let needed = self.position / per_cluster;
        while self.clusters_walked <= needed {
            match self.walk.step(links) {
                Some(Ok(cluster)) => {
                    self.current = Some(cluster);
                    self.clusters_walked += 1;
                }
                Some(Err(error)) => return Err(FileReadError::Chain(error)),
                None => return Err(FileReadError::ShorterThanItsSize),
            }
        }
        let Some(cluster) = self.current else {
            return Err(FileReadError::ShorterThanItsSize);
        };
        let sector_bytes = boot.sector_size().bytes() as u32;
        let within = self.position % per_cluster;
        let wrong_cluster = FileReadError::Chain(ChainError::NotADataCluster(cluster));
        let sector = boot
            .cluster_first_sector(cluster)
            .and_then(|first| first.checked_add(within / sector_bytes))
            .ok_or(wrong_cluster)?;
        let offset = within % sector_bytes;
        let len = (sector_bytes - offset)
            .min(self.size_bytes - self.position)
            .min(u32::try_from(want).unwrap_or(u32::MAX));
        self.position += len;
        Ok(Some(Span {
            sector,
            offset: offset as usize,
            length_bytes: len as usize,
        }))
    }
}

/// **A file open for reading**: what [`Volume::open_file_reader`] returns.
///
/// It reads forward from a position and seeks to any position in the file. A read costs one
/// device read per sector it touches, plus the table reads the chain walk needs. A seek backwards
/// walks the chain again from its start, bounded by the volume's cluster count.
#[derive(Debug)]
pub struct FileReader<'v, D> {
    volume: &'v mut Volume<D>,
    cursor: FileCursor,
    buffer: [u8; SectorSize::MAXIMUM_BYTES],
}

impl<'v, D: SectorDevice> FileReader<'v, D> {
    pub(crate) fn new(
        volume: &'v mut Volume<D>,
        first_cluster: u32,
        size: u32,
    ) -> FileReader<'v, D> {
        FileReader {
            volume,
            cursor: FileCursor::new(first_cluster, size),
            buffer: [0; SectorSize::MAXIMUM_BYTES],
        }
    }

    /// The file's length in bytes, as its directory entry states.
    pub const fn size_bytes(&self) -> u32 {
        self.cursor.size_bytes
    }

    /// The position of the next byte read.
    pub const fn position(&self) -> u32 {
        self.cursor.position()
    }

    /// **Move to byte `position`**, or to the end of the file when `position` is past it.
    pub fn set_position(&mut self, position: u32) {
        let boot = *self.volume.boot_sector();
        self.cursor.seek(&boot, position);
    }

    /// **Read up to `buffer.len()` bytes from the current position**, and return how many. It
    /// reads fewer only at the end of the file, so a return of 0 for a nonempty buffer is the end.
    ///
    /// Never returns bytes past the file's size. An error ends the reader, and bytes copied earlier
    /// in the same call are not reported: later calls return [`FileReadError::Failed`].
    ///
    /// This is not `std::io::Read::read`: the crate is `no_std` and implements no `std::io` trait,
    /// and the error type is [`FileReadError`]. The `embedded-io-07` feature implements
    /// `embedded_io::Read` and `embedded_io::Seek` through this method and
    /// [`FileReader::set_position`].
    pub fn read(&mut self, buffer: &mut [u8]) -> Result<usize, FileReadError<D::Error>> {
        let boot = *self.volume.boot_sector();
        let sector_bytes = boot.sector_size().bytes();
        let mut filled = 0;
        // Every span holds at least one byte, so `buffer.len()` passes are enough, and a request
        // for zero bytes (the buffer is full) ends the loop. The bound keeps a cursor that
        // returned an empty span from spinning here.
        for _ in 0..buffer.len() {
            let Some(span) = self
                .cursor
                .next_span(self.volume, &boot, buffer.len() - filled)?
            else {
                break;
            };
            let read = if span.offset == 0 && span.length_bytes == sector_bytes {
                // A whole sector goes straight into the caller's buffer.
                self.volume
                    .read_sector(span.sector, &mut buffer[filled..filled + sector_bytes])
            } else {
                self.volume
                    .read_sector(span.sector, &mut self.buffer[..sector_bytes])
            };
            if let Err(error) = read {
                self.cursor.failed = true;
                return Err(FileReadError::Chain(error));
            }
            if !(span.offset == 0 && span.length_bytes == sector_bytes) {
                buffer[filled..filled + span.length_bytes]
                    .copy_from_slice(&self.buffer[span.offset..span.offset + span.length_bytes]);
            }
            filled += span.length_bytes;
        }
        Ok(filled)
    }
}

impl<D: SectorDevice> Volume<D> {
    /// **Open the file `entry` names for reading.**
    ///
    /// Refuses a directory, the volume label, and an entry its directory slot no longer holds
    /// ([`OpenError::EntryChanged`]), which it reads to check. The entry's first cluster and size
    /// came off the volume: a first cluster that is not a data cluster, and a size the chain cannot
    /// cover, surface as errors on the first read that needs them.
    pub fn open_file_reader(
        &mut self,
        entry: &NamedEntry,
    ) -> Result<FileReader<'_, D>, OpenError<D::Error>> {
        OpenError::check(self, entry)?;
        Ok(FileReader::new(
            self,
            entry.first_cluster(),
            entry.size_bytes(),
        ))
    }
}

#[cfg(kani)]
mod verification;

#[cfg(test)]
mod tests;
