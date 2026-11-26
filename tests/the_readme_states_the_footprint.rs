//! **The README's Footprint section states the sizes the types have.**
//!
//! Each figure the section gives, for a device and a clock of no size on a 64-bit host, is checked
//! here against `size_of`, and each sentence that states one is checked to be in the README, so the
//! two cannot drift apart.

#![cfg(target_pointer_width = "64")]

use file_allocation_table::{
    Directory, FileReader, FileWriter, FormatPlan, NamedEntry, NoClock, Volume, VolumeLayout,
    WritableVolume,
};

/// A device of no size, so each figure is the crate's own bytes.
type Device = ();

/// Each README sentence, with the size it states.
fn stated() -> [(&'static str, usize, usize); 11] {
    [
        (
            "A `Volume` or a `WritableVolume` is 4,216 bytes plus the device",
            4_216,
            size_of::<Volume<Device>>(),
        ),
        (
            "A `Volume` or a `WritableVolume` is 4,216 bytes plus the device",
            4_216,
            size_of::<WritableVolume<Device, NoClock>>(),
        ),
        (
            "holds its own buffer, 4,152 bytes each",
            4_152,
            size_of::<Directory<'static, Device>>(),
        ),
        (
            "holds its own buffer, 4,152 bytes each",
            4_152,
            size_of::<FileReader<'static, Device>>(),
        ),
        (
            "A `FileWriter` is 96 bytes",
            96,
            size_of::<FileWriter<'static, Device, NoClock>>(),
        ),
        ("a `NamedEntry` 576", 576, size_of::<NamedEntry>()),
        ("a `FormatPlan` 64", 64, size_of::<FormatPlan>()),
        ("a `VolumeLayout` 624", 624, size_of::<VolumeLayout>()),
        (
            "which takes 3,584 bytes off the volume",
            3_584,
            size_of::<Volume<Device>>() - size_of::<Volume<Device, 512>>(),
        ),
        (
            "each `Directory`",
            3_584,
            size_of::<Directory<'static, Device>>() - size_of::<Directory<'static, Device, 512>>(),
        ),
        (
            "and each `FileReader`",
            3_584,
            size_of::<FileReader<'static, Device>>()
                - size_of::<FileReader<'static, Device, 512>>(),
        ),
    ]
}

#[test]
fn each_stated_size_is_the_measured_one() {
    let readme = include_str!("../README.md")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for (sentence, figure, measured) in stated() {
        assert!(
            readme.contains(sentence),
            "the README no longer says: {sentence}"
        );
        assert_eq!(measured, figure, "the README says {figure} in: {sentence}");
    }
}
