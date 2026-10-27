//! Keeping parsed binaries together with the bytes they borrow.

use std::sync::Arc;

/// A value parsed from `data` that borrows it, as pewter's `PEFile<'a>`
/// and `ElfFile<'a>` do, made `'static` by keeping the bytes alive with it.
///
/// Nothing borrowed from `value` may outlive the `Parsed`: Xur only ever
/// sees copies, converted to Xur values.
pub(super) struct Parsed<T> {
    // Declared before `data`, so it is dropped before the bytes it borrows.
    value: T,
    data: Arc<[u8]>,
}

impl<T> Parsed<T> {
    pub(super) fn new<E>(
        data: Vec<u8>,
        parse: impl FnOnce(&'static [u8]) -> Result<T, E>,
    ) -> Result<Self, E> {
        let data: Arc<[u8]> = data.into();
        // SAFETY: the bytes are behind an `Arc`, so they never move, are
        // never changed, and live until `data` is dropped, after `value`.
        let bytes: &'static [u8] = unsafe { &*Arc::as_ptr(&data) };
        Ok(Self {
            value: parse(bytes)?,
            data,
        })
    }

    pub(super) fn get(&self) -> &T {
        &self.value
    }

    /// Something else borrowing the same bytes, such as part of `value`.
    pub(super) fn derive<U>(&self, f: impl FnOnce(&T) -> U) -> Parsed<U> {
        Parsed {
            value: f(&self.value),
            data: Arc::clone(&self.data),
        }
    }
}
