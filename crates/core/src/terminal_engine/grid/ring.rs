//! The rings of rows that hold history and screens: the part of
//! `VecDeque`'s interface the grid uses, plus a rotation of a full ring that
//! moves no element. A flood scrolls every line into a full history, so that
//! rotation, inline, replaces moving two rows through the stack per line.
//! Scrolling part of a screen rotates a range of its rows in one pass.
use std::{
    iter::Chain,
    mem::MaybeUninit,
    ops::{Index, IndexMut, Range, RangeFull},
    ptr, slice,
};

pub(super) type Iter<'a, T> = Chain<slice::Iter<'a, T>, slice::Iter<'a, T>>;
pub(super) type IterMut<'a, T> = Chain<slice::IterMut<'a, T>, slice::IterMut<'a, T>>;

pub(super) struct Ring<T> {
    // Slots `head..head + len`, wrapping past the end, hold the elements in
    // order; every other slot is uninitialized.
    slots: Box<[MaybeUninit<T>]>,
    head: usize,
    len: usize,
}

impl<T> Ring<T> {
    pub(super) fn new() -> Self {
        Self::with_capacity(0)
    }

    pub(super) fn with_capacity(capacity: usize) -> Self {
        Self {
            slots: Box::new_uninit_slice(capacity),
            head: 0,
            len: 0,
        }
    }

    #[inline]
    pub(super) fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub(super) fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// The slot of element `index`, for `index` at most the capacity.
    #[inline]
    fn slot(&self, index: usize) -> usize {
        let slot = self.head + index;
        if slot >= self.slots.len() {
            slot - self.slots.len()
        } else {
            slot
        }
    }

    #[inline]
    pub(super) fn get(&self, index: usize) -> Option<&T> {
        // SAFETY: elements below `len` occupy initialized slots, and
        // `slot` stays below the capacity.
        (index < self.len)
            .then(|| unsafe { self.slots.get_unchecked(self.slot(index)).assume_init_ref() })
    }

