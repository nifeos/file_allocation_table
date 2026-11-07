//! **`Display` and `core::error::Error` for every public error type.**
//!
//! Each message describes its own level only, in lower case with no closing period, as the
//! standard library's errors do. An error that wraps another (a device error, a parse error, a
//! chain error) does not repeat the inner message: [`core::error::Error::source`] returns the inner
//! error, so a caller that walks the chain prints each level once. A type generic over the device's
//! error `E` displays for any `E`, and is an `Error` when `E` is one.

use core::error::Error;
use core::fmt;

use crate::{
    ChainError, FileOpenError, FileReadError, FileSystemInformationError, FormatError, LayoutError,
    LongFileNameError, LookupError, MasterBootRecordError, MasterBootRecordReadError, ParseError,
    PartitionError, ReadError, TableEntry, WriteError,
};

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ParseError::TooShort => "the boot sector buffer holds fewer than 512 bytes",
            ParseError::NoSignature => "the boot sector does not end in 0x55 0xAA",
            ParseError::NoJump => "the boot sector does not start with a jump instruction",
            ParseError::SectorSize => "the sector size is not 512, 1024, 2048 or 4096 bytes",
            ParseError::ClusterSize => {
                "the cluster size is not a power of two from 1 to 128 sectors"
            }
            ParseError::NoReservedSectors => "the volume has no reserved sectors",
            ParseError::NoTables => "the volume has no file allocation table",
            ParseError::Media => "the media byte is not 0xF0 or 0xF8 to 0xFF",
            ParseError::NoSectors => "the volume states no sectors",
            ParseError::NoTableSectors => "the file allocation table has no sectors",
            ParseError::MetadataPastEnd => {
                "the reserved sectors, tables and root directory end past the volume"
            }
            ParseError::NoClusters => "the data region holds no whole cluster",
            ParseError::LayoutContradictsClusterCount => {
                "the boot sector's layout is for one FAT type and its cluster count makes another"
            }
            ParseError::TooManyClusters => "the volume has more clusters than FAT32 can number",
            ParseError::TableTooSmall => "a file allocation table cannot hold every cluster",
            ParseError::RootCluster => "the FAT32 root directory cluster is not a data cluster",
            ParseError::Version => "the FAT32 version is not 0.0",
            ParseError::ActiveTable => "the FAT32 active table is one the volume does not have",
        })
    }
}

impl Error for ParseError {}

impl<E> fmt::Display for ReadError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::Device(_) => f.write_str("the device refused to read the boot sector"),
            ReadError::Refused(_) => f.write_str("sector 0 is not a boot sector this crate reads"),
            ReadError::SectorSizeMismatch { device, volume } => write!(
                f,
                "the device has {}-byte sectors and the volume {}-byte sectors",
                device.bytes(),
                volume.bytes()
            ),
        }
    }
}

impl<E: Error + 'static> Error for ReadError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ReadError::Device(error) => Some(error),
            ReadError::Refused(error) => Some(error),
            ReadError::SectorSizeMismatch { .. } => None,
        }
    }
}

impl<E> fmt::Display for ChainError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChainError::Device(_) => f.write_str("the device refused a sector read or write"),
            ChainError::NotADataCluster(cluster) => {
                write!(f, "cluster {cluster} is not a data cluster of this volume")
            }
            ChainError::Broken { cluster, entry } => {
                let entry = match entry {
                    TableEntry::Free => "free",
                    TableEntry::Reserved => "reserved",
                    TableEntry::Next(_) => "a cluster the volume does not have",
                    TableEntry::Bad => "bad",
                    TableEntry::EndOfChain => "the end of a chain",
                };
                write!(
                    f,
                    "the chain is broken at cluster {cluster}, whose table entry is {entry}"
                )
            }
            ChainError::TooLong => {
                f.write_str("the chain has more clusters than the volume, so the table loops")
            }
        }
    }
}

impl<E: Error + 'static> Error for ChainError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ChainError::Device(error) => Some(error),
            _ => None,
        }
    }
}

impl fmt::Display for FileOpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            FileOpenError::IsADirectory => "the entry is a directory, not a file",
            FileOpenError::IsAVolumeLabel => "the entry is the volume label, not a file",
        })
    }
}

impl Error for FileOpenError {}

impl<E> fmt::Display for FileReadError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            FileReadError::Chain(_) => "the file's chain could not be followed",
            FileReadError::ShorterThanItsSize => "the file's chain ends before its size",
            FileReadError::Failed => "an earlier read failed, so this reader reads nothing more",
        })
    }
}

impl<E: Error + 'static> Error for FileReadError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            FileReadError::Chain(error) => Some(error),
            _ => None,
        }
    }
}

impl fmt::Display for FileSystemInformationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            FileSystemInformationError::TooShort => {
                "the file system information sector buffer holds fewer than 512 bytes"
            }
            FileSystemInformationError::NoSignature => {
                "the file system information sector lacks one of its three signatures"
            }
        })
    }
}

