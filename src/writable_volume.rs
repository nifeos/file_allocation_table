//! **A volume open for writing**: a [`Volume`] and the [`Clock`] its writes stamp with.
//!
//! The clock is required: a read-only [`Volume`] has no write methods, and the only way to the
//! write methods is a `WritableVolume`, which cannot exist without a clock. A caller with no time
//! source passes [`NoClock`](crate::NoClock) and says so. Every read (lookup, listing, file reads, the
//! label, the clean bit) goes through [`WritableVolume::as_volume`] or
//! [`WritableVolume::as_volume_mut`], which lend the `Volume` itself. A read stamps nothing, so no read takes the clock and no access date changes on a
//! read.
//!
//! The clock is read once per operation, and every entry the operation writes carries that one
//! reading. A new file or directory is created, written and accessed at that time. A write or a
//! resize sets the write time, the write date and the access date. A rename keeps the entry's
//! times. Attribute and label changes stamp nothing except a label's first entry.

use crate::attributes::Attributes;
use crate::boot_sector::OpenOptions;
use crate::file::OpenError;
use crate::file_writer::FileWriter;
use crate::named_entry::NamedEntry;
use crate::reader::{ChainError, DirectoryCluster, Volume};
use crate::sector::WritableSectorDevice;
use crate::time::{Clock, Stamp};
use crate::write_cursor::WriteError;

/// **A volume open for writing.**
///
/// # Examples
///
/// ```
/// use file_allocation_table::{
///     DirectoryCluster, FormatPlan, FormatOptions, NoClock, SectorDevice, SectorSize, WritableSectorDevice,
///     WritableVolume,
/// };
///
/// struct Memory(Vec<u8>);
/// impl SectorDevice for Memory {
///     type Error = std::io::Error;
///     fn sector_size(&self) -> SectorSize { SectorSize::Bytes512 }
///     fn read_sector(&mut self, index: u64, buffer: &mut [u8]) -> Result<(), Self::Error> {
///         let start = index as usize * 512;
///         let sector = self.0.get(start..start + 512).ok_or(std::io::ErrorKind::UnexpectedEof)?;
///         buffer.copy_from_slice(sector);
///         Ok(())
///     }
/// }
/// impl WritableSectorDevice for Memory {
///     fn write_sector(&mut self, index: u64, buffer: &[u8]) -> Result<(), Self::Error> {
///         let start = index as usize * 512;
///         let sector = self.0.get_mut(start..start + 512).ok_or(std::io::ErrorKind::UnexpectedEof)?;
///         sector.copy_from_slice(buffer);
///         Ok(())
///     }
/// }
///
/// let sectors = 16 * 1024;
/// let format = FormatPlan::new(FormatOptions::new(SectorSize::Bytes512, sectors), &mut NoClock)?;
/// let mut device = Memory(vec![0; sectors as usize * 512]);
/// format.write_to(&mut device)?;
/// // No time source here, so the caller says so.
/// let mut volume = WritableVolume::open(device, NoClock)?;
/// volume.create_file(DirectoryCluster::ROOT, "HELLO.TXT")?;
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct WritableVolume<D, C, const BUFFER_BYTES: usize = { crate::SectorSize::MAXIMUM_BYTES }> {
    volume: Volume<D, BUFFER_BYTES>,
    clock: C,
}

impl<D: WritableSectorDevice, C: Clock> WritableVolume<D, C> {
    /// **Open the volume on `device` for writing**, stamping with `clock`.
    ///
    /// # Errors
    ///
    /// As [`Volume::open`].
    pub fn open(device: D, clock: C) -> Result<WritableVolume<D, C>, crate::ReadError<D::Error>> {
        WritableVolume::open_with(device, clock, OpenOptions::new())
    }

    /// **Open the volume on `device` for writing as `options` say**, stamping with `clock`.
    ///
    /// # Errors
    ///
    /// As [`Volume::open_with`].
    pub fn open_with(
        device: D,
        clock: C,
        options: OpenOptions,
    ) -> Result<WritableVolume<D, C>, crate::ReadError<D::Error>> {
        WritableVolume::open_with_buffer(device, clock, options)
    }
}