    #[inline]
    pub(super) fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index >= self.len {
            return None;
        }
        let slot = self.slot(index);
        // SAFETY: as in `get`.
        Some(unsafe { self.slots.get_unchecked_mut(slot).assume_init_mut() })
    }

    #[cfg(test)]
    pub(super) fn front(&self) -> Option<&T> {
        self.get(0)
    }

    #[inline]
    pub(super) fn front_mut(&mut self) -> Option<&mut T> {
        self.get_mut(0)
    }

    #[inline]
    pub(super) fn back(&self) -> Option<&T> {
        self.get(self.len.wrapping_sub(1))
    }

    #[inline]
    pub(super) fn back_mut(&mut self) -> Option<&mut T> {
        self.get_mut(self.len.wrapping_sub(1))
    }

    #[inline]
    pub(super) fn push_back(&mut self, value: T) {
        if self.len == self.slots.len() {
            self.grow();
        }
        let slot = self.slot(self.len);
        self.slots[slot].write(value);
        self.len += 1;
    }

    #[inline]
    pub(super) fn push_front(&mut self, value: T) {
        if self.len == self.slots.len() {
            self.grow();
        }
        self.head = self.head.checked_sub(1).unwrap_or(self.slots.len() - 1);
        self.slots[self.head].write(value);
        self.len += 1;
    }

    #[inline]
    pub(super) fn pop_front(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        // SAFETY: the first element is initialized; the slot is left out of
        // the elements before anything else reads it.
        let value = unsafe { self.slots[self.head].assume_init_read() };
        self.head = self.slot(1);
        self.len -= 1;
        Some(value)
    }

    #[inline]
    pub(super) fn pop_back(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        let slot = self.slot(self.len);
        // SAFETY: the last element is initialized, and is no longer one.
        Some(unsafe { self.slots[slot].assume_init_read() })
    }

    /// Move the first element to the back. In a full ring that moves
    /// nothing: the first element's slot becomes the last.
    #[inline]
    pub(super) fn rotate_front_to_back(&mut self) {
        if self.len == self.slots.len() && self.len != 0 {
            self.head = self.slot(1);
        } else {
            self.move_front_to_back();
        }
    }

    #[cold]
    #[inline(never)]
    fn move_front_to_back(&mut self) {
        if let Some(value) = self.pop_front() {
            self.push_back(value);
        }
    }

    /// Rotate the elements in `range` so the one at `range.start + count`
    /// comes first, moving each once rather than once per step.
    pub(super) fn rotate_range_left(&mut self, range: Range<usize>, count: usize) {
        self.contiguous_range(range).rotate_left(count);
    }

    /// Rotate the elements in `range` so its last `count` come first.
    pub(super) fn rotate_range_right(&mut self, range: Range<usize>, count: usize) {
        self.contiguous_range(range).rotate_right(count);
    }

    fn contiguous_range(&mut self, range: Range<usize>) -> &mut [T] {
        assert!(
            range.start <= range.end && range.end <= self.len,
            "range out of bounds"
        );
        // A range within either slice needs no move.
        let split = self.len.min(self.slots.len() - self.head);
        if range.start < split && range.end > split {
            self.make_contiguous();
        }
        let (front, back) = self.as_mut_slices();
        if range.end <= front.len() {
            &mut front[range]
        } else {
            let split = front.len();
            &mut back[range.start - split..range.end - split]
        }
    }

    pub(super) fn truncate(&mut self, len: usize) {
        while self.len > len {
            drop(self.pop_back());
        }
    }

    pub(super) fn clear(&mut self) {
        self.truncate(0);
        self.head = 0;
    }

    /// Split off the elements from `at` on, in order.
    pub(super) fn split_off(&mut self, at: usize) -> Self {
        assert!(at <= self.len, "index out of bounds");
        let mut rest = Self::with_capacity(self.len - at);
        while self.len > at {
            let value = self.pop_back().expect("element past the split");
            rest.push_front(value);
        }
        rest
    }

    /// Take every element, first to last.
    pub(super) fn drain(&mut self, _: RangeFull) -> IntoIter<T> {
        IntoIter(std::mem::take(self))
    }

    pub(super) fn shrink_to_fit(&mut self) {
        if self.slots.len() > self.len {
            self.set_capacity(self.len);
        }
    }

    pub(super) fn shrink_to(&mut self, min_capacity: usize) {
        let capacity = min_capacity.max(self.len);
        if self.slots.len() > capacity {
            self.set_capacity(capacity);
        }
    }

    pub(super) fn as_slices(&self) -> (&[T], &[T]) {
        let first = self.len.min(self.slots.len() - self.head);
        let base = self.slots.as_ptr().cast::<T>();
        // SAFETY: the first `first` elements are in slots `head..head +
        // first`, and the rest, if they wrap, in slots `0..len - first`.
        unsafe {
            (
                slice::from_raw_parts(base.add(self.head), first),
                slice::from_raw_parts(base, self.len - first),
            )
        }
    }

    pub(super) fn as_mut_slices(&mut self) -> (&mut [T], &mut [T]) {
        let first = self.len.min(self.slots.len() - self.head);
        let base = self.slots.as_mut_ptr().cast::<T>();
        // SAFETY: as in `as_slices`; the two ranges of slots are disjoint.
        unsafe {
            (
                slice::from_raw_parts_mut(base.add(self.head), first),
                slice::from_raw_parts_mut(base, self.len - first),
            )
        }
    }

    pub(super) fn iter(&self) -> Iter<'_, T> {
        let (front, back) = self.as_slices();
        front.iter().chain(back)
    }

    pub(super) fn iter_mut(&mut self) -> IterMut<'_, T> {
        let (front, back) = self.as_mut_slices();
        front.iter_mut().chain(back)
    }

    pub(super) fn range_mut(&mut self, range: Range<usize>) -> IterMut<'_, T> {
        assert!(
            range.start <= range.end && range.end <= self.len,
            "range out of bounds"
        );
        let (front, back) = self.as_mut_slices();
        let split = front.len();
        let front = &mut front[range.start.min(split)..range.end.min(split)];
        let back = &mut back[range.start.saturating_sub(split)..range.end.saturating_sub(split)];
        front.iter_mut().chain(back)
    }

    /// Move the elements to slots `0..len`.
    fn make_contiguous(&mut self) {
        // Rotating every slot moves the uninitialized ones along with the
        // elements, so the elements keep their order from slot 0.
        self.slots.rotate_left(self.head);
        self.head = 0;
    }

    #[cold]
    #[inline(never)]
    fn grow(&mut self) {
        self.set_capacity((self.slots.len() * 2).max(4));
    }

    fn set_capacity(&mut self, capacity: usize) {
        debug_assert!(capacity >= self.len);
        let mut slots = Box::new_uninit_slice(capacity);
        let (front, back) = self.as_slices();
        let base = slots.as_mut_ptr().cast::<T>();
        // SAFETY: the new slots hold at least `len` elements; the elements
        // move to slots `0..len` in order, and the old slots, which never
        // drop what they hold, are freed below without dropping them.
        unsafe {
            ptr::copy_nonoverlapping(front.as_ptr(), base, front.len());
            ptr::copy_nonoverlapping(back.as_ptr(), base.add(front.len()), back.len());
        }
        self.slots = slots;
        self.head = 0;
    }
}

