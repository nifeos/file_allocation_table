//! The general format's sizing at the edges of each type, the sectors it writes, and its refusals.

use super::*;
use crate::NoClock;
use crate::boot_sector::BootSector;

fn options(sectors: u32) -> FormatOptions {
    FormatOptions::new(SectorSize::Bytes512, sectors)
}

#[test]
fn the_type_and_the_cluster_follow_the_volumes_size_by_the_specifications_tables() {
    use FileAllocationTableType::{Fat12, Fat16, Fat32};
    // Each row's last size and the size after it, in 512-byte sectors.
    for (sectors, kind, cluster) in [
        (2 * 1024, Fat12, 1),
        (4 * 1024, Fat12, 1),
        (8_400, Fat12, 4),
        (8_401, Fat16, 2),
        (32_680, Fat16, 2),
        (32_681, Fat16, 4),
        (262_144, Fat16, 4),
        (262_145, Fat16, 8),
        (524_288, Fat16, 8),
        (524_289, Fat16, 16),
        (1_048_575, Fat16, 16),
        (1_048_576, Fat32, 8),
        (16_777_216, Fat32, 8),
        (16_777_217, Fat32, 16),
        (33_554_433, Fat32, 32),
        (67_108_865, Fat32, 64),
    ] {
        let format = FormatPlan::new(options(sectors), &mut NoClock).unwrap();
        assert_eq!(
            (
                format.file_allocation_table_type(),
                format.sectors_per_cluster()
            ),
            (kind, cluster),
            "{sectors} sectors"
        );
    }
}

#[test]
fn a_size_whose_row_gives_no_legal_count_takes_the_smallest_cluster_that_does() {
    use FileAllocationTableType::{Fat16, Fat32};
    // FAT32 named on a size its table calls too small for FAT32: the smallest cluster that gives
    // FAT32 a legal count, and none does.
    assert_eq!(
        FormatPlan::new(
            options(40 * 1024).with_file_allocation_table_type(Fat32),
            &mut NoClock
        )
        .err(),
        Some(FormatError::NoSuchGeometry)
    );
    // A 4096-byte sector is larger than the 2 KiB row, so the cluster is one sector.
    let format = FormatPlan::new(
        FormatOptions::new(SectorSize::Bytes4096, 16 * 1024),
        &mut NoClock,
    )
    .unwrap();
    assert_eq!(
        (
            format.file_allocation_table_type(),
            format.sectors_per_cluster()
        ),
        (Fat16, 1)
    );
}

#[test]
fn a_named_type_takes_its_own_tables_row_and_a_named_cluster_replaces_the_table() {
    use FileAllocationTableType::{Fat16, Fat32};
    let format = FormatPlan::new(
        options(200 * 1024).with_file_allocation_table_type(Fat32),
        &mut NoClock,
    )
    .unwrap();
    assert_eq!(
        (
            format.file_allocation_table_type(),
            format.sectors_per_cluster()
        ),
        (Fat32, 1)
    );
    let format = FormatPlan::new(
        options(1_500_000).with_file_allocation_table_type(Fat16),
        &mut NoClock,
    )
    .unwrap();
    assert_eq!(
        (
            format.file_allocation_table_type(),
            format.sectors_per_cluster()
        ),
        (Fat16, 32)
    );
    // One sector to the cluster, named, gives the smallest table, and the type follows the count.
    let format = FormatPlan::new(
        options(200 * 1024).with_sectors_per_cluster(1),
        &mut NoClock,
    )
    .unwrap();
    assert_eq!(
        (
            format.file_allocation_table_type(),
            format.sectors_per_cluster()
        ),
        (Fat32, 1)
    );
}

#[test]
fn the_fat32_numbers_are_the_efi_volumes_numbers() {
    // 512 MiB in 512-byte sectors with 4096-byte clusters, a common EFI system partition.
    let format = FormatPlan::new(
        options(1024 * 1024)
            .with_file_allocation_table_type(FileAllocationTableType::Fat32)
            .with_sectors_per_cluster(8),
        &mut NoClock,
    )
    .unwrap();
    let layout = crate::VolumeLayoutOptions::new(1024 * 1024)
        .with_volume_id(1)
        .with_label("EFI SYSTEM")
        .and_then(|o| o.with_file("BOOTX64.EFI", 4096))
        .and_then(|o| crate::VolumeLayout::new(o, &mut NoClock))
        .unwrap();
    assert_eq!(format.clusters(), layout.clusters());
    assert_eq!(format.table_sectors(), layout.table_sectors());
    assert_eq!(
        u64::from(format.geometry.first_data_sector()),
        layout.first_data_sector()
    );
}