impl Error for FileSystemInformationError {}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            FormatError::TooSmall => "the device is too small for a volume",
            FormatError::NoSuchGeometry => {
                "no cluster size gives that FAT type a legal cluster count"
            }
            FormatError::LabelRefused => "the volume label is not a legal short name",
            FormatError::TimeOutOfRange => "the clock reported a time FAT cannot store",
            FormatError::ClusterSizeRefused => {
                "the cluster size is not a power of two from 1 to 128 sectors"
            }
        })
    }
}

impl Error for FormatError {}

impl fmt::Display for LongFileNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LongFileNameError::BadStart => {
                "the long name's run has no first piece, or its ordinal is 0 or above 20"
            }
            LongFileNameError::OutOfOrder => "the long name's pieces are out of order",
            LongFileNameError::PiecesDisagree => "the long name's pieces carry different checksums",
            LongFileNameError::Incomplete => "the long name's run ends before ordinal 1",
            LongFileNameError::ChecksumMismatch => {
                "the long name's checksum is not its short entry's"
            }
            LongFileNameError::BadPadding => "the long name's length or padding is wrong",
        })
    }
}

impl Error for LongFileNameError {}

impl<E> fmt::Display for LookupError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LookupError::Chain(_) => "a directory on the path could not be read",
            LookupError::NotFound => "no entry has that name",
            LookupError::NotADirectory => "a component before the last names a file",
            LookupError::RootHasNoEntry => "the path names the root, which has no entry",
        })
    }
}

impl<E: Error + 'static> Error for LookupError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            LookupError::Chain(error) => Some(error),
            _ => None,
        }
    }
}

impl fmt::Display for MasterBootRecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MasterBootRecordError::TooShort => {
                f.write_str("the master boot record buffer holds fewer than 512 bytes")
            }
            MasterBootRecordError::NoSignature => {
                f.write_str("the master boot record does not end in 0x55 0xAA")
            }
            MasterBootRecordError::BadStatus { index } => {
                write!(f, "partition {index} has a reserved status byte")
            }
            MasterBootRecordError::EmptyPartition { index } => {
                write!(f, "partition {index} has a type but no sectors")
            }
            MasterBootRecordError::PartitionOverflows { index } => {
                write!(f, "partition {index} ends past sector 2^32")
            }
        }
    }
}

impl Error for MasterBootRecordError {}

impl<E> fmt::Display for MasterBootRecordReadError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            MasterBootRecordReadError::Device(_) => "the device refused to read sector 0",
            MasterBootRecordReadError::Parse(_) => "sector 0 is not a partition table",
        })
    }
}

impl<E: Error + 'static> Error for MasterBootRecordReadError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            MasterBootRecordReadError::Device(error) => Some(error),
            MasterBootRecordReadError::Parse(error) => Some(error),
        }
    }
}

impl<E> fmt::Display for PartitionError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PartitionError::Device(_) => f.write_str("the device refused the read"),
            PartitionError::OutsidePartition(sector) => {
                write!(f, "sector {sector} is past the partition's last sector")
            }
        }
    }
}

impl<E: Error + 'static> Error for PartitionError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            PartitionError::Device(error) => Some(error),
            PartitionError::OutsidePartition(_) => None,
        }
    }
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LayoutError::NotFat32 => "the partition has too few clusters to be FAT32",
            LayoutError::TooLarge => "the partition is larger than FAT32 addresses",
            LayoutError::NameRefused => "the file name is neither a short name nor a long name",
            LayoutError::LabelRefused => "the volume label is not a legal short name",
            LayoutError::TimeOutOfRange => "the clock reported a time FAT cannot store",
            LayoutError::FileTooBig => "the file does not fit in the volume",
        })
    }
}

impl Error for LayoutError {}

impl<E> fmt::Display for WriteError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            WriteError::Chain(_) => "a chain could not be followed or a sector read or written",
            WriteError::VolumeFull => "every data cluster is in use",
            WriteError::FileTooLarge => "the file would pass 4 GiB less one byte",
            WriteError::ShorterThanItsSize => "the file's chain ends before its size",
            WriteError::EntryChanged => {
                "the slot no longer holds the entry; look the entry up again"
            }
            WriteError::Failed => "an earlier write failed, so this writer writes nothing more",
            WriteError::DirectoryFull => "the directory has no free slot and cannot grow",
            WriteError::NameRefused => "the name is neither a short name nor a long name",
            WriteError::LabelRefused => "the volume label is not a legal short name",
            WriteError::AlreadyExists => "the directory already holds an entry of that name",
            WriteError::NotAFile => "the entry is a directory or the volume label, not a file",
            WriteError::NotADirectory => "the entry is not a directory that can be removed",
            WriteError::ReadOnly => "the file is read-only",
            WriteError::TimeOutOfRange => "the clock reported a time FAT cannot store",
            WriteError::DirectoryNotEmpty => "the directory is not empty",
            WriteError::MovedIntoItself => "a directory cannot move into itself or below itself",
        })
    }
}

impl<E: Error + 'static> Error for WriteError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            WriteError::Chain(error) => Some(error),
            _ => None,
        }
    }
}
