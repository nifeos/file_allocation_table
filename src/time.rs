//! **Time on a volume**: the clock a writer must supply, and the packed form a directory entry
//! stores.
//!
//! The crate owns no clock, and a write that stamps a time needs one, so every caller that writes
//! supplies a [`Clock`]. A caller with no time source says so with [`NoClock`], and the entries it
//! writes carry the zero timestamp, the same bytes a `mkfs` with no clock has always written.
//!
//! FAT stores a date as 16 bits (year from 1980, month, day), a time as 16 bits (hour, minute and
//! seconds in units of two), and a creation time's odd second as a tenths-of-a-second byte of 0 or
//! 100. It stores no time zone. A clock is expected to supply UTC. The
//! crate stores exactly the fields its clock reports and converts nothing, so it cannot check that a
//! clock gives UTC; that is the caller's duty. A reader that treats FAT times as local time, such as
//! Windows, shows a stored UTC value as local time. A read takes no clock and never writes.
//!
//! A time FAT cannot store is refused, not adjusted: the operation fails with `TimeOutOfRange`
//! before it writes a byte. A clock whose time is not trustworthy reports `None` and gets the zero
//! timestamp.

/// **A calendar date and time of day**, as plain numbers with no time zone.
///
/// FAT stores a date and a time of day with no time zone. The value is expected to be UTC. The
/// crate stores these fields as given and converts nothing, so it cannot check that they are UTC. A
/// reader that treats FAT times as local time, such as Windows, shows a UTC value as local time.
///
/// [`CivilDateTime::new`] is the one way to make a value, and it refuses a date no calendar holds,
/// so every value is a real date and time of day. A real date can still lie outside the years FAT
/// stores: a writer refuses that one (see [`Clock`]) with `TimeOutOfRange`, and writes nothing.
///
/// # Examples
///
/// ```
/// use file_allocation_table::{CivilDateTime, CivilDateTimeError};
///
/// let noon = CivilDateTime::new(2026, 11, 10, 12, 0, 0).unwrap();
/// assert_eq!((noon.year(), noon.month(), noon.day()), (2026, 11, 10));
/// assert_eq!(
///     CivilDateTime::new(2026, 2, 29, 0, 0, 0),
///     Err(CivilDateTimeError::Day)
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CivilDateTime {
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
}

/// **A date no calendar holds**: the field of [`CivilDateTime::new`] that is out of its range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CivilDateTimeError {
    /// The month is not 1 to 12.
    Month,
    /// The day is 0 or past the last day of its month (February has 29 days in a leap year of the
    /// Gregorian calendar and 28 otherwise).
    Day,
    /// The hour is past 23.
    Hour,
    /// The minute is past 59.
    Minute,
    /// The second is past 59.
    Second,
}

/// The number of days in `month` (1 to 12) of `year`, in the proleptic Gregorian calendar.
const fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

impl CivilDateTime {
    /// **A date and time of day**, or the field that makes it one no calendar holds: a month
    /// outside 1 to 12, a day of 0 or past the end of its month, an hour past 23, a minute or a
    /// second past 59. Any year is accepted; a writer refuses one FAT cannot store.
    pub const fn new(
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    ) -> Result<CivilDateTime, CivilDateTimeError> {
        if month < 1 || month > 12 {
            return Err(CivilDateTimeError::Month);
        }
        if day < 1 || day > days_in_month(year, month) {
            return Err(CivilDateTimeError::Day);
        }
        if hour > 23 {
            return Err(CivilDateTimeError::Hour);
        }
        if minute > 59 {
            return Err(CivilDateTimeError::Minute);
        }
        if second > 59 {
            return Err(CivilDateTimeError::Second);
        }
        Ok(CivilDateTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
        })
    }

    /// The year, for example 2026.
    pub const fn year(&self) -> u16 {
        self.year
    }

    /// The month, 1 to 12.
    pub const fn month(&self) -> u8 {
        self.month
    }

    /// The day of the month, from 1 to the last day of the month.
    pub const fn day(&self) -> u8 {
        self.day
    }

    /// The hour, 0 to 23.
    pub const fn hour(&self) -> u8 {
        self.hour
    }

    /// The minute, 0 to 59.
    pub const fn minute(&self) -> u8 {
        self.minute
    }

    /// The second, 0 to 59. FAT keeps two-second resolution, and a creation time keeps the odd
    /// second in its tenths byte.
    pub const fn second(&self) -> u8 {
        self.second
    }
}

