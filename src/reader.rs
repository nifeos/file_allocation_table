//! **An open volume**: a device, its checked boot sector, and the reads every later structure
//! makes through them. It reads the table entry by entry and walks cluster chains with a bound
//! no table contents can defeat.

use crate::allocation::WriterState;
use crate::boot_sector::{BootSector, ReadError};
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
    NotADataCluster(u32),
    /// `cluster`'s table entry cannot continue a chain: it is free, reserved or bad, or names a
    /// cluster the volume does not have.
    Broken {
        /// The cluster whose entry was read.
        cluster: u32,
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
/// use file_allocation_table::efi_system_partition::BYTES_PER_SECTOR;
/// use file_allocation_table::{
///     NoClock, SectorDevice, SectorSize, Volume, VolumeLayout, VolumeLayoutOptions,
/// };
///
/// // A volume this crate's writer lays out, held in memory.
/// let options = VolumeLayoutOptions::new(70_000 * 8 + 2_000)
///     .with_volume_id(1)
///     .with_label("EFI SYSTEM")
///     .and_then(|options| options.with_file("BOOTX64.EFI", 5_000))
///     .unwrap();
/// let layout = VolumeLayout::new(options, &mut NoClock).unwrap();
/// struct Image(VolumeLayout);
/// impl SectorDevice for Image {
///     type Error = ();
///     fn sector_size(&self) -> SectorSize {
///         SectorSize::Bytes512
///     }
///     fn read_sector(&mut self, index: u64, buffer: &mut [u8]) -> Result<(), ()> {
///         if !self.0.fill_sector(index, buffer) {
///             buffer.fill(0);
///         }
///         Ok(())
///     }
/// }
///
/// let mut volume = Volume::open(Image(layout)).unwrap();
/// let clusters: Vec<u32> = volume.chain(5).map(Result::unwrap).collect();
/// assert_eq!(clusters, [5, 6]);
/// ```
#[derive(Debug)]
pub struct Volume<D> {
    pub(crate) device: D,
    pub(crate) boot: BootSector,
    pub(crate) cached: Option<u32>,
    pub(crate) buffer: [u8; SectorSize::MAXIMUM_BYTES],
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
    pub fn open(mut device: D) -> Result<Volume<D>, ReadError<D::Error>> {
        let boot = BootSector::read(&mut device)?;
        Ok(Volume {
            device,
            boot,
            cached: None,
            buffer: [0; SectorSize::MAXIMUM_BYTES],
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
    pub fn table_entry(&mut self, cluster: u32) -> Result<TableEntry, ChainError<D::Error>> {
        self.boot
            .cluster_first_sector(cluster)
            .ok_or(ChainError::NotADataCluster(cluster))?;
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
        .ok_or(ChainError::NotADataCluster(index))?;
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
    pub fn chain(&mut self, first_cluster: u32) -> Chain<'_, D> {
        Chain {
            volume: self,
            walk: Walk::new(first_cluster),
        }
    }

    /// **The root directory**: a region of FAT12 and FAT16, a chain of FAT32.
    pub fn root_directory(&mut self) -> Directory<'_, D> {
        Directory::root(self)
    }

    /// **The root directory as named entries**: each short entry with its checked long name.
    pub fn root_named_entries(&mut self) -> NamedEntries<'_, D> {
        NamedEntries::new(self.root_directory())
    }

    /// **The directory `directory`, as named entries.**
    pub fn named_entries(&mut self, directory: DirectoryCluster) -> NamedEntries<'_, D> {
        NamedEntries::new(self.directory(directory))
    }

    /// **The directory `directory`**: the root for [`DirectoryCluster::ROOT`], else the chain
    /// from its first cluster.
    pub fn directory(&mut self, directory: DirectoryCluster) -> Directory<'_, D> {
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

impl<D: SectorDevice> Links for Volume<D> {
    type Error = D::Error;

    fn clusters(&self) -> u32 {
        self.boot.clusters()
    }

    fn table_entry(&mut self, cluster: u32) -> Result<TableEntry, ChainError<D::Error>> {
        Volume::table_entry(self, cluster)
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
                    return Some(Err(ChainError::NotADataCluster(first)));
                }
                first
            }
            (None, Some(last)) => match links.table_entry(last) {
                Err(error) => return Some(Err(error)),
                Ok(TableEntry::EndOfChain) => return None,
                Ok(TableEntry::Next(next)) if is_data(clusters, next) => next,
                Ok(entry) => {
                    return Some(Err(ChainError::Broken {
                        cluster: last,
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
pub struct Chain<'v, D> {
    volume: &'v mut Volume<D>,
    walk: Walk,
}

impl<D: SectorDevice> Iterator for Chain<'_, D> {
    type Item = Result<u32, ChainError<D::Error>>;

    fn next(&mut self) -> Option<Self::Item> {
        self.walk.step(self.volume)
    }
}

#[cfg(kani)]
pub(crate) mod verification;

#[cfg(test)]
pub(crate) mod tests;
