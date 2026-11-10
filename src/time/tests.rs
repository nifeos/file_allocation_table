//! Packing a time into the fields of a directory entry, and what clamps.

use super::*;

fn at(year: u16, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> CivilDateTime {
    CivilDateTime::new(year, month, day, hour, minute, second).expect("a real date")
}

/// The packed fields of a time that must be legal.
fn pack(time: CivilDateTime) -> Stamp {
    Stamp::pack(Some(time)).expect("a legal time")
}

#[test]
fn a_time_packs_into_the_specifications_fields() {
    // 2026-11-01 12:34:57: year 46 from 1980, hour 12, minute 34, seconds in twos (28), and the
    // odd second in the creation tenths byte.
    let stamp = pack(at(2026, 11, 1, 12, 34, 57));
    assert_eq!(stamp.creation_date, (46 << 9) + (11 << 5) + 1);
    assert_eq!(stamp.creation_time, (12 << 11) + (34 << 5) + 28);
    assert_eq!(stamp.creation_tenths, 100);
    assert_eq!(stamp.write_date, stamp.creation_date);
    assert_eq!(stamp.write_time, stamp.creation_time);
    assert_eq!(stamp.access_date, stamp.creation_date);
    // An even second has no tenths.
    assert_eq!(pack(at(2026, 11, 1, 12, 34, 56)).creation_tenths, 0);
    // The first and the last moments FAT can store.
    let first = pack(at(1980, 1, 1, 0, 0, 0));
    assert_eq!((first.creation_date, first.creation_time), (33, 0));
    let last = pack(at(2107, 12, 31, 23, 59, 58));
    assert_eq!(last.creation_date, (127 << 9) + (12 << 5) + 31);
    assert_eq!(last.creation_time, (23 << 11) + (59 << 5) + 29);
}

#[test]
fn no_time_packs_to_the_zero_timestamp() {
    assert_eq!(Stamp::pack(None), Ok(Stamp::ZERO));
    assert_eq!(NoClock.now(), None);
}

#[test]
fn the_first_and_last_moments_are_accepted_and_the_ones_beside_them_are_refused() {
    let legal = |t: CivilDateTime| Stamp::pack(Some(t)).is_ok();
    assert!(!legal(at(1979, 12, 31, 23, 59, 58)));
    assert!(legal(at(1980, 1, 1, 0, 0, 0)));
    assert!(legal(at(2107, 12, 31, 23, 59, 58)));
    assert!(legal(at(2107, 12, 31, 23, 59, 59)));
    assert!(!legal(at(2108, 1, 1, 0, 0, 0)));
    // A year far outside, either way.
    assert!(!legal(at(0, 1, 1, 0, 0, 0)));
    assert!(!legal(at(u16::MAX, 1, 1, 0, 0, 0)));
}

#[test]
fn each_field_just_outside_its_range_is_refused_and_just_inside_is_accepted() {
    use CivilDateTimeError::{Day, Hour, Minute, Month, Second};
    let new = CivilDateTime::new;
    assert_eq!(new(2000, 0, 1, 0, 0, 0), Err(Month));
    assert_eq!(new(2000, 13, 1, 0, 0, 0), Err(Month));
    assert!(new(2000, 1, 1, 0, 0, 0).is_ok() && new(2000, 12, 1, 0, 0, 0).is_ok());
    assert_eq!(new(2000, 1, 0, 0, 0, 0), Err(Day));
    assert_eq!(new(2000, 1, 32, 0, 0, 0), Err(Day));
    assert!(new(2000, 1, 31, 0, 0, 0).is_ok());
    assert_eq!(new(2000, 1, 1, 24, 0, 0), Err(Hour));
    assert!(new(2000, 1, 1, 23, 0, 0).is_ok());
    assert_eq!(new(2000, 1, 1, 0, 60, 0), Err(Minute));
    assert!(new(2000, 1, 1, 0, 59, 0).is_ok());
    assert_eq!(new(2000, 1, 1, 0, 0, 60), Err(Second));
    assert!(new(2000, 1, 1, 0, 0, 59).is_ok());
}

#[test]
fn a_day_past_the_end_of_its_month_is_refused() {
    let new = CivilDateTime::new;
    for (month, last) in [
        (1, 31),
        (3, 31),
        (4, 30),
        (5, 31),
        (6, 30),
        (7, 31),
        (8, 31),
        (9, 30),
        (10, 31),
        (11, 30),
        (12, 31),
    ] {
        assert!(new(2026, month, last, 0, 0, 0).is_ok(), "month {month}");
        assert_eq!(
            new(2026, month, last + 1, 0, 0, 0),
            Err(CivilDateTimeError::Day),
            "month {month}"
        );
    }
    // February: 29 days in a Gregorian leap year, 28 otherwise; a century is a leap year only
    // when it divides by 400.
    for (year, last) in [(2024, 29), (2026, 28), (2000, 29), (2100, 28), (1980, 29)] {
        assert!(new(year, 2, last, 0, 0, 0).is_ok(), "{year}");
        assert_eq!(
            new(year, 2, last + 1, 0, 0, 0),
            Err(CivilDateTimeError::Day),
            "{year}"
        );
    }
}

#[test]
fn any_year_is_a_date_and_the_getters_read_back_each_field() {
    let time = CivilDateTime::new(1066, 10, 14, 9, 30, 59).unwrap();
    assert_eq!(
        (
            time.year(),
            time.month(),
            time.day(),
            time.hour(),
            time.minute(),
            time.second()
        ),
        (1066, 10, 14, 9, 30, 59)
    );
    // A real date FAT cannot store is the writer's refusal, not the constructor's.
    assert!(Stamp::pack(Some(time)).is_err());
}

#[test]
fn a_stored_day_past_the_end_of_its_month_reads_as_no_time() {
    // 2026-02-30, which the specification's five-bit day field can hold.
    let date = (46 << 9) + (2 << 5) + 30;
    assert_eq!(unpack(date, 0, 0), None);
    assert!(unpack(date - 2, 0, 0).is_some());
}

#[test]
fn a_new_entry_takes_all_six_fields_and_a_modified_one_only_three() {
    let stamp = pack(at(2026, 11, 1, 12, 34, 57));
    let mut slot = [0xEEu8; 32];
    stamp.write_new(&mut slot);
    assert_eq!(slot[13], 100);
    assert_eq!(
        u16::from_le_bytes([slot[14], slot[15]]),
        stamp.creation_time
    );
    assert_eq!(
        u16::from_le_bytes([slot[16], slot[17]]),
        stamp.creation_date
    );
    assert_eq!(u16::from_le_bytes([slot[18], slot[19]]), stamp.access_date);
    assert_eq!(u16::from_le_bytes([slot[22], slot[23]]), stamp.write_time);
    assert_eq!(u16::from_le_bytes([slot[24], slot[25]]), stamp.write_date);
    // The bytes between (the high cluster word) and around are not touched.
    assert_eq!(slot[0..13], [0xEE; 13]);
    assert_eq!(slot[20..22], [0xEE; 2]);
    assert_eq!(slot[26..32], [0xEE; 6]);
    let later = pack(at(2027, 1, 2, 3, 4, 6));
    later.write_modified(&mut slot);
    assert_eq!(slot[13], 100, "the creation tenths stay");
    assert_eq!(
        u16::from_le_bytes([slot[16], slot[17]]),
        stamp.creation_date
    );
    assert_eq!(u16::from_le_bytes([slot[24], slot[25]]), later.write_date);
    assert_eq!(u16::from_le_bytes([slot[18], slot[19]]), later.access_date);
}

#[test]
fn a_moved_entry_copies_every_time_field_and_nothing_else() {
    let mut from = [0u8; 32];
    for (i, byte) in from.iter_mut().enumerate() {
        *byte = i as u8 + 1;
    }
    let mut to = [0xEEu8; 32];
    copy_times(&from, &mut to);
    assert_eq!(to[13..20], from[13..20]);
    assert_eq!(to[22..26], from[22..26]);
    assert_eq!(to[0..13], [0xEE; 13]);
    assert_eq!(to[20..22], [0xEE; 2]);
    assert_eq!(to[26..32], [0xEE; 6]);
}

#[test]
fn a_clock_lent_by_reference_reports_what_the_clock_reports() {
    struct Fixed(Option<CivilDateTime>);
    impl Clock for Fixed {
        fn now(&mut self) -> Option<CivilDateTime> {
            self.0
        }
    }
    let mut clock = Fixed(Some(at(2026, 11, 1, 12, 34, 56)));
    // A generic caller takes the clock by value, which is how a `&mut` clock reaches the crate.
    fn read<C: Clock>(mut clock: C) -> Option<CivilDateTime> {
        clock.now()
    }
    assert_eq!(read(&mut clock), Some(at(2026, 11, 1, 12, 34, 56)));
    clock.0 = None;
    assert_eq!(read(&mut clock), None);
}

#[test]
fn the_last_year_stores_as_itself() {
    let mid = pack(at(2107, 6, 15, 12, 0, 0));
    assert_eq!(mid.creation_date, (127 << 9) + (6 << 5) + 15);
    assert_eq!(mid.creation_time, 12 << 11);
    let early = pack(at(1980, 6, 15, 12, 0, 0));
    assert_eq!(early.creation_date, (6 << 5) + 15);
}

#[test]
fn a_stored_time_unpacks_into_the_fields_it_packed_from() {
    let stamp = pack(at(2026, 11, 1, 12, 34, 57));
    assert_eq!(stamp.created(), Some(at(2026, 11, 1, 12, 34, 57)));
    // The write time keeps two-second resolution; the access date has no time of day.
    assert_eq!(stamp.modified(), Some(at(2026, 11, 1, 12, 34, 56)));
    assert_eq!(stamp.accessed(), Some(at(2026, 11, 1, 0, 0, 0)));
    for edge in [at(1980, 1, 1, 0, 0, 0), at(2107, 12, 31, 23, 59, 59)] {
        assert_eq!(pack(edge).created(), Some(edge));
    }
}

#[test]
fn the_zero_timestamp_and_each_field_out_of_range_unpack_to_none() {
    assert_eq!(Stamp::ZERO.created(), None);
    assert_eq!(Stamp::ZERO.modified(), None);
    assert_eq!(Stamp::ZERO.accessed(), None);
    // 2000-01-01 00:00:00 is 20 << 9 | 1 << 5 | 1 with a zero time.
    let date = (20 << 9) | (1 << 5) | 1;
    assert_eq!(unpack(date, 0, 0), Some(at(2000, 1, 1, 0, 0, 0)));
    for (date, time, odd) in [
        ((20 << 9) | 1, 0, 0),             // month 0
        ((20 << 9) | (13 << 5) | 1, 0, 0), // month 13
        ((20 << 9) | (12 << 5), 0, 0),     // day 0
        (date, 24 << 11, 0),               // hour 24
        (date, 60 << 5, 0),                // minute 60
        (date, 30, 0),                     // second 60
        (date, 29, 2),                     // second 60 by the odd second
    ] {
        assert_eq!(unpack(date, time, odd), None, "{date:#x} {time:#x} {odd}");
    }
    // The last legal value of each field.
    assert_eq!(
        unpack((127 << 9) | (12 << 5) | 31, (23 << 11) | (59 << 5) | 29, 1),
        Some(at(2107, 12, 31, 23, 59, 59))
    );
    // A tenths byte past 199 is not a time, and 199 is its last legal value.
    let mut stamp = pack(at(2026, 11, 1, 12, 34, 57));
    stamp.creation_tenths = 199;
    assert_eq!(stamp.created(), Some(at(2026, 11, 1, 12, 34, 57)));
    stamp.creation_tenths = 200;
    assert_eq!(stamp.created(), None);
}

#[test]
fn a_slot_reads_into_the_six_fields_at_their_offsets() {
    let mut slot = [0u8; 32];
    for (at, byte) in slot.iter_mut().enumerate() {
        *byte = at as u8;
    }
    let stamp = Stamp::read(&slot);
    assert_eq!(stamp.creation_tenths, 13);
    assert_eq!(stamp.creation_time, u16::from_le_bytes([14, 15]));
    assert_eq!(stamp.creation_date, u16::from_le_bytes([16, 17]));
    assert_eq!(stamp.access_date, u16::from_le_bytes([18, 19]));
    assert_eq!(stamp.write_time, u16::from_le_bytes([22, 23]));
    assert_eq!(stamp.write_date, u16::from_le_bytes([24, 25]));
    // What a new entry writes reads back as itself.
    let written = pack(at(2026, 11, 1, 12, 34, 57));
    let mut slot = [0u8; 32];
    written.write_new(&mut slot);
    assert_eq!(Stamp::read(&slot), written);
}
