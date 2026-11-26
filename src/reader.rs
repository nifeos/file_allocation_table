//! **An open volume**: a device, its checked boot sector, and the reads every later structure
//! makes through them. It reads the table entry by entry and walks cluster chains with a bound
//! no table contents can defeat.

use crate::allocation::WriterState;
use crate::boot_sector::{BootSector, OpenOptions, ReadError};
use crate::directory::Directory;
use crate::named_entry::NamedEntries;
use crate::sector::{SectorDevice, SectorSize};
use crate::table::{self, TableEntry};
use crate::time::Stamp;

/// **Why a table read or a chain walk stopped.**
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ChainError<E> {
    /// The device refused a read; its own error, unchanged.
    Device(E),
    /// The cluster asked for, or a chain's first cluster, is not one of this volume's data
    /// clusters (2 to `clusters() + 1`).
    NotADataCluster(Cluster),
    /// `cluster`'s table entry cannot continue a chain: it is free, reserved or bad, or names a
    /// cluster the volume does not have.
    Broken {
        /// The cluster whose entry was read.
        cluster: Cluster,
        /// What the entry said.
        entry: TableEntry,
    },
    /// The chain has more clusters than the volume holds, so it revisits one: the table loops.
    TooLong,
}

/// **A FAT volume open for reading**, over a device the caller supplies.
///
/// It owns the device (pass `&mut device` to keep it) and caches the last table sector it read,
/// so a chain walk through one sector of the table costs one device read.
///
/// # Examples
///
/// ```
/// use file_allocation_table::efi_system_partition::FILE_CLUSTER;
/// use file_allocation_table::{
///     Cluster, NoClock, SectorDevice, SectorSize, Volume, VolumeLayout, VolumeLayoutOptions,
/// };
///
/// let options = VolumeLayoutOptions::new(70_000 * 8 + 2_000)
///     .with_volume_id(1)
///     .with_label("EFI SYSTEM")
///     .and_then(|options| options.with_file("BOOTX64.EFI", 5_000))?;
/// let layout = VolumeLayout::new(options, &mut NoClock)?;
/// struct Image(VolumeLayout);
/// impl SectorDevice for Image {
///     type Error = core::convert::Infallible;
///     fn sector_size(&self) -> SectorSize {
///         SectorSize::Bytes512
///     }
///     fn read_sector(&mut self, index: u64, buffer: &mut [u8]) -> Result<(), Self::Error> {
///         if self.0.fill_sector(index, buffer).is_err() {
///             buffer.fill(0);
///         }
///         Ok(())
///     }
/// }
///
/// let mut volume = Volume::open(Image(layout))?;
/// // The layout puts the boot file at FILE_CLUSTER (5, after the root, EFI and BOOT), and its
/// // 5,000 bytes take two clusters of 4,096 bytes.
/// let first = Cluster::new(FILE_CLUSTER);
/// let clusters = volume.chain(first).collect::<Result<Vec<Cluster>, _>>()?;
/// assert_eq!(clusters, [first, Cluster::new(FILE_CLUSTER + 1)]);
/// # Ok::<(), Box<dyn core::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct Volume<D, const BUFFER_BYTES: usize = { SectorSize::MAXIMUM_BYTES }> {
    pub(crate) device: D,
    pub(crate) boot: BootSector,
    pub(crate) cached: Option<u32>,
    pub(crate) buffer: [u8; BUFFER_BYTES],
    /// What the writer keeps between calls; a reader never touches it.
    pub(crate) writer: WriterState,
    /// The time the operation under way stamps: packed from the clock once per operation.
    pub(crate) stamp: Stamp,
    /// Where the last slot lookup in a cluster chain ended, so the next one in the same chain
    /// starts there rather than at the chain's first cluster. A table write forgets it.
    pub(crate) slot_cursor: Option<SlotCursor>,
}

/// **A place in a directory's chain** that a slot lookup reached: the directory's first cluster,
/// and the cluster at position `position` of its chain. It stays true until the table changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SlotCursor {
    pub(crate) first: u32,
    pub(crate) position: u64,
    pub(crate) cluster: u32,
}

