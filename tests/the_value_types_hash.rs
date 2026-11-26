//! **The builders' types and the named entry hash**, beside `PartialEq` and `Eq` as every other
//! public value type of the crate does, so a caller can key a map or a set by one.

use std::collections::HashSet;
use std::hash::Hash;

use file_allocation_table::{
    FormatOptions, FormatPlan, LayoutError, NamedEntry, NoClock, SectorSize, VolumeLayout,
    VolumeLayoutOptions,
};

fn hashes<T: Hash + Eq>(value: T) {
    let mut set = HashSet::new();
    assert!(set.insert(value));
}

#[test]
fn the_layout_and_format_types_and_the_named_entry_hash() {
    let options = VolumeLayoutOptions::new(70_000 * 8 + 2_000);
    hashes(options);
    hashes(VolumeLayout::new(options, &mut NoClock).unwrap());
    hashes(LayoutError::NotFat32);
    let format = FormatOptions::new(SectorSize::Bytes512, 8_000);
    hashes(format);
    hashes(FormatPlan::new(format, &mut NoClock).unwrap());
    fn is_a_key<T: Hash + Eq>() {}
    is_a_key::<NamedEntry>();
}
