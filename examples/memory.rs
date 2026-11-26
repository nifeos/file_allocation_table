//! **Format a volume in memory, write a file into a directory, and read it back by path.**

use std::io::ErrorKind;

use file_allocation_table::{
    Attributes, DirectoryCluster, FormatOptions, FormatPlan, NoClock, SectorDevice, SectorSize,
    WritableSectorDevice, WritableVolume,
};

/// A volume held in memory. A real caller wraps a block device the same way.
struct Memory(Vec<u8>);

impl Memory {
    /// Sector `index`'s bytes, or the error a read past the end gets.
    fn sector(&mut self, index: u64) -> Result<&mut [u8], std::io::Error> {
        let start = usize::try_from(index).map_err(|_| ErrorKind::InvalidInput)? * 512;
        Ok(self
            .0
            .get_mut(start..start + 512)
            .ok_or(ErrorKind::UnexpectedEof)?)
    }
}

impl SectorDevice for Memory {
    type Error = std::io::Error;

    fn sector_size(&self) -> SectorSize {
        SectorSize::Bytes512
    }

    fn read_sector(&mut self, index: u64, buffer: &mut [u8]) -> Result<(), Self::Error> {
        buffer.copy_from_slice(self.sector(index)?);
        Ok(())
    }
}

impl WritableSectorDevice for Memory {
    fn write_sector(&mut self, index: u64, buffer: &[u8]) -> Result<(), Self::Error> {
        self.sector(index)?.copy_from_slice(buffer);
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn core::error::Error>> {
    // Format 8 MiB. The size makes it FAT16 with 1 KiB clusters; about 4 MiB or less makes FAT12,
    // and 512 MiB or more makes FAT32.
    let sectors = 16 * 1024;
    let options = FormatOptions::new(SectorSize::Bytes512, sectors).with_label("EXAMPLE")?;
    let format = FormatPlan::new(options, &mut NoClock)?;
    let mut device = Memory(vec![0; sectors as usize * 512]);
    format.write_to(&mut device)?;

    // Open it, make a directory and a file with a long name, and write some bytes.
    let mut volume = WritableVolume::open(device, NoClock)?;
    let docs = volume.create_directory(DirectoryCluster::ROOT, "Documents")?;
    let inside = docs
        .directory_cluster()
        .ok_or("a directory names its cluster")?;
    let file = volume.create_file(inside, "A long file name.txt")?;
    volume.open_file_writer(&file)?.write(b"hello, volume")?;
    let stored = volume.get_mut().lookup("Documents/A long file name.txt")?;
    volume.set_attributes(&stored, Attributes::ARCHIVE | Attributes::HIDDEN)?;
    volume.mark_clean()?;

    // Read it back by path, as another program would.
    let reader = volume.get_mut();
    let entry = reader.lookup("documents/A LONG FILE NAME.TXT")?;
    let mut bytes = [0u8; 32];
    let read = reader.open_file_reader(&entry)?.read(&mut bytes)?;
    assert_eq!(&bytes[..read], b"hello, volume");
    let label = reader.volume_label()?.ok_or("the volume has a label")?;
    assert_eq!(label.trim_ascii_end(), b"EXAMPLE");
    Ok(())
}