/// **Where a writer gets the time.** A volume that writes holds one.
///
/// The time a clock supplies is expected to be UTC. FAT stores a date and a time of
/// day with no time zone, the crate writes the fields it is given and converts nothing, and it
/// cannot check that a clock gives UTC. A reader that treats FAT times as local time, such as
/// Windows, shows a UTC value as local time. A read takes no clock and never writes.
///
/// [`Clock::now`] returns `None` when there is no time to report, and the entry then carries the
/// zero timestamp. A clock whose time is not trustworthy (one that has not been set, or has lost
/// power) returns `None`; it does not guess. A time FAT cannot store, a year before 1980 or after
/// 2107, is refused: the operation then fails with `TimeOutOfRange` before it writes a byte, and
/// the writer stays usable. A date no calendar holds never reaches a writer, because
/// [`CivilDateTime::new`] refuses it.
///
/// The crate takes the clock as a type parameter and not as a trait object, so it needs no
/// allocator and no `dyn` indirection, and a caller's clock is a plain value.
///
/// # Examples
///
/// ```
/// use file_allocation_table::{Clock, CivilDateTime};
///
/// /// A clock a test sets by hand.
/// struct Fixed(Option<CivilDateTime>);
///
/// impl Clock for Fixed {
///     fn now(&mut self) -> Option<CivilDateTime> {
///         self.0
///     }
/// }
/// ```
pub trait Clock {
    /// The current time, or `None` when there is none to report.
    fn now(&mut self) -> Option<CivilDateTime>;
}

/// **The explicit "no time"**: a clock that always reports none. A volume written with it carries
/// the zero timestamp in every entry it stamps, so an image built with it is reproducible. It
/// reports no time, so the question of a time zone does not arise (a clock that does report a time
/// must report UTC; see [`Clock`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct NoClock;

impl Clock for NoClock {
    fn now(&mut self) -> Option<CivilDateTime> {
        None
    }
}

impl<C: Clock + ?Sized> Clock for &mut C {
    fn now(&mut self) -> Option<CivilDateTime> {
        (**self).now()
    }
}

/// The first year FAT stores.
const FIRST_YEAR: u16 = 1980;
/// The last year FAT stores.
const LAST_YEAR: u16 = 2107;

/// **A time FAT cannot store**: what a clock reported was out of range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OutOfRange;

/// **The six time fields of a directory entry, packed**: what a stamp writes and what a short
/// entry reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Stamp {
    /// `DIR_CrtTimeTenth`: 0 or 100, the odd second of the creation time.
    pub(crate) creation_tenths: u8,
    /// `DIR_CrtTime`.
    pub(crate) creation_time: u16,
    /// `DIR_CrtDate`.
    pub(crate) creation_date: u16,
    /// `DIR_LstAccDate`.
    pub(crate) access_date: u16,
    /// `DIR_WrtTime`.
    pub(crate) write_time: u16,
    /// `DIR_WrtDate`.
    pub(crate) write_date: u16,
}

impl Stamp {
    /// The zero timestamp: every field zero.
    pub(crate) const ZERO: Stamp = Stamp {
        creation_tenths: 0,
        creation_time: 0,
        creation_date: 0,
        access_date: 0,
        write_time: 0,
        write_date: 0,
    };