impl<D: SectorDevice> Volume<D> {
    /// **Open the volume on `device`**: read and check its boot sector.
    ///
    /// Refuses what [`BootSector::read`] refuses, including a volume whose sector size is not the
    /// device's.
    ///
    /// # Errors
    ///
    /// [`ReadError::Device`](crate::ReadError::Device),
    /// [`ReadError::Refused`](crate::ReadError::Refused) and
    /// [`ReadError::SectorSizeMismatch`](crate::ReadError::SectorSizeMismatch), as
    /// [`BootSector::read`] returns them.
    ///
    /// # Examples
    ///
    /// ```
    /// use file_allocation_table::{
    ///     NoClock, SectorDevice, SectorSize, Volume, VolumeLayout, VolumeLayoutOptions,
    /// };
    ///
    /// /// The sectors of an empty FAT32 volume, as the layout fills them; the rest read as zeros.
    /// struct Image(VolumeLayout);
    /// impl SectorDevice for Image {
    ///     type Error = core::convert::Infallible;
    ///     fn sector_size(&self) -> SectorSize {
    ///         SectorSize::Bytes512
    ///     }
    ///     fn read_sector(&mut self, index: u64, buffer: &mut [u8]) -> Result<(), Self::Error> {
    ///         if self.0.fill_sector(index, buffer).is_err() {
    ///             buffer.fill(0);
    ///         }
    ///         Ok(())
    ///     }
    /// }
    ///
    /// let options = VolumeLayoutOptions::new(70_000 * 8 + 2_000);
    /// let mut volume = Volume::open(Image(VolumeLayout::new(options, &mut NoClock)?))?;
    /// assert_eq!(volume.boot_sector().sector_size(), SectorSize::Bytes512);
    /// assert!(volume.root_named_entries().next().is_none());
    /// # Ok::<(), Box<dyn core::error::Error>>(())
    /// ```
    pub fn open(device: D) -> Result<Volume<D>, ReadError<D::Error>> {
        Volume::open_with(device, OpenOptions::new())
    }

    /// **Open the volume on `device` as `options` say**; [`Volume::open`] is this with
    /// [`OpenOptions::new`], the strict reading.
    ///
    /// # Errors
    ///
    /// As [`Volume::open`], with the boot sector read as `options` say.
    pub fn open_with(device: D, options: OpenOptions) -> Result<Volume<D>, ReadError<D::Error>> {
        Volume::open_with_buffer(device, options)
    }
}

impl<D: SectorDevice, const BUFFER_BYTES: usize> Volume<D, BUFFER_BYTES> {
    /// **Open the volume on `device` as `options` say, with a buffer of `BUFFER_BYTES`**: the
    /// way to open a volume whose buffers are smaller than the default of 4,096 bytes, the
    /// largest sector size. A device of 512-byte sectors needs only 512, and a `Volume`, a
    /// [`Directory`] and a [`FileReader`](crate::FileReader) each hold one buffer of this length.
    ///
    /// `BUFFER_BYTES` must be 512, 1,024, 2,048 or 4,096, or the call does not compile.
    ///
    /// # Errors
    ///
    /// [`ReadError::SectorLargerThanBuffer`](crate::ReadError::SectorLargerThanBuffer) when the
    /// device's sectors are longer than `BUFFER_BYTES`, before anything is read, and otherwise as
    /// [`Volume::open_with`].
    ///
    /// # Examples
    ///
    /// ```
    /// use file_allocation_table::{OpenOptions, ReadError, SectorDevice, SectorSize, Volume};
    ///
    /// /// A device of 4,096-byte sectors, every one of them zeros.
    /// struct Large;
    /// impl SectorDevice for Large {
    ///     type Error = core::convert::Infallible;
    ///     fn sector_size(&self) -> SectorSize {
    ///         SectorSize::Bytes4096
    ///     }
    ///     fn read_sector(&mut self, _: u64, buffer: &mut [u8]) -> Result<(), Self::Error> {
    ///         buffer.fill(0);
    ///         Ok(())
    ///     }
    /// }
    ///
    /// // The buffer is the only part that shrinks, and its sectors would not fit in it.
    /// assert_eq!(size_of::<Volume<Large, 512>>() + 3_584, size_of::<Volume<Large>>());
    /// let refused = Volume::<_, 512>::open_with_buffer(Large, OpenOptions::new());
    /// assert!(matches!(refused, Err(ReadError::SectorLargerThanBuffer { .. })));
    /// ```
    pub fn open_with_buffer(
        mut device: D,
        options: OpenOptions,
    ) -> Result<Volume<D, BUFFER_BYTES>, ReadError<D::Error>> {
        const {
            assert!(
                matches!(BUFFER_BYTES, 512 | 1024 | 2048 | 4096),
                "a volume's buffer holds 512, 1024, 2048 or 4096 bytes"
            );
        }
        let device_sector = device.sector_size();
        if device_sector.bytes() > BUFFER_BYTES {
            return Err(ReadError::SectorLargerThanBuffer {
                device: device_sector,
                buffer_bytes: BUFFER_BYTES,
            });
        }
        let boot = BootSector::read_with(&mut device, options)?;
        Ok(Volume {
            device,
            boot,
            cached: None,
            buffer: [0; BUFFER_BYTES],
            writer: WriterState::new(),
            stamp: Stamp::ZERO,
            slot_cursor: None,
        })
    }

