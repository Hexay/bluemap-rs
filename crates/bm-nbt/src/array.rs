/// A big-endian `IntArray` (`N = 4`) or `LongArray` (`N = 8`) read in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BeArray<'a, const N: usize> {
    bytes: &'a [u8],
}

impl<'a, const N: usize> BeArray<'a, N> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        debug_assert_eq!(bytes.len() % N, 0);
        Self { bytes }
    }

    pub fn len(&self) -> usize {
        self.bytes.len() / N
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

impl<'a> BeArray<'a, 4> {
    pub fn get(&self, i: usize) -> Option<i32> {
        let b = self.bytes.get(i * 4..i * 4 + 4)?;
        Some(i32::from_be_bytes(b.try_into().unwrap()))
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = i32> + 'a {
        self.bytes.as_chunks::<4>().0.iter().map(|b| i32::from_be_bytes(*b))
    }
}

impl<'a> BeArray<'a, 8> {
    pub fn get(&self, i: usize) -> Option<u64> {
        let b = self.bytes.get(i * 8..i * 8 + 8)?;
        Some(u64::from_be_bytes(b.try_into().unwrap()))
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = u64> + 'a {
        self.bytes.as_chunks::<8>().0.iter().map(|b| u64::from_be_bytes(*b))
    }

    /// Native-endian copy into `out` (cleared first), for repeated random access.
    pub fn copy_into(&self, out: &mut Vec<u64>) {
        out.clear();
        out.extend(self.iter());
    }
}