    /// **Pack `now`**, or refuse it. `None` packs to [`Stamp::ZERO`]. A year outside 1980 to 2107
    /// is [`OutOfRange`]; every other field is already in range, because [`CivilDateTime::new`]
    /// checked it. The check runs before any byte is written.
    pub(crate) fn pack(now: Option<CivilDateTime>) -> Result<Stamp, OutOfRange> {
        let Some(now) = now else {
            return Ok(Stamp::ZERO);
        };
        let legal = now.year >= FIRST_YEAR && now.year <= LAST_YEAR;
        if !legal {
            return Err(OutOfRange);
        }
        let date =
            ((now.year - FIRST_YEAR) << 9) + (u16::from(now.month) << 5) + u16::from(now.day);
        let time =
            (u16::from(now.hour) << 11) + (u16::from(now.minute) << 5) + u16::from(now.second / 2);
        Ok(Stamp {
            creation_tenths: (now.second % 2) * 100,
            creation_time: time,
            creation_date: date,
            access_date: date,
            write_time: time,
            write_date: date,
        })
    }

    /// The six fields as the 32-byte short entry `slot` stores them.
    pub(crate) const fn read(slot: &[u8; 32]) -> Stamp {
        Stamp {
            creation_tenths: slot[13],
            creation_time: u16::from_le_bytes([slot[14], slot[15]]),
            creation_date: u16::from_le_bytes([slot[16], slot[17]]),
            access_date: u16::from_le_bytes([slot[18], slot[19]]),
            write_time: u16::from_le_bytes([slot[22], slot[23]]),
            write_date: u16::from_le_bytes([slot[24], slot[25]]),
        }
    }

    /// The creation time, to the second.
    pub(crate) const fn created(&self) -> Option<CivilDateTime> {
        if self.creation_tenths > 199 {
            return None;
        }
        unpack(
            self.creation_date,
            self.creation_time,
            self.creation_tenths / 100,
        )
    }

    /// The last write time, to two seconds.
    pub(crate) const fn modified(&self) -> Option<CivilDateTime> {
        unpack(self.write_date, self.write_time, 0)
    }

    /// The last access date, at midnight.
    pub(crate) const fn accessed(&self) -> Option<CivilDateTime> {
        unpack(self.access_date, 0, 0)
    }

    /// Write all six fields into the 32-byte short entry `slot`: a new entry, created, written and
    /// accessed now.
    pub(crate) fn write_new(&self, slot: &mut [u8]) {
        slot[13] = self.creation_tenths;
        slot[14..16].copy_from_slice(&self.creation_time.to_le_bytes());
        slot[16..18].copy_from_slice(&self.creation_date.to_le_bytes());
        self.write_modified(slot);
    }

    /// Write the write time, the write date and the access date into `slot`, and leave the
    /// creation fields: an entry changed now.
    pub(crate) fn write_modified(&self, slot: &mut [u8]) {
        slot[18..20].copy_from_slice(&self.access_date.to_le_bytes());
        slot[22..24].copy_from_slice(&self.write_time.to_le_bytes());
        slot[24..26].copy_from_slice(&self.write_date.to_le_bytes());
    }
}

/// **Unpack a stored date and time**, with `odd_second` (0 or 1) added to the time's two-second
/// count, or `None` when a field is outside its range or the day is past the end of its month. The
/// zero timestamp has month 0, so it is `None`: the entry carries no time.
pub(crate) const fn unpack(date: u16, time: u16, odd_second: u8) -> Option<CivilDateTime> {
    let month = ((date >> 5) & 0xF) as u8;
    let day = (date & 0x1F) as u8;
    let hour = (time >> 11) as u8;
    let minute = ((time >> 5) & 0x3F) as u8;
    let second = (time & 0x1F) as u8 * 2 + odd_second;
    match CivilDateTime::new(FIRST_YEAR + (date >> 9), month, day, hour, minute, second) {
        Ok(time) => Some(time),
        Err(_) => None,
    }
}

/// Copy the time fields of the short entry `from` into `to`: a moved entry keeps its times.
pub(crate) fn copy_times(from: &[u8], to: &mut [u8]) {
    to[13..20].copy_from_slice(&from[13..20]);
    to[22..26].copy_from_slice(&from[22..26]);
}

#[cfg(kani)]
mod verification;

#[cfg(test)]
mod tests;
