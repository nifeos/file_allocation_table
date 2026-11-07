//! **Every public error displays and is a `core::error::Error`.**
//!
//! Each variant has a message of its own: not empty, in lower case, with no closing period, and
//! unlike every other variant of its type. A wrapping variant returns what it wraps as its
//! source, and a caller's `?` takes any of them into a boxed error.

use std::error::Error;
use std::fmt;

use file_allocation_table::{
    ChainError, FileOpenError, FileReadError, FileSystemInformationError, FormatError, LayoutError,
    LongFileNameError, LookupError, MasterBootRecordError, MasterBootRecordReadError, ParseError,
    PartitionError, ReadError, SectorSize, TableEntry, WriteError,
};

/// A device error that is a `std::error::Error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeviceGone;

impl fmt::Display for DeviceGone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the device is gone")
    }
}

impl Error for DeviceGone {}

/// Check every message of one type, and return them.
fn messages<T: fmt::Display>(errors: &[T]) -> Vec<String> {
    let messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
    for message in &messages {
        assert!(!message.is_empty());
        assert!(!message.ends_with('.'), "{message}");
        let first = message.chars().next().unwrap();
        assert!(!first.is_uppercase(), "{message}");
    }
    let mut distinct = messages.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), messages.len(), "{messages:?}");
    messages
}

/// The message of `error`'s source, if it has one.
fn source_message(error: &dyn Error) -> Option<String> {
    error.source().map(ToString::to_string)
}

#[test]
fn every_parse_and_format_error_has_its_own_message() {
    messages(&[
        ParseError::TooShort,
        ParseError::NoSignature,
        ParseError::NoJump,
        ParseError::SectorSize,
        ParseError::ClusterSize,
        ParseError::NoReservedSectors,
        ParseError::NoTables,
        ParseError::Media,
        ParseError::NoSectors,
        ParseError::NoTableSectors,
        ParseError::MetadataPastEnd,
        ParseError::NoClusters,
        ParseError::LayoutContradictsClusterCount,
        ParseError::TooManyClusters,
        ParseError::TableTooSmall,
        ParseError::RootCluster,
        ParseError::Version,
        ParseError::ActiveTable,
    ]);
    messages(&[
        FileSystemInformationError::TooShort,
        FileSystemInformationError::NoSignature,
    ]);
    messages(&[
        FormatError::TooSmall,
        FormatError::NoSuchGeometry,
        FormatError::LabelRefused,
        FormatError::TimeOutOfRange,
        FormatError::ClusterSizeRefused,
    ]);
    messages(&[
        LayoutError::NotFat32,
        LayoutError::TooLarge,
        LayoutError::NameRefused,
        LayoutError::LabelRefused,
        LayoutError::TimeOutOfRange,
        LayoutError::FileTooBig,
    ]);
    messages(&[
        LongFileNameError::BadStart,
        LongFileNameError::OutOfOrder,
        LongFileNameError::PiecesDisagree,
        LongFileNameError::Incomplete,
        LongFileNameError::ChecksumMismatch,
        LongFileNameError::BadPadding,
    ]);
    messages(&[FileOpenError::IsADirectory, FileOpenError::IsAVolumeLabel]);
    let partitions = messages(&[
        MasterBootRecordError::TooShort,
        MasterBootRecordError::NoSignature,
        MasterBootRecordError::BadStatus { index: 1 },
        MasterBootRecordError::EmptyPartition { index: 2 },
        MasterBootRecordError::PartitionOverflows { index: 3 },
    ]);
    assert_eq!(partitions[2], "partition 1 has a reserved status byte");
    assert_eq!(partitions[3], "partition 2 has a type but no sectors");
    assert_eq!(partitions[4], "partition 3 ends past sector 2^32");
}

