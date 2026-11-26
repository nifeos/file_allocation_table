//! **A volume opened with a buffer of one 512-byte sector reads and writes as the default does.**
//!
//! A device of 512-byte sectors needs no more than 512 bytes of buffer. With
//! `open_with_buffer`, the `Volume`, `Directory` and `FileReader` each hold that much instead of
//! 4,096 bytes, and a device whose sectors are longer than the buffer is refused before anything is
//! read.

use file_allocation_table::{
    DirectoryCluster, FormatOptions, FormatPlan, NoClock, OpenOptions, ReadError, SectorDevice,
    SectorSize, Volume, WritableSectorDevice, WritableVolume,
};

/// A volume held in memory, in sectors of `bytes_per_sector`.
struct Memory {
    bytes: Vec<u8>,
    bytes_per_sector: SectorSize,
}

impl Memory {
    fn sector(&mut self, index: u64) -> Result<&mut [u8], u64> {
        let length = self.bytes_per_sector.bytes();
        let start = usize::try_from(index).map_err(|_| index)? * length;
        self.bytes.get_mut(start..start + length).ok_or(index)
    }
}

impl SectorDevice for Memory {
    type Error = u64;

    fn sector_size(&self) -> SectorSize {
        self.bytes_per_sector
    }

    fn read_sector(&mut self, index: u64, buffer: &mut [u8]) -> Result<(), u64> {
        buffer.copy_from_slice(self.sector(index)?);
        Ok(())
    }
}

impl WritableSectorDevice for Memory {
    fn write_sector(&mut self, index: u64, buffer: &[u8]) -> Result<(), u64> {
        self.sector(index)?.copy_from_slice(buffer);
        Ok(())
    }
}

/// A formatted volume of `sectors` sectors of `bytes_per_sector`.
fn formatted(bytes_per_sector: SectorSize, sectors: u64) -> Memory {
    let plan = FormatPlan::new(FormatOptions::new(bytes_per_sector, sectors), &mut NoClock)
        .expect("a plan");
    let length = usize::try_from(sectors).expect("fits") * bytes_per_sector.bytes();
    let mut device = Memory {
        bytes: vec![0; length],
        bytes_per_sector,
    };
    plan.write_to(&mut device).expect("the format writes");
    device
}

#[test]
fn a_512_byte_buffer_writes_a_file_and_reads_it_back() {
    let device = formatted(SectorSize::Bytes512, 16 * 1024);
    let mut writer =
        WritableVolume::<_, _, 512>::open_with_buffer(device, NoClock, OpenOptions::new())
            .expect("the volume opens with a 512-byte buffer");
    let contents: Vec<u8> = (0..5_000u32).map(|value| value as u8).collect();
    let folder = writer
        .create_directory(DirectoryCluster::ROOT, "Small buffer")
        .expect("the directory is made");
    let inside = folder
        .directory_cluster()
        .expect("a directory names its cluster");
    let file = writer
        .create_file(inside, "several sectors.bin")
        .expect("the file is made");
    writer
        .open_file_writer(&file)
        .expect("the writer opens")
        .write(&contents)
        .expect("the bytes are written");

    let mut reader = Volume::<_, 512>::open_with_buffer(writer.into_device(), OpenOptions::new())
        .expect("the volume opens again");
    let entry = reader
        .lookup("small buffer/SEVERAL SECTORS.BIN")
        .expect("the file is found");
    let mut read_back = vec![0; contents.len() + 1];
    let mut file_reader = reader.open_file_reader(&entry).expect("the reader opens");
    let mut filled = 0;
    loop {
        let read = file_reader.read(&mut read_back[filled..]).expect("a read");
        if read == 0 {
            break;
        }
        filled += read;
    }
    assert_eq!(&read_back[..filled], contents.as_slice());

    let mut default = Volume::open(reader.into_device()).expect("the default buffer opens it");
    assert_eq!(
        default
            .lookup("Small buffer/several sectors.bin")
            .expect("the default buffer finds it")
            .size_bytes(),
        5_000
    );
}

#[test]
fn a_buffer_shorter_than_the_device_sector_is_refused() {
    let device = formatted(SectorSize::Bytes4096, 4 * 1024);
    let refused = Volume::<_, 512>::open_with_buffer(device, OpenOptions::new());
    assert!(matches!(
        refused,
        Err(ReadError::SectorLargerThanBuffer {
            device: SectorSize::Bytes4096,
            buffer_bytes: 512,
        })
    ));
}

#[test]
fn only_the_buffers_shrink() {
    assert_eq!(
        size_of::<Volume<Memory, 512>>() + 3_584,
        size_of::<Volume<Memory>>()
    );
    assert_eq!(
        size_of::<WritableVolume<Memory, NoClock, 1024>>() + 3_072,
        size_of::<WritableVolume<Memory, NoClock>>()
    );
}
