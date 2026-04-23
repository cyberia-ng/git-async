use core::ops::Range;

pub(crate) trait SubsliceRange {
    fn subslice_range_stable(&self, subslice: &Self) -> Option<Range<usize>>;
}

impl<T> SubsliceRange for [T] {
    fn subslice_range_stable(&self, subslice: &[T]) -> Option<Range<usize>> {
        let first = subslice.first()?;
        let start = self.element_offset(first)?;
        let end = start + subslice.len();
        Some(Range { start, end })
    }
}