impl<T> Default for Ring<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Drop for Ring<T> {
    fn drop(&mut self) {
        let (front, back) = self.as_mut_slices();
        // SAFETY: both slices hold the initialized elements, which nothing
        // reads again.
        unsafe {
            ptr::drop_in_place(front);
            ptr::drop_in_place(back);
        }
    }
}

impl<T> Index<usize> for Ring<T> {
    type Output = T;

    #[inline]
    fn index(&self, index: usize) -> &T {
        assert!(index < self.len, "Out of bounds access");
        // SAFETY: as in `get`.
        unsafe { self.slots.get_unchecked(self.slot(index)).assume_init_ref() }
    }
}

impl<T> IndexMut<usize> for Ring<T> {
    #[inline]
    fn index_mut(&mut self, index: usize) -> &mut T {
        assert!(index < self.len, "Out of bounds access");
        let slot = self.slot(index);
        // SAFETY: as in `get`.
        unsafe { self.slots.get_unchecked_mut(slot).assume_init_mut() }
    }
}

impl<T> FromIterator<T> for Ring<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let elements: Box<[T]> = iter.into_iter().collect();
        let len = elements.len();
        // SAFETY: `MaybeUninit<T>` has `T`'s layout, so the allocation holds
        // `len` initialized slots.
        let slots = unsafe { Box::from_raw(Box::into_raw(elements) as *mut [MaybeUninit<T>]) };
        Self {
            slots,
            head: 0,
            len,
        }
    }
}

impl<'a, T> IntoIterator for &'a Ring<T> {
    type Item = &'a T;
    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut Ring<T> {
    type Item = &'a mut T;
    type IntoIter = IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

/// The elements of a ring, taken first to last.
pub(super) struct IntoIter<T>(Ring<T>);

impl<T> Iterator for IntoIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        self.0.pop_front()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.0.len(), Some(self.0.len()))
    }
}

impl<T> DoubleEndedIterator for IntoIter<T> {
    fn next_back(&mut self) -> Option<T> {
        self.0.pop_back()
    }
}

