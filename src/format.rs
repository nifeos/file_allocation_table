//! **The general format**: lay out an empty FAT12, FAT16 or FAT32 volume of any size and sector
//! size, as the sectors a caller writes, with no I/O.
//!
//! One piece of arithmetic sizes every volume, [`Geometry`], and one pair of builders writes its
//! boot sector and file system information sector; the one-file EFI volume of
//! [`crate::VolumeLayout`] uses both, so the `BPB` is written in one place. Sizing is Microsoft's
//! own (`fatgen103`, "Determining FAT type"), which over-sizes the table slightly rather than solve
//! the circular dependency, so the number it gives is the number every other tool gives; the data
//! area is then rounded up to start on a cluster boundary.

use crate::boot_sector::FileAllocationTableType;
use crate::long_name::is_short_byte;
use crate::sector::{SectorSize, WritableSectorDevice};
use crate::time::{Clock, Stamp};

/// Reserved sectors of a FAT32 volume: the boot sector, the file system information sector, their
/// backups at 6 and 7, and room left over as every tool leaves it.
const FAT32_RESERVED: u32 = 32;
/// Root directory entries of a FAT12 or FAT16 volume, the count most tools write.
const ROOT_ENTRIES: u32 = 512;
/// The cluster FAT32's root directory starts at.
const ROOT_CLUSTER: u32 = 2;
/// The most clusters FAT32 addresses.
const FAT32_MOST_CLUSTERS: u32 = 0x0FFF_FFF5 - 2;
/// `ATTR_VOLUME_ID`.
const ATTR_VOLUME_ID: u8 = 0x08;

/// **Why a volume cannot be laid out.** Each is a refusal before any byte is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FormatError {
    /// The device is too small for the reserved sectors, a table and one cluster.
    TooSmall,
    /// No cluster size gives the requested type a legal cluster count: the volume is too small or
    /// too large for it. A FAT32 volume needs 65,525 clusters and a FAT16 volume 4,085.
    NoSuchGeometry,
    /// The label is longer than eleven bytes or holds a byte a short name refuses.
    LabelRefused,
    /// The clock reported a time FAT cannot store (see [`crate::Clock`]).
    TimeOutOfRange,
    /// The cluster size is not a power of two from 1 to 128 sectors.
    ClusterSizeRefused,
}

/// **What to format**: the device's size and sector size, and optionally the type, the cluster
/// size, the serial number, the label and the sectors ahead of the volume on its disk.
///
/// Without a cluster size the format picks one by the volume's size, from the specification's
/// tables (`DskTableFAT16` and `DskTableFAT32` in Microsoft's `fatgen103`), as Windows does and as
/// `mkfs.fat` does for FAT32. Without a type it picks that by size too, as both of them do:
///
/// | volume size | type | cluster |
/// |---|---|---|
/// | up to 8,400 sectors of 512 bytes (about 4.1 MiB) | FAT12 | the smallest that gives a legal count |
/// | to 16 MiB | FAT16 | 1 KiB |
/// | to 128 MiB | FAT16 | 2 KiB |
/// | to 256 MiB | FAT16 | 4 KiB |
/// | below 512 MiB | FAT16 | 8 KiB |
/// | to 8 GiB | FAT32 | 4 KiB |
/// | to 16 GiB | FAT32 | 8 KiB |
/// | to 32 GiB | FAT32 | 16 KiB |
/// | larger | FAT32 | 32 KiB |
///
/// A cluster is never smaller than one sector. A named type takes its own table's row for the
/// size (FAT16 runs on to 32 KiB at 2 GiB, and FAT32 starts at 512 bytes up to 260 MiB). Where a
/// row gives no legal count (a sector size above 512 bytes can do that, as can a type named for a
/// size outside its table), the smallest cluster that gives one wins, and without a named type the
/// type follows from the count. A named cluster size,
/// [`FormatOptions::with_sectors_per_cluster`], replaces the table, and the type then follows from
/// the count: one sector to the cluster is the explicit choice of the smallest table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatOptions {
    bytes_per_sector: SectorSize,
    total_sectors: u32,
    file_allocation_table_type: Option<FileAllocationTableType>,
    cluster_sectors: Option<u32>,
    volume_id: u32,
    label: [u8; 11],
    has_label: bool,
    hidden_sectors: u32,
}

