//! **The `embedded-io` 0.7 traits behave as the inherent methods do.** With the `embedded-io-07`
//! feature on, a file written through `Write` and read through `Read` holds the same bytes the
//! inherent calls would, a seek stops at the end of the file and refuses a position before its
//! start, and a full volume reports `WriteZero`. The README's example runs here too, since the
//! README's documentation tests build without the feature.

#![cfg(feature = "embedded-io-07")]

use embedded_io::{Error as _, ErrorKind, Read, Seek, SeekFrom, Write};
use file_allocation_table::{
    DirectoryCluster, FileReadError, FormatOptions, FormatPlan, NoClock, SectorDevice, SectorSize,
    WritableSectorDevice, WritableVolume, WriteError,
};

/// A device error that is a `core::error::Error`, as `embedded_io::Error` requires.
#[derive(Debug)]
struct OutOfRange;

impl core::fmt::Display for OutOfRange {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("the sector is past the end of the volume")
    }
}

impl core::error::Error for OutOfRange {}

/// A volume held in memory.
struct Memory(Vec<u8>);

impl SectorDevice for Memory {
    type Error = OutOfRange;

    fn sector_size(&self) -> SectorSize {
        SectorSize::Bytes512
    }

    fn read_sector(&mut self, index: u64, buffer: &mut [u8]) -> Result<(), OutOfRange> {
        let start = usize::try_from(index).map_err(|_| OutOfRange)? * 512;
        buffer.copy_from_slice(self.0.get(start..start + 512).ok_or(OutOfRange)?);
        Ok(())
    }
}

impl WritableSectorDevice for Memory {
    fn write_sector(&mut self, index: u64, buffer: &[u8]) -> Result<(), OutOfRange> {
        let start = usize::try_from(index).map_err(|_| OutOfRange)? * 512;
        let sector = self.0.get_mut(start..start + 512).ok_or(OutOfRange)?;
        sector.copy_from_slice(buffer);
        Ok(())
    }
}

// The README example, from here to the line that ends it. `the_readme_carries_this_example`
// holds the README's copy to it.
// README example start
/// Any `embedded-io` writer, such as a file on this crate's volume.
fn greet(out: &mut impl embedded_io::Write) -> Result<(), impl embedded_io::Error> {
    out.write_all(b"hello, ")?;
    out.write_all(b"embedded-io")
}

fn readme_example() {
    let sectors = 16 * 1024;
    let options = FormatOptions::new(SectorSize::Bytes512, sectors);
    let mut device = Memory(vec![0; sectors as usize * 512]);
    let format = FormatPlan::new(options, &mut NoClock).unwrap();
    format.write_to(&mut device).unwrap();
    let mut volume = WritableVolume::open(device, NoClock).unwrap();
    let file = volume
        .create_file(DirectoryCluster::ROOT, "GREETING.TXT")
        .unwrap();

    greet(&mut volume.open_file_writer(&file).unwrap()).unwrap();

    let entry = volume.lookup("GREETING.TXT").unwrap();
    let mut reader = volume.open_file_reader(&entry).unwrap();
    reader.seek(SeekFrom::End(-11)).unwrap();
    let mut bytes = [0u8; 11];
    reader.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"embedded-io");
}
// README example end

#[test]
fn the_readme_example_runs() {
    readme_example();
}

#[test]
fn the_readme_carries_this_example() {
    let source = include_str!("the_embedded_io_traits_read_write_and_seek.rs");
    let (_, rest) = source.split_once("// README example start\n").unwrap();
    let (example, _) = rest.split_once("// README example end\n").unwrap();
    let example = example.replace("fn readme_example()", "fn main()");
    let readme = include_str!("../README.md");
    assert!(
        readme.contains(&example),
        "the README's embedded-io example differs from this test's copy:\n{example}"
    );
}

