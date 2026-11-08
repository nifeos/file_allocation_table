//! **Machine-checked proof (Kani) that the run finder is first-fit**, over every sequence of up to
//! six slots and every run length up to three. Behind `#[cfg(kani)]`, so only `cargo kani`
//! compiles it.
//!
//! The finder is a small state machine, so the sequence length is the bound Kani needs to unroll;
//! the writer feeds it one slot per directory slot and the rust-fatfs tests cross real directories.

use super::*;

fn any_kind() -> SlotKind {
    match kani::any::<u8>() % 3 {
        0 => SlotKind::Used,
        1 => SlotKind::Free,
        _ => SlotKind::End,
    }
}

/// Falsification: replayable `falsifications/directory_writer.verification.the_run_finder_returns_the_first_run_of_free_slots.patch`
/// **The run finder returns the first run of free slots, and only a run of free slots.** For any
/// sequence of used, free and end slots and any run length: when it returns a start, nothing
/// from there to the slot that completed it is used, the run is `count` slots or ends at an end
/// marker, and it begins right after a used slot or at the start. When it returns none, no end marker
/// came and every window of `count` slots holds a used one.
#[kani::proof]
#[kani::unwind(8)]
fn the_run_finder_returns_the_first_run_of_free_slots() {
    let count: u32 = kani::any();
    kani::assume((1..=3).contains(&count));
    let mut finder = RunFinder::new(count);
    let kinds: [SlotKind; 6] = [
        any_kind(),
        any_kind(),
        any_kind(),
        any_kind(),
        any_kind(),
        any_kind(),
    ];
    let mut found = None;
    let mut at = 0;
    while at < kinds.len() {
        if let Some(start) = finder.push(kinds[at]) {
            found = Some((start, at as u32));
            break;
        }
        at += 1;
    }
    let Some((start, done)) = found else {
        // No run: no end marker came, and every window of `count` slots holds a used one.
        assert!(!kinds.contains(&SlotKind::End));
        let mut first = 0;
        while first + count as usize <= kinds.len() {
            assert!(kinds[first..first + count as usize].contains(&SlotKind::Used));
            first += 1;
        }
        return;
    };
    assert!(start <= done);
    let mut i = start as usize;
    while i <= done as usize {
        assert_ne!(kinds[i], SlotKind::Used);
        i += 1;
    }
    if kinds[done as usize] == SlotKind::End {
        assert!(done - start < count);
    } else {
        assert_eq!(done - start + 1, count);
    }
    // First fit: the run starts right after a used slot, or at the directory's start.
    if start > 0 {
        assert_eq!(kinds[start as usize - 1], SlotKind::Used);
    }
}

/// Falsification: replayable `falsifications/directory_writer.verification.the_first_free_alias_number_is_the_least_one_not_marked.patch`
/// **The first free number of an alias window is the least number of the window not marked.**
/// For any window of 128 numbers starting anywhere an alias may, and any three numbers marked,
/// in the window or outside it: the number returned lies in the window, is not one of the
/// marks, and every number of the window below it is. The writer's window has eight times the
/// words and the same code.
#[kani::proof]
#[kani::unwind(4)]
fn the_first_free_alias_number_is_the_least_one_not_marked() {
    let first: u32 = kani::any();
    kani::assume((1..=999_999).contains(&first));
    let marks: [u32; 3] = kani::any();
    let mut window = AliasWindow::<2>::new(first);
    for number in marks {
        window.mark(number);
    }
    // Three marks cannot fill 128 numbers.
    let Some(free) = window.first_free() else {
        panic!("a window with three marks has a free number");
    };
    assert!(free >= first && free - first < 128);
    assert!(!marks.contains(&free));
    let below: u32 = kani::any();
    kani::assume(below >= first && below < free);
    assert!(marks.contains(&below));
}