    /// The volume's boot sector, as [`Volume::open`] checked it.
    pub const fn boot_sector(&self) -> &BootSector {
        &self.boot
    }

    /// Give the device back.
    pub fn into_device(self) -> D {
        self.device
    }

    /// Read volume-relative sector `sector` into `buffer`, which is one sector long. Every caller
    /// passes a sector `BootSector::parse` placed inside the volume.
    pub(crate) fn read_sector(
        &mut self,
        sector: u32,
        buffer: &mut [u8],
    ) -> Result<(), ChainError<D::Error>> {
        self.device
            .read_sector(u64::from(sector), buffer)
            .map_err(ChainError::Device)
    }

    /// Make the cache hold table sector `sector`.
    pub(crate) fn load(&mut self, sector: u32) -> Result<(), ChainError<D::Error>> {
        if self.cached != Some(sector) {
            self.cached = None;
            let size = self.boot.sector_size().bytes();
            self.device
                .read_sector(u64::from(sector), &mut self.buffer[..size])
                .map_err(ChainError::Device)?;
            self.cached = Some(sector);
        }
        Ok(())
    }

    /// **Read data cluster `cluster`'s table entry.**
    ///
    /// Refuses a cluster that is not a data cluster of this volume, and passes on the device's
    /// error. A FAT12 entry that straddles two sectors costs two reads.
    ///
    /// # Errors
    ///
    /// [`ChainError::NotADataCluster`](crate::ChainError::NotADataCluster) when `cluster` is not a
    /// data cluster of this volume, and [`ChainError::Device`](crate::ChainError::Device) when the
    /// device refuses a read.
    pub fn table_entry(&mut self, cluster: Cluster) -> Result<TableEntry, ChainError<D::Error>> {
        self.read_table_entry(cluster.0)
    }

    /// [`Volume::table_entry`] by the bare number, for the walks and the writer inside the crate.
    pub(crate) fn read_table_entry(
        &mut self,
        cluster: u32,
    ) -> Result<TableEntry, ChainError<D::Error>> {
        self.boot
            .cluster_first_sector(cluster)
            .ok_or(ChainError::NotADataCluster(Cluster(cluster)))?;
        let raw = self.read_raw_entry(cluster)?;
        Ok(TableEntry::decode(
            self.boot.file_allocation_table_type(),
            raw,
        ))
    }