#[test]
fn the_data_area_starts_on_a_cluster_boundary_for_every_type_and_cluster_size() {
    for (kind, sectors) in [
        (FileAllocationTableType::Fat12, 4 * 1024),
        (FileAllocationTableType::Fat16, 80 * 1024),
        (FileAllocationTableType::Fat32, 160 * 1024),
    ] {
        for cluster in [1, 2, 4, 8, 16] {
            let Ok(format) = FormatPlan::new(
                options(sectors)
                    .with_file_allocation_table_type(kind)
                    .with_sectors_per_cluster(cluster),
                &mut NoClock,
            ) else {
                continue;
            };
            assert_eq!(
                format.geometry.first_data_sector() % cluster,
                0,
                "{kind:?} {cluster}"
            );
        }
    }
}

#[test]
fn every_sector_the_format_writes_is_whole_and_the_boot_sector_parses_to_the_same_volume() {
    let format =
        FormatPlan::new(options(64 * 1024).with_volume_id(0x1234_5678), &mut NoClock).unwrap();
    let mut sector = [0u8; 512];
    assert!(format.fill_sector(0, &mut sector));
    let boot = BootSector::parse(&sector).unwrap();
    assert_eq!(boot.clusters(), format.clusters());
    assert_eq!(boot.table_sectors(), format.table_sectors());
    assert_eq!(boot.volume_id(), Some(0x1234_5678));
    // Past the metadata, or into a buffer of the wrong length, nothing is written.
    let mut untouched = [0xAAu8; 512];
    assert!(!format.fill_sector(format.metadata_sectors(), &mut untouched));
    assert!(!format.fill_sector(0, &mut [0xAAu8; 100]));
    assert_eq!(untouched, [0xAAu8; 512]);
}

#[test]
fn a_fat32_volume_carries_fs_info_its_backups_and_a_root_cluster() {
    let format = FormatPlan::new(
        options(200 * 1024).with_file_allocation_table_type(FileAllocationTableType::Fat32),
        &mut NoClock,
    )
    .unwrap();
    let (mut zero, mut backup, mut info, mut info_backup) =
        ([0u8; 512], [0u8; 512], [0u8; 512], [0u8; 512]);
    assert!(format.fill_sector(0, &mut zero));
    assert!(format.fill_sector(6, &mut backup));
    assert!(format.fill_sector(1, &mut info));
    assert!(format.fill_sector(7, &mut info_backup));
    assert_eq!(zero, backup);
    assert_eq!(info, info_backup);
    let parsed = crate::FileSystemInformation::parse(&info, format.clusters()).unwrap();
    assert_eq!(parsed.free_cluster_count(), Some(format.clusters() - 1));
    assert_eq!(parsed.next_free_cluster(), Some(3));
    // The first table sector says media, clean, and the end of the root's chain.
    let mut table = [0u8; 512];
    assert!(format.fill_sector(32, &mut table));
    assert_eq!(
        table[..12],
        [
            0xF8, 0xFF, 0xFF, 0x0F, 0xFF, 0xFF, 0xFF, 0x0F, 0xFF, 0xFF, 0xFF, 0x0F
        ]
    );
    // The second copy begins the same way, and the sector after the first entry is zero.
    assert!(format.fill_sector(32 + format.table_sectors(), &mut table));
    assert_eq!(table[0], 0xF8);
    assert!(format.fill_sector(33, &mut table));
    assert!(table.iter().all(|b| *b == 0));
}

#[test]
fn a_label_becomes_the_boot_sectors_label_and_the_roots_first_entry() {
    let format = FormatPlan::new(
        options(64 * 1024).with_label("MY DISK").unwrap(),
        &mut NoClock,
    )
    .unwrap();
    let mut boot = [0u8; 512];
    assert!(format.fill_sector(0, &mut boot));
    assert_eq!(&boot[43..54], b"MY DISK    ");
    let root = format.geometry.reserved_sectors + 2 * format.table_sectors();
    let mut sector = [0xFFu8; 512];
    assert!(format.fill_sector(root, &mut sector));
    assert_eq!(&sector[..11], b"MY DISK    ");
    assert_eq!(sector[11], 0x08);
    // Without a label the root starts empty and the boot sector says NO NAME.
    let plain = FormatPlan::new(options(64 * 1024), &mut NoClock).unwrap();
    assert!(plain.fill_sector(root, &mut sector));
    assert!(sector.iter().all(|b| *b == 0));
}

#[test]
fn a_volume_of_65536_sectors_or_more_uses_the_32_bit_total_and_a_smaller_one_the_16_bit() {
    for (sectors, small) in [(65_535u32, true), (65_536, false)] {
        let format = FormatPlan::new(
            options(sectors).with_file_allocation_table_type(FileAllocationTableType::Fat16),
            &mut NoClock,
        )
        .unwrap();
        let mut boot = [0u8; 512];
        assert!(format.fill_sector(0, &mut boot));
        let total16 = u16::from_le_bytes([boot[19], boot[20]]);
        let total32 = u32::from_le_bytes([boot[32], boot[33], boot[34], boot[35]]);
        assert_eq!((total16 != 0, total32 != 0), (small, !small));
    }
}