impl FormatOptions {
    /// Options for a device of `total_sectors` sectors of `sector_size`, with every choice left to
    /// the format: serial number 0, the label `NO NAME`, no hidden sectors.
    pub const fn new(sector_size: SectorSize, total_sectors: u32) -> FormatOptions {
        FormatOptions {
            bytes_per_sector: sector_size,
            total_sectors,
            file_allocation_table_type: None,
            cluster_sectors: None,
            volume_id: 0,
            label: *b"NO NAME    ",
            has_label: false,
            hidden_sectors: 0,
        }
    }

    /// Require this type, or a refusal.
    #[must_use]
    pub const fn with_file_allocation_table_type(
        mut self,
        kind: FileAllocationTableType,
    ) -> FormatOptions {
        self.file_allocation_table_type = Some(kind);
        self
    }

    /// Require this many sectors to the cluster: a power of two from 1 to 128.
    #[must_use]
    pub const fn with_sectors_per_cluster(mut self, sectors: u32) -> FormatOptions {
        self.cluster_sectors = Some(sectors);
        self
    }

    /// The serial number; a caller with an entropy capability draws it, and nothing here invents
    /// one.
    #[must_use]
    pub const fn with_volume_id(mut self, volume_id: u32) -> FormatOptions {
        self.volume_id = volume_id;
        self
    }

    /// The sectors ahead of the volume on its disk, which only `BPB_HiddSec` records.
    #[must_use]
    pub const fn with_hidden_sectors(mut self, hidden_sectors: u32) -> FormatOptions {
        self.hidden_sectors = hidden_sectors;
        self
    }

    /// The volume label: up to eleven upper-case letters, digits, spaces and the punctuation a
    /// short name admits. A label also becomes the volume-label entry of the root directory.
    pub fn with_label(mut self, label: &str) -> Result<FormatOptions, FormatError> {
        self.label = label_bytes(label).ok_or(FormatError::LabelRefused)?;
        self.has_label = true;
        Ok(self)
    }
}

/// The eleven bytes of `BS_VolLab` for `label`, or `None` when it is empty, over eleven bytes, or
/// holds a byte a short name refuses. The text is padded with spaces.
pub(crate) fn label_bytes(label: &str) -> Option<[u8; 11]> {
    label_from_bytes(label.as_bytes())
}

/// [`label_bytes`] over the text's bytes, which the proof reaches without a text to validate.
pub(crate) fn label_from_bytes(bytes: &[u8]) -> Option<[u8; 11]> {
    if bytes.is_empty() || bytes.len() > 11 || !bytes.iter().all(|b| is_short_byte(*b)) {
        return None;
    }
    let mut out = [b' '; 11];
    out[..bytes.len()].copy_from_slice(bytes);
    Some(out)
}

/// **A volume's measurements**, from its size alone. Every field is derived by [`Geometry::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Geometry {
    pub(crate) kind: FileAllocationTableType,
    pub(crate) bytes_per_sector: SectorSize,
    pub(crate) cluster_sectors: u32,
    pub(crate) reserved_sectors: u32,
    pub(crate) root_entries: u32,
    pub(crate) root_sectors: u32,
    pub(crate) table_sectors: u32,
    pub(crate) total_sectors: u32,
    pub(crate) clusters: u32,
}

/// Why a geometry does not exist for the choices made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GeometryError {
    /// The device cannot hold the reserved sectors, a table and a cluster.
    TooSmall,
    /// The cluster count, carried here, falls in another type's range or past what the type
    /// addresses.
    WrongType(u32),
}