impl<D: WritableSectorDevice, C: Clock, const BUFFER_BYTES: usize>
    WritableVolume<D, C, BUFFER_BYTES>
{
    /// **Open the volume on `device` for writing as `options` say, with a buffer of
    /// `BUFFER_BYTES`**, stamping with `clock`; see [`Volume::open_with_buffer`].
    ///
    /// # Errors
    ///
    /// As [`Volume::open_with_buffer`].
    pub fn open_with_buffer(
        device: D,
        clock: C,
        options: OpenOptions,
    ) -> Result<WritableVolume<D, C, BUFFER_BYTES>, crate::ReadError<D::Error>> {
        Ok(WritableVolume {
            volume: Volume::open_with_buffer(device, options)?,
            clock,
        })
    }

    /// Give the device back.
    pub fn into_device(self) -> D {
        self.volume.into_device()
    }

    /// Give the volume back without its clock, for reading.
    pub fn into_volume(self) -> Volume<D, BUFFER_BYTES> {
        self.volume
    }

    /// **Lend the volume for reading.** The volume's own methods take `&mut self` where they read
    /// the device; [`Self::as_volume_mut`] lends it for those.
    pub const fn as_volume(&self) -> &Volume<D, BUFFER_BYTES> {
        &self.volume
    }

    /// **Lend the volume for reading through the device**: a lookup, a listing or a file reader.
    /// A `Volume` has no write method, so nothing done through it bypasses the clock.
    pub const fn as_volume_mut(&mut self) -> &mut Volume<D, BUFFER_BYTES> {
        &mut self.volume
    }

    /// Read the clock once for the operation about to run.
    ///
    /// A time FAT cannot store is refused here, ahead of every byte the operation would write.
    fn tick(&mut self) -> Result<(), WriteError<D::Error>> {
        self.volume.stamp =
            Stamp::pack(self.clock.now()).map_err(|_| WriteError::TimeOutOfRange)?;
        Ok(())
    }

    /// **Write the file system information hints and set the clean-shutdown bit again.**
    ///
    /// # Errors
    ///
    /// [`ChainError::Device`](crate::ChainError::Device) when the device refuses a read or a write.
    pub fn mark_clean(&mut self) -> Result<(), ChainError<D::Error>> {
        self.volume.mark_clean()
    }

    /// **Create an empty file called `name`** in `directory`, created, written and accessed now.
    ///
    /// A name may not end in a dot or a space. Windows and Linux strip those, and this crate
    /// refuses with [`WriteError::NameRefused`] rather than create a name other than the one asked
    /// for.
    ///
    /// # Errors
    ///
    /// [`WriteError::TimeOutOfRange`](crate::WriteError::TimeOutOfRange) when the clock reports a
    /// time FAT cannot store, before anything is written,
    /// [`WriteError::NameRefused`](crate::WriteError::NameRefused) for a name that is neither a
    /// legal short name nor a long name,
    /// [`WriteError::AlreadyExists`](crate::WriteError::AlreadyExists) when the directory holds an
    /// entry of that name, [`WriteError::DirectoryFull`](crate::WriteError::DirectoryFull) when the
    /// directory has no free run of slots and cannot grow,
    /// [`WriteError::VolumeFull`](crate::WriteError::VolumeFull) when it would grow and no cluster
    /// is free, and [`WriteError::Chain`](crate::WriteError::Chain) when the device refuses a read
    /// or a write or a chain the change follows is damaged.
    ///
    /// # Examples
    ///
    /// ```
    /// use file_allocation_table::{
    ///     DirectoryCluster, FormatOptions, FormatPlan, NoClock, SectorDevice, SectorSize,
    ///     WritableSectorDevice, WritableVolume, WriteError,
    /// };
    ///
    /// # struct Memory(Vec<u8>);
    /// # impl SectorDevice for Memory {
    /// #     type Error = std::io::Error;
    /// #     fn sector_size(&self) -> SectorSize { SectorSize::Bytes512 }
    /// #     fn read_sector(&mut self, index: u64, buffer: &mut [u8]) -> Result<(), Self::Error> {
    /// #         let start = index as usize * 512;
    /// #         let sector = self.0.get(start..start + 512).ok_or(std::io::ErrorKind::UnexpectedEof)?;
    /// #         buffer.copy_from_slice(sector);
    /// #         Ok(())
    /// #     }
    /// # }
    /// # impl WritableSectorDevice for Memory {
    /// #     fn write_sector(&mut self, index: u64, buffer: &[u8]) -> Result<(), Self::Error> {
    /// #         let start = index as usize * 512;
    /// #         let sector = self.0.get_mut(start..start + 512).ok_or(std::io::ErrorKind::UnexpectedEof)?;
    /// #         sector.copy_from_slice(buffer);
    /// #         Ok(())
    /// #     }
    /// # }
    /// // `Memory` is an in-memory device, as in the example on `WritableVolume`.
    /// let sectors = 16 * 1024;
    /// let mut device = Memory(vec![0; sectors as usize * 512]);
    /// FormatPlan::new(FormatOptions::new(SectorSize::Bytes512, sectors), &mut NoClock)?
    ///     .write_to(&mut device)?;
    /// let mut volume = WritableVolume::open(device, NoClock)?;
    ///
    /// let notes = volume.create_file(DirectoryCluster::ROOT, "Meeting notes.txt")?;
    /// assert_eq!(notes.size_bytes(), 0);
    /// let found = volume.as_volume_mut().lookup("MEETING NOTES.TXT")?;
    /// assert_eq!(found.first_cluster(), notes.first_cluster());
    ///
    /// // The name is taken now, and a trailing dot is never stored.
    /// assert!(matches!(
    ///     volume.create_file(DirectoryCluster::ROOT, "meeting notes.txt"),
    ///     Err(WriteError::AlreadyExists)
    /// ));
    /// assert!(matches!(
    ///     volume.create_file(DirectoryCluster::ROOT, "draft."),
    ///     Err(WriteError::NameRefused)
    /// ));
    /// # Ok::<(), Box<dyn core::error::Error>>(())
    /// ```
    pub fn create_file(
        &mut self,
        directory: DirectoryCluster,
        name: &str,
    ) -> Result<NamedEntry, WriteError<D::Error>> {
        self.tick()?;
        self.volume.create_file(directory.0, name)
    }

    /// **Create an empty directory called `name`** in `directory`, created, written and accessed
    /// now, with the same times on its `.` and `..`.
    ///
    /// A name may not end in a dot or a space. Windows and Linux strip those, and this crate
    /// refuses with [`WriteError::NameRefused`] rather than create a name other than the one asked
    /// for.
    ///
    /// # Errors
    ///
    /// [`WriteError::TimeOutOfRange`](crate::WriteError::TimeOutOfRange) when the clock reports a
    /// time FAT cannot store, before anything is written,
    /// [`WriteError::NameRefused`](crate::WriteError::NameRefused) for a name that is neither a
    /// legal short name nor a long name,
    /// [`WriteError::AlreadyExists`](crate::WriteError::AlreadyExists) when the directory holds an
    /// entry of that name, [`WriteError::DirectoryFull`](crate::WriteError::DirectoryFull) when the
    /// directory has no free run of slots and cannot grow,
    /// [`WriteError::VolumeFull`](crate::WriteError::VolumeFull) when it would grow and no cluster
    /// is free (the new directory's own cluster included), and
    /// [`WriteError::Chain`](crate::WriteError::Chain) when the device refuses a read or a write or
    /// a chain the change follows is damaged.
    pub fn create_directory(
        &mut self,
        directory: DirectoryCluster,
        name: &str,
    ) -> Result<NamedEntry, WriteError<D::Error>> {
        self.tick()?;
        self.volume.create_directory(directory.0, name)
    }

    /// **Remove the empty directory that `entry` names.**
    ///
    /// # Errors
    ///
    /// [`WriteError::NotADirectory`](crate::WriteError::NotADirectory) for a file, `.` and `..`,
    /// [`WriteError::DirectoryNotEmpty`](crate::WriteError::DirectoryNotEmpty) when the directory
    /// holds other entries, [`WriteError::EntryChanged`](crate::WriteError::EntryChanged) when the
    /// entry's slot no longer holds `entry`; look it up again, and
    /// [`WriteError::Chain`](crate::WriteError::Chain) when the device refuses a read or a write or
    /// a chain the change follows is damaged.
    pub fn remove_directory(&mut self, entry: &NamedEntry) -> Result<(), WriteError<D::Error>> {
        self.volume.remove_directory(entry)
    }

    /// **Rename `entry`, and move it to `directory`**; the entry keeps its times.
    ///
    /// The new entry is written first, then a moved directory's `..`, then the old slots are
    /// freed. A crash before the last step leaves the file under both names, on one chain, which
    /// `fsck.fat` repairs as shared clusters; it never leaves the file under neither.
    ///
    /// # Errors
    ///
    /// [`WriteError::NotAFile`](crate::WriteError::NotAFile) for `.`, `..` and the volume label,
    /// [`WriteError::MovedIntoItself`](crate::WriteError::MovedIntoItself) when a directory would
    /// move into itself or below itself,
    /// [`WriteError::NameRefused`](crate::WriteError::NameRefused) for a name that is neither a
    /// legal short name nor a long name,
    /// [`WriteError::AlreadyExists`](crate::WriteError::AlreadyExists) when the directory holds an
    /// entry of that name, [`WriteError::DirectoryFull`](crate::WriteError::DirectoryFull) when the
    /// directory has no free run of slots and cannot grow,
    /// [`WriteError::VolumeFull`](crate::WriteError::VolumeFull) when it would grow and no cluster
    /// is free, [`WriteError::EntryChanged`](crate::WriteError::EntryChanged) when the entry's slot
    /// no longer holds `entry`; look it up again, and
    /// [`WriteError::Chain`](crate::WriteError::Chain) when the device refuses a read or a write or
    /// a chain the change follows is damaged.
    pub fn rename(
        &mut self,
        entry: &NamedEntry,
        directory: DirectoryCluster,
        new_name: &str,
    ) -> Result<NamedEntry, WriteError<D::Error>> {
        self.volume.rename(entry, directory.0, new_name)
    }

    /// **Delete the file that `entry` names.**
    ///
    /// # Errors
    ///
    /// [`WriteError::NotAFile`](crate::WriteError::NotAFile) for a directory and the volume label,
    /// [`WriteError::ReadOnly`](crate::WriteError::ReadOnly) for a read-only file,
    /// [`WriteError::EntryChanged`](crate::WriteError::EntryChanged) when the entry's slot no
    /// longer holds `entry`; look it up again, and [`WriteError::Chain`](crate::WriteError::Chain)
    /// when the device refuses a read or a write or a chain the change follows is damaged.
    pub fn remove_file(&mut self, entry: &NamedEntry) -> Result<(), WriteError<D::Error>> {
        self.volume.remove_file(entry)
    }

    /// **Set `entry`'s read-only, hidden, system and archive bits to exactly `attributes`.** The
    /// entry's times do not change.
    ///
    /// # Errors
    ///
    /// [`WriteError::NotAFile`](crate::WriteError::NotAFile) for `.`, `..` and the volume label,
    /// [`WriteError::EntryChanged`](crate::WriteError::EntryChanged) when the entry's slot no
    /// longer holds `entry`; look it up again, and [`WriteError::Chain`](crate::WriteError::Chain)
    /// when the device refuses a read or a write or a chain the change follows is damaged.
    pub fn set_attributes(
        &mut self,
        entry: &NamedEntry,
        attributes: Attributes,
    ) -> Result<NamedEntry, WriteError<D::Error>> {
        self.volume.set_attributes(entry, attributes)
    }

    /// **Set the volume label.**
    ///
    /// # Errors
    ///
    /// [`WriteError::TimeOutOfRange`](crate::WriteError::TimeOutOfRange) when the clock reports a
    /// time FAT cannot store, before anything is written,
    /// [`WriteError::LabelRefused`](crate::WriteError::LabelRefused) when `label` is empty, over
    /// eleven bytes, or holds a byte a short name refuses,
    /// [`WriteError::DirectoryFull`](crate::WriteError::DirectoryFull) when the root has no free
    /// slot for a new label entry and cannot grow,
    /// [`WriteError::VolumeFull`](crate::WriteError::VolumeFull) when it would grow and no cluster
    /// is free, and [`WriteError::Chain`](crate::WriteError::Chain) when the device refuses a read
    /// or a write or a chain the change follows is damaged.
    pub fn set_volume_label(&mut self, label: &str) -> Result<(), WriteError<D::Error>> {
        self.tick()?;
        self.volume.set_volume_label(label)
    }

    /// **Open the file `entry` names for writing**, at its start. The writer reads this volume's
    /// clock on each write. Refuses a directory and the volume label, saying which, and an entry
    /// its directory slot no longer holds ([`OpenError::EntryChanged`]), which it reads to check,
    /// as [`Volume::open_file_reader`] does. The entry [`Self::create_file`] returns says size 0
    /// and no cluster, so after writing through it, call [`lookup`](crate::Volume::lookup) again
    /// before opening another reader or writer.
    ///
    /// # Errors
    ///
    /// [`OpenError::IsADirectory`](crate::OpenError::IsADirectory) for a directory,
    /// [`OpenError::IsAVolumeLabel`](crate::OpenError::IsAVolumeLabel) for the volume label,
    /// [`OpenError::EntryChanged`](crate::OpenError::EntryChanged) when the entry's slot no longer
    /// holds `entry`, and [`OpenError::Chain`](crate::OpenError::Chain) when that slot cannot be
    /// read, as [`Volume::open_file_reader`] returns them.
    pub fn open_file_writer(
        &mut self,
        entry: &NamedEntry,
    ) -> Result<FileWriter<'_, D, C, BUFFER_BYTES>, OpenError<D::Error>> {
        FileWriter::new(&mut self.volume, &mut self.clock, entry)
    }
}

#[cfg(test)]
mod tests;