struct Sink(usize);

impl crate::SectorDevice for Sink {
    type Error = usize;
    fn sector_size(&self) -> SectorSize {
        SectorSize::Bytes512
    }
    fn read_sector(&mut self, _: u64, _: &mut [u8]) -> Result<(), usize> {
        Ok(())
    }
}

impl crate::WritableSectorDevice for Sink {
    fn write_sector(&mut self, index: u64, _: &[u8]) -> Result<(), usize> {
        if index == 3 {
            return Err(3);
        }
        self.0 += 1;
        Ok(())
    }
}

#[test]
fn writing_a_format_stops_at_the_devices_first_error() {
    let format = FormatPlan::new(options(8 * 1024), &mut NoClock).unwrap();
    let mut sink = Sink(0);
    assert_eq!(format.write_to(&mut sink), Err(3));
    assert_eq!(sink.0, 3);
}

/// Microsoft's table sizing falls two entries short when the volume divides it exactly (FAT16 with
/// one-sector clusters at 8,289 sectors is one); every size from 1 to 2,000,000 sectors, in steps
/// that grow with it, must give a geometry the reader accepts.
#[test]
fn every_size_the_format_accepts_parses_back_to_the_same_volume() {
    for kind in [
        FileAllocationTableType::Fat12,
        FileAllocationTableType::Fat16,
        FileAllocationTableType::Fat32,
    ] {
        for spc in [1u32, 2, 4, 8, 16, 32, 64, 128] {
            let mut total = 1u32;
            while total < 2_000_000 {
                if let Ok(g) = Geometry::new(kind, SectorSize::Bytes512, spc, total) {
                    let mut sector = [0u8; 512];
                    g.write_boot_sector(1, &[b' '; 11], 0, &mut sector);
                    let parsed = BootSector::parse(&sector);
                    assert!(parsed.is_ok(), "{kind:?} cluster {spc} total {total}");
                    let parsed = parsed.unwrap();
                    assert_eq!(parsed.clusters(), g.clusters, "{kind:?} {spc} {total}");
                    assert_eq!(parsed.first_data_sector(), g.first_data_sector());
                }
                total += 1 + total / 500;
            }
        }
    }
}

#[test]
fn a_table_that_microsofts_formula_undersizes_grows_by_a_sector() {
    // 8,289 sectors of FAT16 with one-sector clusters: the formula gives 32 table sectors, which
    // hold 8,192 entries for 8,192 clusters plus the two reserved ones.
    let g = Geometry::new(
        FileAllocationTableType::Fat16,
        SectorSize::Bytes512,
        1,
        8289,
    )
    .unwrap();
    assert_eq!(g.table_sectors, 33);
    assert!(u64::from(g.table_sectors) * 256 >= u64::from(g.clusters) + 2);
}

#[test]
fn fat32_stops_at_the_last_cluster_the_table_can_number() {
    // With eight-sector clusters the count crosses FAT32's ceiling before a device's 2^32 sectors
    // run out. Find the largest device that still formats, by bisection (the count only grows).
    let (mut low, mut high) = (65_536u32 * 8, u32::MAX);
    while high - low > 1 {
        let middle = low + (high - low) / 2;
        match Geometry::new(
            FileAllocationTableType::Fat32,
            SectorSize::Bytes512,
            8,
            middle,
        ) {
            Ok(_) => low = middle,
            Err(_) => high = middle,
        }
    }
    let last = Geometry::new(FileAllocationTableType::Fat32, SectorSize::Bytes512, 8, low).unwrap();
    assert_eq!(last.clusters, 0x0FFF_FFF3);
    assert!(matches!(
        Geometry::new(
            FileAllocationTableType::Fat32,
            SectorSize::Bytes512,
            8,
            high
        ),
        Err(GeometryError::WrongType(_))
    ));
}

#[test]
fn a_volume_with_one_sector_to_each_table_keeps_its_label_in_the_root() {
    // With one sector to each table, the root's first sector sits right after the two tables;
    // the tables' range must end there, or the label entry lands in neither.
    let format = FormatPlan::new(options(256).with_label("SMALL").unwrap(), &mut NoClock).unwrap();
    assert_eq!(format.table_sectors(), 1);
    let root = format.geometry.reserved_sectors + 2 * format.table_sectors();
    let mut sector = [0xFFu8; 512];
    assert!(format.fill_sector(root, &mut sector));
    assert_eq!(&sector[..11], b"SMALL      ");
    assert_eq!(sector[11], 0x08);
    let mut second_table = [0u8; 512];
    assert!(format.fill_sector(root - 1, &mut second_table));
    assert_eq!(second_table[0], 0xF8);
}