impl Geometry {
    /// **Size a volume of `kind` with `cluster_sectors` sectors to the cluster** on a device of
    /// `total_sectors`, by Microsoft's sizing with the data area rounded to a cluster boundary.
    pub(crate) fn new(
        kind: FileAllocationTableType,
        sector_size: SectorSize,
        cluster_sectors: u32,
        total_sectors: u32,
    ) -> Result<Geometry, GeometryError> {
        let fat32 = kind == FileAllocationTableType::Fat32;
        let bytes = sector_size.bytes() as u32;
        let reserved_sectors = if fat32 { FAT32_RESERVED } else { 1 };
        let root_entries = if fat32 { 0 } else { ROOT_ENTRIES };
        let root_sectors = (root_entries * 32).div_ceil(bytes);
        let after_reserved = total_sectors
            .checked_sub(reserved_sectors + root_sectors)
            .ok_or(GeometryError::TooSmall)?;
        let divisor = (bytes / 2 * cluster_sectors + 2) / if fat32 { 2 } else { 1 };
        let mut table_sectors = after_reserved.div_ceil(divisor);
        // Start the data area on a cluster boundary, so a cluster begins where a device block does.
        // FAT32 grows its tables a sector at a time (two tables move the start by two, and its 32
        // reserved sectors are even, so an even cluster size always meets). A FAT12 or FAT16 start
        // can be odd, so those pad their reserved sectors instead. Either way the table must hold
        // an entry for every cluster and the two reserved ones: Microsoft's formula, which the
        // number above is, falls two entries short when the volume divides it exactly, so a table
        // that is too small grows by a sector and the sizing runs again.
        let bits = u64::from(kind.entry_bits());
        let mut reserved_sectors = reserved_sectors;
        let mut clusters = 0;
        let mut settled = false;
        for _ in 0..cluster_sectors + 8 {
            let padding = if fat32 {
                0
            } else {
                let start = reserved_sectors + root_sectors + 2 * table_sectors;
                (cluster_sectors - start % cluster_sectors) % cluster_sectors
            };
            let first_data = reserved_sectors + padding + root_sectors + 2 * table_sectors;
            if fat32 && first_data % cluster_sectors != 0 {
                table_sectors += 1;
                continue;
            }
            let data_sectors = total_sectors
                .checked_sub(first_data)
                .ok_or(GeometryError::TooSmall)?;
            clusters = data_sectors / cluster_sectors;
            let capacity = u64::from(table_sectors) * u64::from(bytes) * 8 / bits;
            if capacity < u64::from(clusters) + 2 {
                table_sectors += 1;
                continue;
            }
            reserved_sectors += padding;
            settled = true;
            break;
        }
        if !settled || clusters == 0 {
            return Err(GeometryError::TooSmall);
        }
        let in_range = FileAllocationTableType::from_clusters(clusters) == kind
            && (!fat32 || clusters <= FAT32_MOST_CLUSTERS);
        if !in_range {
            return Err(GeometryError::WrongType(clusters));
        }
        Ok(Geometry {
            kind,
            bytes_per_sector: sector_size,
            cluster_sectors,
            reserved_sectors,
            root_entries,
            root_sectors,
            table_sectors,
            total_sectors,
            clusters,
        })
    }

    /// The sector cluster 2 begins at.
    pub(crate) const fn first_data_sector(&self) -> u32 {
        self.reserved_sectors + self.root_sectors + 2 * self.table_sectors
    }

    /// Write the boot sector into `out`, which is one sector long and zeroed. `label` is the
    /// eleven bytes of `BS_VolLab`.
    pub(crate) fn write_boot_sector(
        &self,
        volume_id: u32,
        label: &[u8; 11],
        hidden_sectors: u32,
        out: &mut [u8],
    ) {
        let fat32 = self.kind == FileAllocationTableType::Fat32;
        // A jump every FAT driver expects to find, over the BPB. Nothing executes it here.
        out[0..3].copy_from_slice(&[0xEB, if fat32 { 0x58 } else { 0x3C }, 0x90]);
        out[3..11].copy_from_slice(b"MSWIN4.1"); // BS_OEMName; the spec's own recommendation
        put16(out, 11, self.bytes_per_sector.bytes() as u16); // BPB_BytsPerSec
        out[13] = self.cluster_sectors as u8; // BPB_SecPerClus
        put16(out, 14, self.reserved_sectors as u16); // BPB_RsvdSecCnt
        out[16] = 2; // BPB_NumFATs
        put16(out, 17, self.root_entries as u16); // BPB_RootEntCnt, zero on FAT32
        let small = !fat32 && self.total_sectors < 0x1_0000;
        put16(out, 19, if small { self.total_sectors as u16 } else { 0 }); // BPB_TotSec16
        out[21] = 0xF8; // BPB_Media, "fixed disk"
        put16(out, 22, if fat32 { 0 } else { self.table_sectors as u16 }); // BPB_FATSz16
        put16(out, 24, 63); // BPB_SecPerTrk, a geometry nothing has had since 2005
        put16(out, 26, 255); // BPB_NumHeads, likewise
        put32(out, 28, hidden_sectors); // BPB_HiddSec
        put32(out, 32, if small { 0 } else { self.total_sectors }); // BPB_TotSec32
        let name: &[u8; 8] = match self.kind {
            FileAllocationTableType::Fat12 => b"FAT12   ",
            FileAllocationTableType::Fat16 => b"FAT16   ",
            FileAllocationTableType::Fat32 => b"FAT32   ",
        };
        if fat32 {
            put32(out, 36, self.table_sectors); // BPB_FATSz32
            put16(out, 40, 0); // BPB_ExtFlags: both tables live, mirrored
            put16(out, 42, 0); // BPB_FSVer
            put32(out, 44, ROOT_CLUSTER); // BPB_RootClus
            put16(out, 48, 1); // the file system information sector's number
            put16(out, 50, 6); // BPB_BkBootSec
            out[64] = 0x80; // BS_DrvNum
            out[66] = 0x29; // BS_BootSig, which makes the three fields below meaningful
            put32(out, 67, volume_id); // BS_VolID
            out[71..82].copy_from_slice(label); // BS_VolLab
            out[82..90].copy_from_slice(name); // BS_FilSysType, informational and still checked
        } else {
            out[36] = 0x80; // BS_DrvNum
            out[38] = 0x29; // BS_BootSig
            put32(out, 39, volume_id); // BS_VolID
            out[43..54].copy_from_slice(label); // BS_VolLab
            out[54..62].copy_from_slice(name); // BS_FilSysType
        }
        out[510] = 0x55;
        out[511] = 0xAA;
    }
}