    /// The raw value of table entry `index`, for an index below `clusters() + 2`: the two reserved
    /// entries are readable here and not through [`Volume::table_entry`].
    pub(crate) fn read_raw_entry(&mut self, index: u32) -> Result<u32, ChainError<D::Error>> {
        let (sector, within) = table::raw_position_in(
            &self.boot,
            index,
            self.boot.active_table().unwrap_or_default(),
        )
        .ok_or(ChainError::NotADataCluster(Cluster(index)))?;
        let file_allocation_table_type = self.boot.file_allocation_table_type();
        let size = self.boot.sector_size().bytes();
        let mut bytes = [0u8; 4];
        let wanted = table::entry_bytes(file_allocation_table_type);
        self.load(sector)?;
        // FAT16 and FAT32 entries are aligned to their width, so only FAT12's split.
        let here = wanted.min(size - within);
        bytes[..here].copy_from_slice(&self.buffer[within..within + here]);
        if here < wanted {
            self.load(sector + 1)?;
            bytes[here..wanted].copy_from_slice(&self.buffer[..wanted - here]);
        }
        Ok(table::assemble(file_allocation_table_type, index, bytes))
    }

    /// **The chain that starts at `first_cluster`**, cluster by cluster.
    ///
    /// The walk yields `first_cluster`, then each cluster the table names, and ends at an end-of-chain
    /// entry. It ends with an error at an entry that cannot continue a chain, at a cluster the
    /// volume does not have, or once it would yield more clusters than the volume holds, which
    /// only a loop can make. After an error it yields nothing more.
    pub fn chain(&mut self, first_cluster: Cluster) -> Chain<'_, D, BUFFER_BYTES> {
        Chain {
            volume: self,
            walk: Walk::new(first_cluster.0),
        }
    }

    /// **The root directory**: a region of FAT12 and FAT16, a chain of FAT32.
    pub fn root_directory(&mut self) -> Directory<'_, D, BUFFER_BYTES> {
        Directory::root(self)
    }

    /// **The root directory as named entries**: each short entry with its checked long name.
    pub fn root_named_entries(&mut self) -> NamedEntries<'_, D, BUFFER_BYTES> {
        NamedEntries::new(self.root_directory())
    }

    /// **The directory `directory`, as named entries.**
    pub fn named_entries(
        &mut self,
        directory: DirectoryCluster,
    ) -> NamedEntries<'_, D, BUFFER_BYTES> {
        NamedEntries::new(self.directory(directory))
    }

    /// **The directory `directory`**: the root for [`DirectoryCluster::ROOT`], else the chain
    /// from its first cluster.
    pub fn directory(&mut self, directory: DirectoryCluster) -> Directory<'_, D, BUFFER_BYTES> {
        if directory == DirectoryCluster::ROOT {
            Directory::root(self)
        } else {
            Directory::chain(self, directory.0)
        }
    }
}

/// **A directory, by the first cluster of its chain**: what the methods that read or write in a
/// directory take. [`DirectoryCluster::ROOT`] is the root directory, and
/// [`NamedEntry::directory_cluster`](crate::NamedEntry::directory_cluster) gives every other one,
/// from the directory's own entry. A file's entry gives none, so a file's first cluster cannot be
/// passed where a directory is meant.
///
/// The number is 0 for the root, the value a `..` entry holds in a directory whose parent is the
/// root, on FAT32 as well as on FAT12 and FAT16.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DirectoryCluster(pub(crate) u32);

impl DirectoryCluster {
    /// The root directory.
    pub const ROOT: DirectoryCluster = DirectoryCluster(0);
}

/// **A cluster number**, as a table entry, a directory entry or a chain walk names it: what
/// [`Volume::chain`] and [`Volume::table_entry`] take and what they yield. The data clusters of a
/// volume are numbered 2 to `clusters() + 1`; a number outside that range is still a `Cluster`,
/// and the methods that take one refuse it with [`ChainError::NotADataCluster`].
///
/// It is its own type so that a sector count or a byte size cannot pass as a cluster. A
/// [`DirectoryCluster`] converts to one: the root's is 0, which no chain starts at.
///
/// # Examples
///
/// ```
/// use file_allocation_table::{Cluster, DirectoryCluster};
///
/// let cluster = Cluster::new(5);
/// assert_eq!(cluster.get(), 5);
/// assert_eq!(Cluster::from(DirectoryCluster::ROOT), Cluster::new(0));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Cluster(pub(crate) u32);

impl Cluster {
    /// The cluster numbered `number`.
    pub const fn new(number: u32) -> Cluster {
        Cluster(number)
    }