/// A formatted volume of `sectors` sectors, open for writing, with one empty file `NAME.BIN`.
fn volume_with_file(sectors: u32) -> WritableVolume<Memory, NoClock> {
    let options = FormatOptions::new(SectorSize::Bytes512, sectors);
    let mut device = Memory(vec![0; sectors as usize * 512]);
    FormatPlan::new(options, &mut NoClock)
        .unwrap()
        .write_to(&mut device)
        .unwrap();
    let mut volume = WritableVolume::open(device, NoClock).unwrap();
    volume
        .create_file(DirectoryCluster::ROOT, "NAME.BIN")
        .unwrap();
    volume
}

/// The byte at `position` of the test files.
fn byte(position: usize) -> u8 {
    (position * 31 + position / 509) as u8
}

#[test]
fn bytes_written_through_write_read_back_through_read_and_seek() {
    let mut volume = volume_with_file(16 * 1024);
    let data: Vec<u8> = (0..5000).map(byte).collect();
    let entry = volume.lookup("NAME.BIN").unwrap();
    {
        let mut writer = volume.open_file_writer(&entry).unwrap();
        // Odd pieces, so writes straddle sectors and clusters.
        for piece in data.chunks(333) {
            Write::write_all(&mut writer, piece).unwrap();
        }
        Write::flush(&mut writer).unwrap();
        assert_eq!(writer.stream_position().unwrap(), 5000);
        // Overwrite the middle, then append, through seeks.
        assert_eq!(writer.seek(SeekFrom::Start(1000)).unwrap(), 1000);
        Write::write_all(&mut writer, &[0xAA; 10]).unwrap();
        assert_eq!(writer.seek(SeekFrom::Current(-10)).unwrap(), 1000);
        assert_eq!(writer.seek(SeekFrom::End(0)).unwrap(), 5000);
        Write::write_all(&mut writer, &[0xBB; 3]).unwrap();
        assert_eq!(writer.size_bytes(), 5003);
    }
    let mut expected = data.clone();
    expected[1000..1010].fill(0xAA);
    expected.extend_from_slice(&[0xBB; 3]);

    let entry = volume.lookup("NAME.BIN").unwrap();
    let mut reader = volume.open_file_reader(&entry).unwrap();
    let mut whole = vec![0u8; 5003];
    Read::read_exact(&mut reader, &mut whole).unwrap();
    assert_eq!(whole, expected);
    // At the end, a read returns 0, and `read_exact` reports the end.
    assert_eq!(Read::read(&mut reader, &mut [0u8; 4]).unwrap(), 0);
    assert!(matches!(
        Read::read_exact(&mut reader, &mut [0u8; 1]),
        Err(embedded_io::ReadExactError::UnexpectedEof)
    ));
    // A backwards seek walks the chain again from its start.
    assert_eq!(reader.seek(SeekFrom::Start(4000)).unwrap(), 4000);
    assert_eq!(reader.seek(SeekFrom::Current(-3999)).unwrap(), 1);
    let mut some = [0u8; 1500];
    Read::read_exact(&mut reader, &mut some).unwrap();
    assert_eq!(some, expected[1..1501]);
    reader.rewind().unwrap();
    assert_eq!(reader.position(), 0);
}

#[test]
fn a_seek_past_the_end_stops_at_the_end() {
    let mut volume = volume_with_file(16 * 1024);
    let entry = volume.lookup("NAME.BIN").unwrap();
    {
        let mut writer = volume.open_file_writer(&entry).unwrap();
        Write::write_all(&mut writer, &[7; 100]).unwrap();
        assert_eq!(writer.seek(SeekFrom::Start(1 << 40)).unwrap(), 100);
        assert_eq!(writer.seek(SeekFrom::End(50)).unwrap(), 100);
        assert_eq!(writer.seek(SeekFrom::Current(i64::MAX)).unwrap(), 100);
    }
    let entry = volume.lookup("NAME.BIN").unwrap();
    let mut reader = volume.open_file_reader(&entry).unwrap();
    assert_eq!(reader.seek(SeekFrom::Start(u64::MAX)).unwrap(), 100);
    assert_eq!(reader.seek(SeekFrom::End(1)).unwrap(), 100);
    assert_eq!(reader.seek(SeekFrom::Current(1)).unwrap(), 100);
}