/// Write the file system information sector's signatures and its two hints into `out`, which is one
/// zeroed sector.
pub(crate) fn write_fs_info(free_clusters: u32, next_free: u32, out: &mut [u8]) {
    put32(out, 0, 0x4161_5252); // FSI_LeadSig, "RRaA"
    put32(out, 484, 0x6141_7272); // FSI_StrucSig, "rrAa"
    put32(out, 488, free_clusters); // FSI_Free_Count
    put32(out, 492, next_free); // FSI_Nxt_Free
    put32(out, 508, 0xAA55_0000); // FSI_TrailSig
}

/// A little-endian `u16` at `at`.
pub(crate) fn put16(out: &mut [u8], at: usize, value: u16) {
    out[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

/// A little-endian `u32` at `at`.
pub(crate) fn put32(out: &mut [u8], at: usize, value: u32) {
    out[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

/// **The type and cluster size a format picks by the volume's size**, when the caller names no
/// cluster size: `kind` when named, else FAT12 up to 8,400 sectors of 512 bytes, FAT16 below
/// 512 MiB and FAT32 from there, with the row of the specification's `DskTableFAT16` or
/// `DskTableFAT32` for the size, in sectors of `sector_size` and at least one. `None` for the
/// cluster where the table has no row: FAT12, which it does not cover, and a FAT16 or FAT32 size
/// it calls an error.
///
/// The tables count the volume and the cluster in sectors of 512 bytes; another sector size is
/// converted through bytes, as a formatter on a 4K-native disk keeps the cluster's bytes.
pub(crate) fn by_volume_size(
    sector_size: SectorSize,
    total_sectors: u32,
    kind: Option<FileAllocationTableType>,
) -> (FileAllocationTableType, Option<u32>) {
    /// `DskTableFAT16`: the first row whose size, in 512-byte sectors, the volume does not exceed
    /// gives the cluster in 512-byte sectors; 0 is the table's "use another type".
    const FAT16: [(u64, u32); 8] = [
        (8_400, 0),
        (32_680, 2),
        (262_144, 4),
        (524_288, 8),
        (1_048_576, 16),
        (2_097_152, 32),
        (4_194_304, 64),
        (u64::MAX, 0),
    ];
    /// `DskTableFAT32`, read the same way.
    const FAT32: [(u64, u32); 6] = [
        (66_600, 0),
        (532_480, 1),
        (16_777_216, 8),
        (33_554_432, 16),
        (67_108_864, 32),
        (u64::MAX, 64),
    ];
    let bytes = sector_size.bytes() as u64;
    let units = u64::from(total_sectors) * bytes / 512;
    let kind = kind.unwrap_or(if units <= 8_400 {
        FileAllocationTableType::Fat12
    } else if units < 1_048_576 {
        FileAllocationTableType::Fat16
    } else {
        FileAllocationTableType::Fat32
    });
    let table: &[(u64, u32)] = match kind {
        FileAllocationTableType::Fat12 => return (kind, None),
        FileAllocationTableType::Fat16 => &FAT16,
        FileAllocationTableType::Fat32 => &FAT32,
    };
    let units_per_cluster = table
        .iter()
        .find(|(most, _)| units <= *most)
        .map_or(0, |(_, cluster)| *cluster);
    if units_per_cluster == 0 {
        return (kind, None);
    }
    let sectors = (u64::from(units_per_cluster) * 512 / bytes).max(1) as u32;
    (kind, Some(sectors))
}

/// **An empty volume, laid out but not written.** Every method is arithmetic; nothing here touches
/// a device until [`FormatPlan::write_to`].
///
/// # Examples
///
/// ```
/// use file_allocation_table::{
///     FileAllocationTableType, FormatPlan, FormatOptions, SectorSize, NoClock, Volume,
/// };
///
/// // 8 MiB of 512-byte sectors: too many clusters for FAT12, so FAT16.
/// let format = FormatPlan::new(FormatOptions::new(SectorSize::Bytes512, 16 * 1024), &mut NoClock).unwrap();
/// assert_eq!(format.file_allocation_table_type(), FileAllocationTableType::Fat16);
/// let mut boot = [0u8; 512];
/// assert!(format.fill_sector(0, &mut boot));
/// let parsed = file_allocation_table::BootSector::parse(&boot).unwrap();
/// assert_eq!(parsed.file_allocation_table_type(), format.file_allocation_table_type());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatPlan {
    geometry: Geometry,
    volume_id: u32,
    label: [u8; 11],
    has_label: bool,
    hidden_sectors: u32,
    stamp: Stamp,
}

impl FormatPlan {
    /// **Lay out the volume `options` describe**, or say why none exists. The label entry, when
    /// there is one, carries the time `clock` reports now (read once); `NoClock` gives the zero
    /// timestamp and a byte-for-byte reproducible image.
    pub fn new(options: FormatOptions, clock: &mut impl Clock) -> Result<FormatPlan, FormatError> {
        const KINDS: [FileAllocationTableType; 3] = [
            FileAllocationTableType::Fat12,
            FileAllocationTableType::Fat16,
            FileAllocationTableType::Fat32,
        ];
        const CLUSTERS: [u32; 8] = [1, 2, 4, 8, 16, 32, 64, 128];
        if let Some(sectors) = options.cluster_sectors
            && !CLUSTERS.contains(&sectors)
        {
            return Err(FormatError::ClusterSizeRefused);
        }
        let stamp = Stamp::pack(clock.now()).map_err(|_| FormatError::TimeOutOfRange)?;
        // The smallest cluster of `only_sectors` (any when `None`) that gives a type of
        // `only_kind` (any when `None`) a legal count, and whether some size was too small.
        let search = |only_kind: Option<FileAllocationTableType>, only_sectors: Option<u32>| {
            let mut seen_small = false;
            for sectors in CLUSTERS {
                if only_sectors.is_some_and(|only| only != sectors) {
                    continue;
                }
                for kind in KINDS {
                    if only_kind.is_some_and(|only| only != kind) {
                        continue;
                    }
                    match Geometry::new(
                        kind,
                        options.bytes_per_sector,
                        sectors,
                        options.total_sectors,
                    ) {
                        Ok(geometry) => return (Some(geometry), seen_small),
                        Err(GeometryError::TooSmall) => seen_small = true,
                        Err(GeometryError::WrongType(_)) => {}
                    }
                }
            }
            (None, seen_small)
        };
        let mut found = None;
        if options.cluster_sectors.is_none() {
            let (kind, sectors) = by_volume_size(
                options.bytes_per_sector,
                options.total_sectors,
                options.file_allocation_table_type,
            );
            found = search(Some(kind), sectors).0;
        }
        // The table's row gave no legal count, or a cluster size was named: the smallest cluster
        // the options allow.
        let mut seen_small = false;
        if found.is_none() {
            (found, seen_small) =
                search(options.file_allocation_table_type, options.cluster_sectors);
        }
        let geometry = found.ok_or(if seen_small {
            FormatError::TooSmall
        } else {
            FormatError::NoSuchGeometry
        })?;
        Ok(FormatPlan {
            geometry,
            volume_id: options.volume_id,
            label: options.label,
            has_label: options.has_label,
            hidden_sectors: options.hidden_sectors,
            stamp,
        })
    }

    /// The type the volume is.
    pub const fn file_allocation_table_type(&self) -> FileAllocationTableType {
        self.geometry.kind
    }

    /// The data clusters the volume has.
    pub const fn clusters(&self) -> u32 {
        self.geometry.clusters
    }

    /// Sectors to the cluster.
    pub const fn sectors_per_cluster(&self) -> u32 {
        self.geometry.cluster_sectors
    }

    /// Sectors in one copy of the table.
    pub const fn table_sectors(&self) -> u32 {
        self.geometry.table_sectors
    }

    /// How many leading sectors [`FormatPlan::fill_sector`] answers for: the reserved sectors, both
    /// tables, the fixed root of FAT12 and FAT16, and FAT32's root cluster.
    pub const fn metadata_sectors(&self) -> u32 {
        let first = self.geometry.first_data_sector();
        match self.geometry.kind {
            FileAllocationTableType::Fat32 => first + self.geometry.cluster_sectors,
            _ => first,
        }
    }

    /// **Fill `out` with volume-relative sector `index`.** `false`, with `out` untouched, when
    /// `index` is at or past [`FormatPlan::metadata_sectors`] or `out` is not one sector long.
    ///
    /// Every sector this answers for is written whole, zeros included, so a volume laid over a
    /// disk that held something before inherits no stale table word or directory entry. The data
    /// clusters beyond are not touched.
    pub fn fill_sector(&self, index: u32, out: &mut [u8]) -> bool {
        let g = &self.geometry;
        if out.len() != g.bytes_per_sector.bytes() || index >= self.metadata_sectors() {
            return false;
        }
        out.fill(0);
        let fat32 = g.kind == FileAllocationTableType::Fat32;
        let tables = g.reserved_sectors..g.reserved_sectors + 2 * g.table_sectors;
        if index == 0 || (fat32 && index == 6) {
            g.write_boot_sector(self.volume_id, &self.label, self.hidden_sectors, out);
        } else if fat32 && (index == 1 || index == 7) {
            write_fs_info(g.clusters - 1, ROOT_CLUSTER + 1, out);
        } else if tables.contains(&index) {
            if index == g.reserved_sectors || index == g.reserved_sectors + g.table_sectors {
                self.first_table_sector(out);
            }
        } else if index == g.reserved_sectors + 2 * g.table_sectors && self.has_label {
            // The first sector of the root directory, FAT12 and FAT16's region or FAT32's cluster.
            out[..11].copy_from_slice(&self.label);
            out[11] = ATTR_VOLUME_ID;
            self.stamp.write_new(&mut out[..32]);
        }
        true
    }

    /// The first sector of a table: the media byte, the end-of-chain marker of entry 1 (which also
    /// says the volume was shut down cleanly), and on FAT32 the end of the root directory's chain.
    fn first_table_sector(&self, out: &mut [u8]) {
        match self.geometry.kind {
            FileAllocationTableType::Fat12 => out[..3].copy_from_slice(&[0xF8, 0xFF, 0xFF]),
            FileAllocationTableType::Fat16 => out[..4].copy_from_slice(&[0xF8, 0xFF, 0xFF, 0xFF]),
            FileAllocationTableType::Fat32 => {
                put32(out, 0, 0x0FFF_FFF8);
                put32(out, 4, 0x0FFF_FFFF);
                put32(out, 8, 0x0FFF_FFFF);
            }
        }
    }

    /// **Write the whole format to `device`**, sector by sector, and return the device's first
    /// error. The device must have the sector size the options named.
    pub fn write_to<D: WritableSectorDevice>(&self, device: &mut D) -> Result<(), D::Error> {
        let mut sector = [0u8; SectorSize::MAXIMUM_BYTES];
        let size = self.geometry.bytes_per_sector.bytes();
        for index in 0..self.metadata_sectors() {
            if self.fill_sector(index, &mut sector[..size]) {
                device.write_sector(u64::from(index), &sector[..size])?;
            }
        }
        Ok(())
    }
}

#[cfg(kani)]
mod verification;

#[cfg(test)]
mod tests;