    /// The cluster's number.
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<DirectoryCluster> for Cluster {
    fn from(directory: DirectoryCluster) -> Cluster {
        Cluster(directory.0)
    }
}

/// **What a chain walk reads**: how many data clusters the volume has, and one table entry at a
/// time. [`Volume`] is the one implementation; the proofs supply a hostile one.
pub(crate) trait Links {
    /// The device's error.
    type Error;

    /// The volume's data clusters, numbered 2 to `clusters() + 1`.
    fn clusters(&self) -> u32;

    /// Data cluster `cluster`'s entry.
    fn table_entry(&mut self, cluster: u32) -> Result<TableEntry, ChainError<Self::Error>>;
}

impl<D: SectorDevice, const BUFFER_BYTES: usize> Links for Volume<D, BUFFER_BYTES> {
    type Error = D::Error;

    fn clusters(&self) -> u32 {
        self.boot.clusters()
    }

    fn table_entry(&mut self, cluster: u32) -> Result<TableEntry, ChainError<D::Error>> {
        Volume::read_table_entry(self, cluster)
    }
}

/// Whether `cluster` is one of `clusters` data clusters, the same test
/// [`BootSector::cluster_first_sector`] makes.
pub(crate) const fn is_data(clusters: u32, cluster: u32) -> bool {
    cluster >= 2 && cluster - 2 < clusters
}

/// **Where a chain walk stands**, apart from the volume it reads, so a directory can walk a
/// chain while it reads the clusters.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Walk {
    /// The cluster to yield next, or `None` once the walk has ended.
    next: Option<u32>,
    /// The cluster yielded last, whose entry names the next one.
    last: Option<u32>,
    /// How many clusters the walk has yielded.
    yielded: u32,
}

impl Walk {
    /// A walk from `first`, which the first step checks.
    pub(crate) const fn new(first: u32) -> Walk {
        Walk {
            next: Some(first),
            last: None,
            yielded: 0,
        }
    }

    /// The next cluster, `None` at the end, or the error that ends the walk.
    pub(crate) fn step<L: Links>(
        &mut self,
        links: &mut L,
    ) -> Option<Result<u32, ChainError<L::Error>>> {
        let result = self.advance(links);
        if !matches!(result, Some(Ok(_))) {
            self.next = None;
            self.last = None;
        }
        result
    }

    fn advance<L: Links>(&mut self, links: &mut L) -> Option<Result<u32, ChainError<L::Error>>> {
        let clusters = links.clusters();
        let cluster = match (self.next.take(), self.last) {
            (Some(first), None) => {
                if !is_data(clusters, first) {
                    return Some(Err(ChainError::NotADataCluster(Cluster(first))));
                }
                first
            }
            (None, Some(last)) => match links.table_entry(last) {
                Err(error) => return Some(Err(error)),
                Ok(TableEntry::EndOfChain) => return None,
                Ok(TableEntry::Next(Cluster(next))) if is_data(clusters, next) => next,
                Ok(entry) => {
                    return Some(Err(ChainError::Broken {
                        cluster: Cluster(last),
                        entry,
                    }));
                }
            },
            _ => return None,
        };
        if self.yielded >= clusters {
            return Some(Err(ChainError::TooLong));
        }
        self.yielded += 1;
        self.last = Some(cluster);
        Some(Ok(cluster))
    }
}

/// **A cluster chain, walked one cluster at a time**: what [`Volume::chain`] returns.
///
/// It yields at most `clusters()` clusters and then ends, whatever the table holds.
#[derive(Debug)]
pub struct Chain<'v, D, const BUFFER_BYTES: usize = { SectorSize::MAXIMUM_BYTES }> {
    volume: &'v mut Volume<D, BUFFER_BYTES>,
    walk: Walk,
}

impl<D: SectorDevice, const BUFFER_BYTES: usize> Iterator for Chain<'_, D, BUFFER_BYTES> {
    type Item = Result<Cluster, ChainError<D::Error>>;

    fn next(&mut self) -> Option<Self::Item> {
        self.walk.step(self.volume).map(|step| step.map(Cluster))
    }
}

#[cfg(kani)]
pub(crate) mod verification;

#[cfg(test)]
pub(crate) mod tests;