#[test]
fn a_seek_before_the_start_is_refused_and_moves_nothing() {
    let mut volume = volume_with_file(16 * 1024);
    let entry = volume.lookup("NAME.BIN").unwrap();
    {
        let mut writer = volume.open_file_writer(&entry).unwrap();
        Write::write_all(&mut writer, &[1, 2, 3, 4, 5]).unwrap();
        writer.seek(SeekFrom::Start(2)).unwrap();
        let refused = writer.seek(SeekFrom::Current(-3)).unwrap_err();
        assert!(matches!(refused, WriteError::SeekBeforeStart));
        assert_eq!(refused.kind(), ErrorKind::InvalidInput);
        assert!(matches!(
            writer.seek(SeekFrom::End(-6)),
            Err(WriteError::SeekBeforeStart)
        ));
        assert_eq!(writer.position(), 2);
        // The writer is still usable.
        Write::write_all(&mut writer, &[9]).unwrap();
    }
    let entry = volume.lookup("NAME.BIN").unwrap();
    let mut reader = volume.open_file_reader(&entry).unwrap();
    reader.seek(SeekFrom::Start(4)).unwrap();
    let refused = reader.seek(SeekFrom::Current(-5)).unwrap_err();
    assert!(matches!(refused, FileReadError::SeekBeforeStart));
    assert_eq!(refused.kind(), ErrorKind::InvalidInput);
    assert!(matches!(
        reader.seek(SeekFrom::End(-6)),
        Err(FileReadError::SeekBeforeStart)
    ));
    assert_eq!(reader.position(), 4);
    reader.rewind().unwrap();
    let mut bytes = [0u8; 5];
    Read::read_exact(&mut reader, &mut bytes).unwrap();
    assert_eq!(bytes, [1, 2, 9, 4, 5]);
}

#[test]
fn a_full_volume_reports_write_zero() {
    // The smallest FAT12 volume the format lays out leaves few clusters, so a long write fills it.
    let mut volume = volume_with_file(128);
    let entry = volume.lookup("NAME.BIN").unwrap();
    let mut writer = volume.open_file_writer(&entry).unwrap();
    let error = Write::write_all(&mut writer, &[0x5A; 128 * 512]).unwrap_err();
    assert!(matches!(error, WriteError::VolumeFull), "{error:?}");
    assert_eq!(error.kind(), ErrorKind::WriteZero);
    assert!(writer.size_bytes() > 0);
}

#[test]
fn a_device_error_reports_other_and_ends_the_reader() {
    let mut volume = volume_with_file(16 * 1024);
    let entry = volume.lookup("NAME.BIN").unwrap();
    Write::write_all(
        &mut volume.open_file_writer(&entry).unwrap(),
        &[3; 64 * 1024],
    )
    .unwrap();
    let entry = volume.lookup("NAME.BIN").unwrap();
    // Cut the device short, past the directory and the start of the file but before its end.
    let mut device = volume.into_device();
    device.0.truncate(150 * 512);
    let mut volume = WritableVolume::open(device, NoClock).unwrap();
    let mut reader = volume.open_file_reader(&entry).unwrap();
    Read::read_exact(&mut reader, &mut [0u8; 512]).unwrap();
    reader.seek(SeekFrom::Start(60 * 1024)).unwrap();
    let error = Read::read(&mut reader, &mut [0u8; 512]).unwrap_err();
    assert!(matches!(error, FileReadError::Chain(_)), "{error:?}");
    assert_eq!(error.kind(), ErrorKind::Other);
    let after = Read::read(&mut reader, &mut [0u8; 512]).unwrap_err();
    assert!(matches!(after, FileReadError::Failed), "{after:?}");
    assert_eq!(after.kind(), ErrorKind::Other);
}