impl<T> ExactSizeIterator for IntoIter<T> {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, rc::Rc};

    // Random operations on a ring and on a `VecDeque` leave the same
    // elements, and every element is dropped exactly once.
    #[test]
    fn rings_behave_like_vec_deques() {
        let mut seed = 0x853c_49e6_748f_ea9b_u64;
        let mut next = |bound: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % bound as u64) as usize
        };
        let live = Rc::new(());
        for _ in 0..200 {
            let mut ring: Ring<(u32, Rc<()>)> = (0..next(6))
                .map(|value| (value as u32, Rc::clone(&live)))
                .collect();
            let mut deque: VecDeque<u32> = ring.iter().map(|(value, _)| *value).collect();
            for step in 0..300u32 {
                let element = || (step + 100, Rc::clone(&live));
                match next(16) {
                    0 | 1 => {
                        ring.push_back(element());
                        deque.push_back(step + 100);
                    }
                    2 => {
                        ring.push_front(element());
                        deque.push_front(step + 100);
                    }
                    3 | 4 => assert_eq!(ring.pop_front().map(|e| e.0), deque.pop_front()),
                    5 => assert_eq!(ring.pop_back().map(|e| e.0), deque.pop_back()),
                    6 | 7 => {
                        ring.rotate_front_to_back();
                        deque.rotate_left(usize::from(!deque.is_empty()));
                    }
                    8 | 9 => {
                        let start = next(deque.len() + 1);
                        let end = start + next(deque.len() - start + 1);
                        let count = next(end - start + 1);
                        let range = &mut deque.make_contiguous()[start..end];
                        if step % 2 == 0 {
                            ring.rotate_range_left(start..end, count);
                            range.rotate_left(count);
                        } else {
                            ring.rotate_range_right(start..end, count);
                            range.rotate_right(count);
                        }
                    }
                    10 => {
                        let len = next(deque.len() + 2);
                        ring.truncate(len);
                        deque.truncate(len);
                    }
                    11 => {
                        let at = next(deque.len() + 1);
                        let rest = ring.split_off(at);
                        let expected = deque.split_off(at);
                        assert!(rest.iter().map(|e| e.0).eq(expected.iter().copied()));
                        if next(2) == 0 {
                            ring = rest;
                            deque = expected;
                        }
                    }
                    12 => {
                        ring.shrink_to_fit();
                        assert_eq!(ring.capacity(), ring.len());
                    }
                    13 => ring.shrink_to(next(8)),
                    14 => {
                        let start = next(deque.len() + 1);
                        let end = start + next(deque.len() - start + 1);
                        for (element, value) in
                            ring.range_mut(start..end).zip(deque.range_mut(start..end))
                        {
                            element.0 += 1;
                            *value += 1;
                        }
                    }
                    _ => {
                        if next(8) == 0 {
                            let drained: Vec<u32> = ring.drain(..).map(|e| e.0).collect();
                            assert!(drained.iter().copied().eq(deque.drain(..)));
                        }
                    }
                }
                assert_eq!(ring.len(), deque.len());
                assert!(ring.iter().map(|e| e.0).eq(deque.iter().copied()));
                assert!(ring.iter_mut().map(|e| e.0).eq(deque.iter().copied()));
                for index in 0..=deque.len() {
                    assert_eq!(ring.get(index).map(|e| e.0), deque.get(index).copied());
                }
                assert_eq!(ring.front().map(|e| e.0), deque.front().copied());
                assert_eq!(ring.back().map(|e| e.0), deque.back().copied());
                assert_eq!(
                    Rc::strong_count(&live),
                    1 + deque.len(),
                    "dropped or leaked"
                );
            }
        }
        assert_eq!(Rc::strong_count(&live), 1);
    }

    // A full ring rotates without moving an element: each stays in its slot.
    #[test]
    fn ranges_rotate_across_the_wrap() {
        for head in 0..6 {
            let mut ring = Ring::with_capacity(6);
            let mut deque = VecDeque::new();
            for value in 0..6 {
                ring.push_back(value);
                deque.push_back(value);
            }
            for _ in 0..head {
                ring.rotate_front_to_back();
                deque.rotate_left(1);
            }
            for (start, end, count) in [(0, 6, 2), (1, 5, 3), (2, 3, 1), (0, 4, 4), (3, 6, 0)] {
                ring.rotate_range_left(start..end, count);
                deque.make_contiguous()[start..end].rotate_left(count);
                assert!(ring.iter().eq(deque.iter()), "{head} {start}..{end}");
                ring.rotate_range_right(start..end, count);
                deque.make_contiguous()[start..end].rotate_right(count);
                assert!(ring.iter().eq(deque.iter()), "{head} {start}..{end}");
            }
        }
    }

    #[test]
    fn full_rings_rotate_in_place() {
        let mut ring: Ring<u32> = (0..4).collect();
        let first = std::ptr::from_ref(&ring[0]);
        ring.rotate_front_to_back();
        assert!(ring.iter().copied().eq([1, 2, 3, 0]));
        assert_eq!(std::ptr::from_ref(&ring[3]), first);
    }
}