#[test]
fn every_device_level_error_has_its_own_message_and_its_source() {
    let device = ChainError::Device(DeviceGone);
    let chains = messages(&[
        device,
        ChainError::NotADataCluster(7),
        ChainError::Broken {
            cluster: 9,
            entry: TableEntry::Free,
        },
        ChainError::Broken {
            cluster: 9,
            entry: TableEntry::Reserved,
        },
        ChainError::Broken {
            cluster: 9,
            entry: TableEntry::Next(400),
        },
        ChainError::Broken {
            cluster: 9,
            entry: TableEntry::Bad,
        },
        ChainError::Broken {
            cluster: 9,
            entry: TableEntry::EndOfChain,
        },
        ChainError::TooLong,
    ]);
    assert_eq!(chains[1], "cluster 7 is not a data cluster of this volume");
    assert_eq!(
        chains[5],
        "the chain is broken at cluster 9, whose table entry is bad"
    );
    assert_eq!(
        source_message(&device).as_deref(),
        Some("the device is gone")
    );
    assert!(ChainError::<DeviceGone>::TooLong.source().is_none());

    let reads = messages(&[
        ReadError::Device(DeviceGone),
        ReadError::Refused(ParseError::NoJump),
        ReadError::SectorSizeMismatch {
            device: SectorSize::Bytes512,
            volume: SectorSize::Bytes4096,
        },
    ]);
    assert_eq!(
        reads[2],
        "the device has 512-byte sectors and the volume 4096-byte sectors"
    );
    assert_eq!(
        source_message(&ReadError::Device(DeviceGone)).as_deref(),
        Some("the device is gone")
    );
    assert_eq!(
        source_message(&ReadError::<DeviceGone>::Refused(ParseError::NoJump)),
        Some(ParseError::NoJump.to_string())
    );
    let mismatch = ReadError::<DeviceGone>::SectorSizeMismatch {
        device: SectorSize::Bytes512,
        volume: SectorSize::Bytes512,
    };
    assert!(mismatch.source().is_none());

    messages(&[
        MasterBootRecordReadError::Device(DeviceGone),
        MasterBootRecordReadError::Parse(MasterBootRecordError::TooShort),
    ]);
    assert_eq!(
        source_message(&MasterBootRecordReadError::Device(DeviceGone)).as_deref(),
        Some("the device is gone")
    );
    assert_eq!(
        source_message(&MasterBootRecordReadError::<DeviceGone>::Parse(
            MasterBootRecordError::TooShort
        )),
        Some(MasterBootRecordError::TooShort.to_string())
    );

    let partitions = messages(&[
        PartitionError::Device(DeviceGone),
        PartitionError::OutsidePartition(12),
    ]);
    assert_eq!(
        partitions[1],
        "sector 12 is past the partition's last sector"
    );
    assert_eq!(
        source_message(&PartitionError::Device(DeviceGone)).as_deref(),
        Some("the device is gone")
    );
    assert!(
        PartitionError::<DeviceGone>::OutsidePartition(12)
            .source()
            .is_none()
    );
}

#[test]
fn every_file_level_error_has_its_own_message_and_its_source() {
    let chain = ChainError::Device(DeviceGone);
    let wrapped: [&dyn Error; 3] = [
        &FileReadError::Chain(chain),
        &LookupError::Chain(chain),
        &WriteError::Chain(chain),
    ];
    for error in wrapped {
        assert_eq!(source_message(error), Some(chain.to_string()));
        let inner = error.source().unwrap();
        assert_eq!(source_message(inner).as_deref(), Some("the device is gone"));
    }
    messages(&[
        FileReadError::Chain(chain),
        FileReadError::ShorterThanItsSize,
        FileReadError::Failed,
    ]);
    assert!(FileReadError::<DeviceGone>::Failed.source().is_none());
    messages(&[
        LookupError::Chain(chain),
        LookupError::NotFound,
        LookupError::NotADirectory,
        LookupError::RootHasNoEntry,
    ]);
    assert!(LookupError::<DeviceGone>::NotFound.source().is_none());
    messages(&[
        WriteError::Chain(chain),
        WriteError::VolumeFull,
        WriteError::FileTooLarge,
        WriteError::ShorterThanItsSize,
        WriteError::EntryChanged,
        WriteError::Failed,
        WriteError::DirectoryFull,
        WriteError::NameRefused,
        WriteError::LabelRefused,
        WriteError::AlreadyExists,
        WriteError::NotAFile,
        WriteError::NotADirectory,
        WriteError::ReadOnly,
        WriteError::TimeOutOfRange,
        WriteError::DirectoryNotEmpty,
        WriteError::MovedIntoItself,
    ]);
    assert!(WriteError::<DeviceGone>::VolumeFull.source().is_none());
}

#[test]
fn a_question_mark_takes_any_of_them_into_a_boxed_error() {
    fn refused() -> Result<(), Box<dyn Error>> {
        Err(WriteError::Chain(ChainError::Device(DeviceGone)))?;
        Ok(())
    }
    let error = refused().unwrap_err();
    let mut chain = vec![error.to_string()];
    let mut next = error.source();
    while let Some(inner) = next {
        chain.push(inner.to_string());
        next = inner.source();
    }
    assert_eq!(chain.len(), 3);
    assert_eq!(chain[2], "the device is gone");
}
