//! The seek arithmetic at its edges, and the kind every error variant reports.

use core::fmt;

use embedded_io::Error as _;

use super::*;
use crate::TableEntry;

/// A device error that is a `core::error::Error`, as the traits need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeviceGone;

impl fmt::Display for DeviceGone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the device is gone")
    }
}

impl core::error::Error for DeviceGone {}

#[test]
fn a_seek_from_the_start_takes_the_position_as_given_up_to_the_largest_u32() {
    assert_eq!(seek_target(SeekFrom::Start(0), 7, 100), Some(0));
    assert_eq!(seek_target(SeekFrom::Start(40), 7, 100), Some(40));
    assert_eq!(seek_target(SeekFrom::Start(500), 7, 100), Some(500));
    assert_eq!(
        seek_target(SeekFrom::Start(u64::from(u32::MAX)), 7, 100),
        Some(u32::MAX)
    );
    assert_eq!(
        seek_target(SeekFrom::Start(u64::from(u32::MAX) + 1), 7, 100),
        Some(u32::MAX)
    );
    assert_eq!(
        seek_target(SeekFrom::Start(u64::MAX), 7, 100),
        Some(u32::MAX)
    );
}

#[test]
fn a_seek_from_the_end_counts_from_the_size() {
    assert_eq!(seek_target(SeekFrom::End(0), 7, 100), Some(100));
    assert_eq!(seek_target(SeekFrom::End(-100), 7, 100), Some(0));
    assert_eq!(seek_target(SeekFrom::End(-1), 7, 100), Some(99));
    assert_eq!(seek_target(SeekFrom::End(5), 7, 100), Some(105));
    assert_eq!(seek_target(SeekFrom::End(-101), 7, 100), None);
    assert_eq!(seek_target(SeekFrom::End(i64::MIN), 7, 100), None);
    assert_eq!(seek_target(SeekFrom::End(i64::MAX), 7, 100), Some(u32::MAX));
}

#[test]
fn a_seek_from_the_current_position_counts_from_it() {
    assert_eq!(seek_target(SeekFrom::Current(0), 7, 100), Some(7));
    assert_eq!(seek_target(SeekFrom::Current(3), 7, 100), Some(10));
    assert_eq!(seek_target(SeekFrom::Current(-7), 7, 100), Some(0));
    assert_eq!(seek_target(SeekFrom::Current(-8), 7, 100), None);
    assert_eq!(
        seek_target(SeekFrom::Current(i64::from(u32::MAX)), 7, 100),
        Some(u32::MAX)
    );
    assert_eq!(
        seek_target(SeekFrom::Current(-1), u32::MAX, u32::MAX),
        Some(u32::MAX - 1)
    );
}

#[test]
fn a_chain_error_is_damaged_data_unless_the_device_failed() {
    let device = ChainError::Device(DeviceGone);
    let damaged: [ChainError<DeviceGone>; 3] = [
        ChainError::NotADataCluster(1),
        ChainError::Broken {
            cluster: 2,
            entry: TableEntry::Free,
        },
        ChainError::TooLong,
    ];
    assert_eq!(FileReadError::Chain(device).kind(), ErrorKind::Other);
    assert_eq!(WriteError::Chain(device).kind(), ErrorKind::Other);
    for chain in damaged {
        assert_eq!(FileReadError::Chain(chain).kind(), ErrorKind::InvalidData);
        assert_eq!(WriteError::Chain(chain).kind(), ErrorKind::InvalidData);
    }
}

#[test]
fn every_read_error_reports_its_kind() {
    let kinds: [(FileReadError<DeviceGone>, ErrorKind); 3] = [
        (FileReadError::ShorterThanItsSize, ErrorKind::InvalidData),
        (FileReadError::Failed, ErrorKind::Other),
        (FileReadError::SeekBeforeStart, ErrorKind::InvalidInput),
    ];
    for (error, kind) in kinds {
        assert_eq!(error.kind(), kind, "{error:?}");
    }
}

#[test]
fn every_write_error_reports_its_kind() {
    let kinds: [(WriteError<DeviceGone>, ErrorKind); 16] = [
        (WriteError::VolumeFull, ErrorKind::WriteZero),
        (WriteError::FileTooLarge, ErrorKind::WriteZero),
        (WriteError::ShorterThanItsSize, ErrorKind::InvalidData),
        (WriteError::TimeOutOfRange, ErrorKind::InvalidData),
        (WriteError::EntryChanged, ErrorKind::NotFound),
        (WriteError::AlreadyExists, ErrorKind::AlreadyExists),
        (WriteError::ReadOnly, ErrorKind::PermissionDenied),
        (WriteError::NameRefused, ErrorKind::InvalidInput),
        (WriteError::LabelRefused, ErrorKind::InvalidInput),
        (WriteError::NotAFile, ErrorKind::InvalidInput),
        (WriteError::NotADirectory, ErrorKind::InvalidInput),
        (WriteError::MovedIntoItself, ErrorKind::InvalidInput),
        (WriteError::SeekBeforeStart, ErrorKind::InvalidInput),
        (WriteError::Failed, ErrorKind::Other),
        (WriteError::DirectoryFull, ErrorKind::Other),
        (WriteError::DirectoryNotEmpty, ErrorKind::Other),
    ];
    for (error, kind) in kinds {
        assert_eq!(error.kind(), kind, "{error:?}");
    }
}
