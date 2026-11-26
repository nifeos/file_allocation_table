//! **A file's attribute bits** the user sets: read-only, hidden, system and archive.
//!
//! `DIR_Attr` holds six bits. Two of them say what the entry is (a directory, the volume label) and
//! the writer owns them; the other four are the user's, and [`WritableVolume::set_attributes`] changes only
//! those. The result can never read as a long-name piece, whose attribute is all four low bits
//! together with the label bit, because the label bit is never set by this module and an entry
//! that already carries it is refused.

use crate::named_entry::NamedEntry;
use crate::reader::Volume;
use crate::sector::WritableSectorDevice;
use crate::write_cursor::WriteError;

/// The four bits a user sets: read-only, hidden, system, archive.
const USER_BITS: u8 = 0x27;

/// **A set of the four user-settable attribute bits.**
///
/// # Examples
///
/// ```
/// use file_allocation_table::Attributes;
///
/// let hidden_log = Attributes::HIDDEN.union(Attributes::ARCHIVE);
/// assert!(hidden_log.contains(Attributes::HIDDEN));
/// assert!(!hidden_log.contains(Attributes::READ_ONLY));
/// assert_eq!(hidden_log.difference(Attributes::HIDDEN), Attributes::ARCHIVE);
///
/// // The operators do the same.
/// assert_eq!(Attributes::HIDDEN | Attributes::ARCHIVE, hidden_log);
/// assert_eq!(hidden_log & Attributes::HIDDEN, Attributes::HIDDEN);
/// assert_eq!(hidden_log & !Attributes::HIDDEN, Attributes::ARCHIVE);
/// assert_eq!(
///     !Attributes::EMPTY,
///     Attributes::READ_ONLY | Attributes::HIDDEN | Attributes::SYSTEM | Attributes::ARCHIVE
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Attributes(u8);

impl Attributes {
    /// No bit set.
    pub const EMPTY: Attributes = Attributes(0);
    /// `ATTR_READ_ONLY`: the file may not be written or removed.
    #[doc(alias = "ATTR_READ_ONLY")]
    pub const READ_ONLY: Attributes = Attributes(0x01);
    /// `ATTR_HIDDEN`: a listing leaves the file out unless asked.
    #[doc(alias = "ATTR_HIDDEN")]
    pub const HIDDEN: Attributes = Attributes(0x02);
    /// `ATTR_SYSTEM`: the file belongs to the operating system.
    #[doc(alias = "ATTR_SYSTEM")]
    pub const SYSTEM: Attributes = Attributes(0x04);
    /// `ATTR_ARCHIVE`: the file changed since a backup last cleared the bit.
    #[doc(alias = "ATTR_ARCHIVE")]
    pub const ARCHIVE: Attributes = Attributes(0x20);

    /// The set with every bit of `self` and of `other`.
    #[must_use]
    pub const fn union(self, other: Attributes) -> Attributes {
        Attributes(self.0 | other.0)
    }

    /// The set without the bits of `other`.
    #[must_use]
    pub const fn difference(self, other: Attributes) -> Attributes {
        Attributes(self.0 & !other.0)
    }

    /// Whether every bit of `other` is in the set.
    pub const fn contains(self, other: Attributes) -> bool {
        self.0 & other.0 == other.0
    }

    /// The bits as `DIR_Attr` stores them.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// The user bits of a stored `DIR_Attr`.
    pub(crate) const fn of_stored(stored: u8) -> Attributes {
        Attributes(stored & USER_BITS)
    }
}

impl core::ops::BitOr for Attributes {
    type Output = Attributes;

    /// The set with every bit of either side: [`Attributes::union`].
    fn bitor(self, other: Attributes) -> Attributes {
        self.union(other)
    }
}

impl core::ops::BitAnd for Attributes {
    type Output = Attributes;

    /// The set with the bits both sides hold.
    fn bitand(self, other: Attributes) -> Attributes {
        Attributes(self.0 & other.0)
    }
}

impl core::ops::Not for Attributes {
    type Output = Attributes;

    /// The user bits the set does not hold. Only the four user bits can be set, so the
    /// complement of [`Attributes::EMPTY`] is all four and never a directory or label bit.
    fn not(self) -> Attributes {
        Attributes(!self.0 & USER_BITS)
    }
}

/// **`DIR_Attr` after the user bits become `new`**: the directory and label bits and the two bits
/// the specification reserves keep their value.
pub(crate) const fn apply(stored: u8, new: Attributes) -> u8 {
    (stored & !USER_BITS) + new.0
}

impl NamedEntry {
    /// The user-settable attribute bits of the entry.
    pub const fn user_attributes(&self) -> Attributes {
        Attributes::of_stored(self.attributes())
    }
}

impl<D: WritableSectorDevice> Volume<D> {
    /// **Set `entry`'s read-only, hidden, system and archive bits to exactly `attributes`**, and
    /// return the entry as stored. The directory bit and every other bit stay. Refuses the volume
    /// label, `.` and `..`.
    pub(crate) fn set_attributes(
        &mut self,
        entry: &NamedEntry,
        attributes: Attributes,
    ) -> Result<NamedEntry, WriteError<D::Error>> {
        if entry.is_dot_entry() || entry.is_volume_label() {
            return Err(WriteError::NotAFile);
        }
        self.verify(entry)?;
        let place = entry.place();
        let mut slot = self
            .read_slot(place.directory, place.short_slot)?
            .ok_or(WriteError::EntryChanged)?;
        slot[11] = apply(slot[11], attributes);
        self.write_slot(place.directory, place.short_slot, &slot)?;
        self.entry_at(place.directory, place.short_slot)
    }
}

#[cfg(kani)]
mod verification;

#[cfg(test)]
mod tests;
