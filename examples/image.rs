//! **Format, fill and list a FAT image file on the host.**
//!
//! A `std::fs::File` becomes a [`SectorDevice`] in a dozen lines, and everything else is the
//! crate's own API. Three commands:
//!
//! ```text
//! cargo run --example image -- format disk.img 64          # a new 64 MiB image
//! cargo run --example image -- put disk.img notes.txt      # copy a host file into the root
//! cargo run --example image -- list disk.img               # print every directory and file
//! ```
//!
//! `put` stamps the entry with the host's clock, read as UTC. The image is a bare volume, as
//! `mkfs.fat` makes on a file; for a whole disk with a partition table, open the volume through
//! [`MasterBootRecord`](file_allocation_table::MasterBootRecord) and
//! [`Partition::restrict_device`](file_allocation_table::Partition::restrict_device) instead.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use file_allocation_table::{
    CivilDateTime, Clock, DirectoryCluster, FormatOptions, FormatPlan, NoClock, SectorDevice,
    SectorSize, Volume, WritableSectorDevice, WritableVolume,
};

/// The sector size of the images this program makes and reads.
const SECTOR_BYTES: u64 = 512;

/// **An image file as a sector device**: sector `index` is the 512 bytes at `index * 512`.
struct ImageFile(File);

impl SectorDevice for ImageFile {
    type Error = io::Error;

    fn sector_size(&self) -> SectorSize {
        SectorSize::Bytes512
    }

    fn read_sector(&mut self, index: u64, buffer: &mut [u8]) -> io::Result<()> {
        self.0.seek(SeekFrom::Start(index * SECTOR_BYTES))?;
        self.0.read_exact(buffer)
    }
}

impl WritableSectorDevice for ImageFile {
    fn write_sector(&mut self, index: u64, buffer: &[u8]) -> io::Result<()> {
        self.0.seek(SeekFrom::Start(index * SECTOR_BYTES))?;
        self.0.write_all(buffer)
    }
}

/// **The host's clock, as UTC.** FAT stores no time zone; the crate expects UTC.
struct HostClock;

impl Clock for HostClock {
    fn now(&mut self) -> Option<CivilDateTime> {
        let seconds = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        let (year, month, day) = civil_from_days(i64::try_from(seconds / 86_400).ok()?);
        let of_day = seconds % 86_400;
        CivilDateTime::new(
            u16::try_from(year).ok()?,
            month,
            day,
            (of_day / 3600) as u8,
            (of_day / 60 % 60) as u8,
            (of_day % 60) as u8,
        )
        .ok()
    }
}

/// The proleptic Gregorian date `days` days after 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u8, u8) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let of_era = shifted.rem_euclid(146_097);
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let day_of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u8;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u8;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// The error every command returns: a message for the user.
type Failure = Box<dyn std::error::Error>;

/// **Make a new image of `mebibytes` MiB** at `path`, with no label and the size's default type
/// and cluster.
fn format(path: &Path, mebibytes: u32) -> Result<(), Failure> {
    let sectors = mebibytes
        .checked_mul(2048)
        .ok_or("the size does not fit a FAT volume")?;
    let plan = FormatPlan::new(
        FormatOptions::new(SectorSize::Bytes512, sectors),
        &mut NoClock,
    )?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)?;
    // The plan writes only the metadata sectors; the data area must read, so it is sized here.
    file.set_len(u64::from(sectors) * SECTOR_BYTES)?;
    plan.write_to(&mut ImageFile(file))?;
    println!(
        "{}: {mebibytes} MiB, {:?}",
        path.display(),
        plan.file_allocation_table_type()
    );
    Ok(())
}

/// **Copy the host file `source` into the root of the image at `path`**, under its own name.
fn put(path: &Path, source: &Path) -> Result<(), Failure> {
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("the source has no name in UTF-8")?;
    let bytes = std::fs::read(source)?;
    let file = OpenOptions::new().read(true).write(true).open(path)?;
    let mut volume = WritableVolume::open(ImageFile(file), HostClock)?;
    let entry = volume.create_file(DirectoryCluster::ROOT, name)?;
    {
        let mut writer = volume.open_file_writer(&entry)?;
        let mut done = 0;
        while done < bytes.len() {
            done += writer.write(&bytes[done..])?;
        }
    }
    // Set the clean-shutdown bit again and write the free-cluster hints.
    volume.mark_clean()?;
    println!("{name}: {} bytes", bytes.len());
    Ok(())
}

/// **Print every directory and file on the image at `path`**, one per line, indented by depth.
fn list(path: &Path) -> Result<(), Failure> {
    let mut volume = Volume::open(ImageFile(File::open(path)?))?;
    if let Some(label) = volume.volume_label()? {
        println!("label {}", String::from_utf8_lossy(label.trim_ascii_end()));
    }
    list_directory(&mut volume, DirectoryCluster::ROOT, 0)
}

/// Print the entries of `directory`, and the entries of each directory under it.
fn list_directory(
    volume: &mut Volume<ImageFile>,
    directory: DirectoryCluster,
    depth: usize,
) -> Result<(), Failure> {
    // Collect first: the listing borrows the volume, and a subdirectory is read with it after.
    let mut entries = Vec::new();
    for entry in volume.named_entries(directory) {
        let entry = entry?;
        if !entry.is_volume_label() && !entry.is_dot_entry() {
            entries.push(entry);
        }
    }
    for entry in entries {
        let name = match entry.long_name() {
            Some(long) => long.chars().collect::<String>(),
            None => short_name_text(entry.short_name()),
        };
        let indent = "  ".repeat(depth);
        match entry.directory_cluster() {
            Some(inner) => {
                println!("{indent}{name}/");
                list_directory(volume, inner, depth + 1)?;
            }
            None => println!(
                "{indent}{name}  {} bytes  {}",
                entry.size_bytes(),
                time_text(entry.modified())
            ),
        }
    }
    Ok(())
}

/// A stored time as `YYYY-MM-DD HH:MM:SS`, or `no time` for the zero timestamp.
fn time_text(time: Option<CivilDateTime>) -> String {
    match time {
        Some(time) => format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            time.year(),
            time.month(),
            time.day(),
            time.hour(),
            time.minute(),
            time.second()
        ),
        None => "no time".into(),
    }
}

/// An 8.3 name's 11 stored bytes as text: the base, then a dot and the extension when it has one.
fn short_name_text(name: [u8; 11]) -> String {
    let base = String::from_utf8_lossy(name[..8].trim_ascii_end());
    let extension = String::from_utf8_lossy(name[8..].trim_ascii_end());
    if extension.is_empty() {
        base.into_owned()
    } else {
        format!("{base}.{extension}")
    }
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let result = match arguments.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["format", path, mebibytes] => match mebibytes.parse() {
            Ok(mebibytes) => format(Path::new(path), mebibytes),
            Err(error) => Err(error.into()),
        },
        ["put", path, source] => put(Path::new(path), Path::new(source)),
        ["list", path] => list(Path::new(path)),
        _ => Err("usage: image format IMAGE MEBIBYTES | put IMAGE FILE | list IMAGE".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("image: {error}");
            ExitCode::FAILURE
        }
    }
}
