//! **Machine-checked proof (Kani) of the packed time**, over every field a clock can report.
//! Behind `#[cfg(kani)]`, so only `cargo kani` compiles it.

use super::*;

/// Falsification: replayable `falsifications/time.verification.any_time_packs_to_legal_fields.patch`
/// **Any time a clock reports is refused or packs to legal fields.** For every year, month, day,
/// hour, minute and second as 16-bit and 8-bit numbers, including every invalid one: the pack is
/// refused exactly when the year is outside 1980 to 2107 or a field is outside its range; otherwise
/// the date's year field is at most 127, its month 1 to 12 and its day 1 to 31, the time's hour is
/// at most 23, its minute at most 59 and its two-second count at most 29, the tenths byte is 0 or
/// 100, the access date is the write date, and the time comes back exactly as it went in.
#[kani::proof]
fn any_time_packs_to_legal_fields() {
    let now = CivilDateTime {
        year: kani::any(),
        month: kani::any(),
        day: kani::any(),
        hour: kani::any(),
        minute: kani::any(),
        second: kani::any(),
    };
    let in_range = (1980..=2107).contains(&now.year)
        && (1..=12).contains(&now.month)
        && (1..=31).contains(&now.day)
        && now.hour <= 23
        && now.minute <= 59
        && now.second <= 59;
    let Ok(stamp) = Stamp::pack(Some(now)) else {
        assert!(!in_range);
        return;
    };
    assert!(in_range);
    let date = stamp.creation_date;
    let time = stamp.creation_time;
    assert!(date >> 9 <= 127);
    assert!((1..=12).contains(&((date >> 5) & 0xF)));
    assert!((1..=31).contains(&(date & 0x1F)));
    assert!(time >> 11 <= 23);
    assert!((time >> 5) & 0x3F <= 59);
    assert!(time & 0x1F <= 29);
    assert!(stamp.creation_tenths == 0 || stamp.creation_tenths == 100);
    assert_eq!(stamp.access_date, stamp.write_date);
    assert_eq!(stamp.write_date, date);
    assert_eq!(stamp.write_time, time);
    assert_eq!(date >> 9, now.year - 1980);
    assert_eq!(((date >> 5) & 0xF) as u8, now.month);
    assert_eq!((date & 0x1F) as u8, now.day);
    assert_eq!((time >> 11) as u8, now.hour);
    assert_eq!(((time >> 5) & 0x3F) as u8, now.minute);
    assert_eq!(
        (time & 0x1F) as u8 * 2 + stamp.creation_tenths / 100,
        now.second
    );
}

/// Falsification: replayable `falsifications/time.verification.every_stored_time_reads_back_as_legal_fields_or_none.patch`
/// **Every stored time reads back as legal fields or as no time, and a packed time reads back as
/// itself.** For every date, time and tenths byte: an unpacked time is one a clock could report
/// (it packs again), its fields are the stored bits, and it is `None` only when a field is out of
/// range. For every time a clock reports that packs, the creation time reads back exactly, the
/// write time to the even second below and the access date at midnight.
#[kani::proof]
fn every_stored_time_reads_back_as_legal_fields_or_none() {
    let stored = Stamp {
        creation_tenths: kani::any(),
        creation_time: kani::any(),
        creation_date: kani::any(),
        access_date: kani::any(),
        write_time: kani::any(),
        write_date: kani::any(),
    };
    let date = stored.creation_date;
    let time = stored.creation_time;
    let legal = (1..=12).contains(&((date >> 5) & 0xF))
        && date & 0x1F != 0
        && time >> 11 <= 23
        && (time >> 5) & 0x3F <= 59
        && time & 0x1F <= 29
        && stored.creation_tenths <= 199;
    match stored.created() {
        Some(read) => {
            assert!(legal);
            assert!(Stamp::pack(Some(read)).is_ok());
            assert_eq!(read.year, 1980 + (date >> 9));
            assert_eq!(u16::from(read.month), (date >> 5) & 0xF);
            assert_eq!(u16::from(read.day), date & 0x1F);
            assert_eq!(u16::from(read.hour), time >> 11);
            assert_eq!(u16::from(read.minute), (time >> 5) & 0x3F);
            assert_eq!(
                read.second,
                (time & 0x1F) as u8 * 2 + stored.creation_tenths / 100
            );
        }
        None => assert!(!legal),
    }

    let now = CivilDateTime {
        year: kani::any(),
        month: kani::any(),
        day: kani::any(),
        hour: kani::any(),
        minute: kani::any(),
        second: kani::any(),
    };
    if let Ok(packed) = Stamp::pack(Some(now)) {
        assert_eq!(packed.created(), Some(now));
        let even = CivilDateTime {
            second: now.second - now.second % 2,
            ..now
        };
        assert_eq!(packed.modified(), Some(even));
        let midnight = CivilDateTime {
            hour: 0,
            minute: 0,
            second: 0,
            ..now
        };
        assert_eq!(packed.accessed(), Some(midnight));
    }
}
