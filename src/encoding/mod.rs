//! Utilities for percent-encoding.

pub mod encoder;
mod estring;
pub(crate) mod imp;
pub mod table;

pub use estring::EString;

use alloc::{
    borrow::{Cow, ToOwned},
    string::{FromUtf8Error, String},
    vec::Vec,
};
use core::{cmp::Ordering, hash, iter::FusedIterator, marker::PhantomData, str};
use encoder::{Encoder, Path};
use ref_cast::{ref_cast_custom, RefCastCustom};

use self::encoder::PathSegment;

/// Percent-encoded string slices.
#[derive(RefCastCustom)]
#[repr(transparent)]
pub struct EStr<E: Encoder> {
    encoder: PhantomData<E>,
    inner: str,
}

impl<E: Encoder> EStr<E> {
    const ASSERT_ALLOWS_ENC: () = assert!(
        E::TABLE.allows_enc(),
        "table does not allow percent-encoding"
    );

    /// Converts a string slice to an `EStr` slice assuming validity.
    #[ref_cast_custom]
    #[inline]
    pub(crate) const fn new_validated(s: &str) -> &Self;

    /// Converts a string slice to an `EStr` slice.
    ///
    /// Only use this function when you have a percent-encoded string at hand.
    /// You may otherwise encode and concatenate strings to an [`EString`]
    /// which derefs to [`EStr`].
    ///
    /// # Panics
    ///
    /// Panics if the string is not properly encoded with `E`.
    pub const fn new(s: &str) -> &Self {
        assert!(E::TABLE.validate(s.as_bytes()), "improperly encoded string");
        return Self::new_validated(s);
    }

    /// Yields the underlying string slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.inner
    }

    /// Decodes the `EStr` slice.
    ///
    /// This method allocates only when there is any percent-encoded octet in the slice.
    ///
    /// # Panics
    ///
    /// Panics at compile time if the table specified
    /// by `E` does not allow percent-encoding.
    ///
    /// # Examples
    ///
    /// ```
    /// use fluent_uri::encoding::{EStr, encoder::Path};
    ///
    /// let dec = EStr::<Path>::new("%C2%A1Hola%21").decode();
    /// assert_eq!(dec.as_bytes(), &[0xc2, 0xa1, 0x48, 0x6f, 0x6c, 0x61, 0x21]);
    /// assert_eq!(dec.into_string()?, "¡Hola!");
    /// # Ok::<_, std::string::FromUtf8Error>(())
    /// ```
    #[inline]
    pub fn decode(&self) -> Decode<'_> {
        let _ = Self::ASSERT_ALLOWS_ENC;

        match imp::decode(self.inner.as_bytes()) {
            Some(vec) => Decode::Owned(vec),
            None => Decode::Borrowed(self.as_str()),
        }
    }

    /// Returns an iterator over subslices of the `EStr` slice separated by the given delimiter.
    ///
    /// # Panics
    ///
    /// Panics if the delimiter is not a [reserved] character.
    ///
    /// [reserved]: https://datatracker.ietf.org/doc/html/rfc3986/#section-2.2
    ///
    /// # Examples
    ///
    /// ```
    /// use fluent_uri::encoding::{EStr, encoder::Path};
    ///
    /// assert!(EStr::<Path>::new("a,b,c").split(',').eq(["a", "b", "c"]));
    /// assert!(EStr::<Path>::new(",").split(',').eq(["", ""]));
    /// ```
    #[inline]
    pub fn split(&self, delim: char) -> Split<'_, E> {
        assert!(
            delim.is_ascii() && table::RESERVED.allows(delim as u8),
            "splitting with non-reserved character"
        );
        Split {
            inner: self.inner.split(delim),
            encoder: PhantomData,
        }
    }

    /// Splits the `EStr` slice on the first occurrence of the given delimiter and
    /// returns prefix before delimiter and suffix after delimiter.
    ///
    /// Returns `None` if the delimiter is not found.
    ///
    /// # Panics
    ///
    /// Panics if the delimiter is not a [reserved] character.
    ///
    /// [reserved]: https://datatracker.ietf.org/doc/html/rfc3986/#section-2.2
    ///
    /// # Examples
    ///
    /// ```
    /// use fluent_uri::encoding::{EStr, encoder::Path};
    ///
    /// assert_eq!(
    ///     EStr::<Path>::new("foo;bar").split_once(';'),
    ///     Some((EStr::new("foo"), EStr::new("bar")))
    /// );
    ///
    /// assert_eq!(EStr::<Path>::new("foo").split_once(';'), None);
    /// ```
    #[inline]
    pub fn split_once(&self, delim: char) -> Option<(&Self, &Self)> {
        assert!(
            delim.is_ascii() && table::RESERVED.allows(delim as u8),
            "splitting with non-reserved character"
        );
        self.inner
            .split_once(delim)
            .map(|(a, b)| (Self::new_validated(a), Self::new_validated(b)))
    }
}

impl<E: Encoder> AsRef<Self> for EStr<E> {
    #[inline]
    fn as_ref(&self) -> &Self {
        self
    }
}

impl<E: Encoder> AsRef<str> for EStr<E> {
    #[inline]
    fn as_ref(&self) -> &str {
        &self.inner
    }
}

impl<E: Encoder> AsRef<[u8]> for EStr<E> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.inner.as_bytes()
    }
}

impl<E: Encoder, F: Encoder> PartialEq<EStr<F>> for EStr<E> {
    #[inline]
    fn eq(&self, other: &EStr<F>) -> bool {
        self.inner == other.inner
    }
}

impl<E: Encoder> PartialEq<str> for EStr<E> {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        &self.inner == other
    }
}

impl<E: Encoder> PartialEq<EStr<E>> for str {
    #[inline]
    fn eq(&self, other: &EStr<E>) -> bool {
        self == &other.inner
    }
}

impl<E: Encoder> Eq for EStr<E> {}

impl<E: Encoder> hash::Hash for EStr<E> {
    #[inline]
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.inner.hash(state)
    }
}

impl<E: Encoder> PartialOrd for EStr<E> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Implements ordering on `EStr` slices.
///
/// `EStr` slices are ordered [lexicographically](Ord#lexicographical-comparison) by their byte values.
/// Normalization is **not** performed prior to ordering.
impl<E: Encoder> Ord for EStr<E> {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.inner.cmp(&other.inner)
    }
}

impl<E: Encoder> Default for &EStr<E> {
    /// Creates an empty `EStr` slice.
    #[inline]
    fn default() -> Self {
        EStr::new_validated("")
    }
}

impl<E: Encoder> ToOwned for EStr<E> {
    type Owned = EString<E>;

    #[inline]
    fn to_owned(&self) -> EString<E> {
        EString::new_validated(self.inner.to_owned())
    }

    #[inline]
    fn clone_into(&self, target: &mut EString<E>) {
        self.inner.clone_into(&mut target.buf)
    }
}

/// Extension methods for the [path] component of URI reference.
///
/// [path]: https://datatracker.ietf.org/doc/html/rfc3986/#section-3.3
impl EStr<Path> {
    /// Returns `true` if the path is absolute, i.e., beginning with "/".
    #[inline]
    pub fn is_absolute(&self) -> bool {
        self.inner.starts_with('/')
    }

    /// Returns `true` if the path is rootless, i.e., not beginning with "/".
    #[inline]
    pub fn is_rootless(&self) -> bool {
        !self.inner.starts_with('/')
    }

    /// Returns an iterator over the path [segments].
    ///
    /// [segments]: https://datatracker.ietf.org/doc/html/rfc3986/#section-3.3
    ///
    /// # Examples
    ///
    /// ```
    /// use fluent_uri::Uri;
    ///
    /// // An empty path has no segments.
    /// let uri = Uri::parse("")?;
    /// assert_eq!(uri.path().segments().next(), None);
    ///
    /// // Segments are separated by "/".
    /// let uri = Uri::parse("a/b/c")?;
    /// assert!(uri.path().segments().eq(["a", "b", "c"]));
    ///
    /// // The empty string before a preceding "/" is not a segment.
    /// // However, segments can be empty in the other cases.
    /// let uri = Uri::parse("/path/to//dir/")?;
    /// assert!(uri.path().segments().eq(["path", "to", "", "dir", ""]));
    /// # Ok::<_, fluent_uri::ParseError>(())
    /// ```
    #[inline]
    pub fn segments(&self) -> Split<'_, PathSegment> {
        let path_stripped = self.inner.strip_prefix('/').unwrap_or(&self.inner);

        let mut split = path_stripped.split('/');
        if self.inner.is_empty() {
            split.next();
        }

        Split {
            inner: split,
            encoder: PhantomData,
        }
    }
}

/// A wrapper of percent-decoded bytes.
///
/// This enum is created by [`EStr::decode`].
///
/// [`decode`]: EStr::decode
#[derive(Clone, Debug)]
pub enum Decode<'a> {
    /// No percent-encoded octets are decoded.
    Borrowed(&'a str),
    /// One or more percent-encoded octets are decoded.
    Owned(Vec<u8>),
}

impl<'a> Decode<'a> {
    /// Returns a reference to the decoded bytes.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Borrowed(s) => s.as_bytes(),
            Self::Owned(vec) => vec,
        }
    }

    /// Consumes this `Decode` and yields the underlying decoded bytes.
    #[inline]
    pub fn into_bytes(self) -> Cow<'a, [u8]> {
        match self {
            Self::Borrowed(s) => Cow::Borrowed(s.as_bytes()),
            Self::Owned(vec) => Cow::Owned(vec),
        }
    }

    /// Converts the decoded bytes to a string.
    ///
    /// Returns `Err` if the decoded bytes are not valid UTF-8.
    #[inline]
    pub fn into_string(self) -> Result<Cow<'a, str>, FromUtf8Error> {
        match self {
            Self::Borrowed(s) => Ok(Cow::Borrowed(s)),
            Self::Owned(vec) => String::from_utf8(vec).map(Cow::Owned),
        }
    }

    /// Converts the decoded bytes to a string, including invalid characters.
    pub fn into_string_lossy(self) -> Cow<'a, str> {
        match self {
            Self::Borrowed(s) => Cow::Borrowed(s),
            Self::Owned(vec) => Cow::Owned(match String::from_utf8(vec) {
                Ok(string) => string,
                Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
            }),
        }
    }
}

/// An iterator over subslices of an [`EStr`] separated by a delimiter.
///
/// This struct is created by [`EStr::split`].
///
/// [`split`]: EStr::split
#[derive(Clone, Debug)]
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct Split<'a, E: Encoder> {
    inner: str::Split<'a, char>,
    encoder: PhantomData<E>,
}

impl<'a, E: Encoder> Iterator for Split<'a, E> {
    type Item = &'a EStr<E>;

    #[inline]
    fn next(&mut self) -> Option<&'a EStr<E>> {
        self.inner.next().map(EStr::new_validated)
    }
}

impl<'a, E: Encoder> DoubleEndedIterator for Split<'a, E> {
    #[inline]
    fn next_back(&mut self) -> Option<&'a EStr<E>> {
        self.inner.next_back().map(EStr::new_validated)
    }
}

impl<E: Encoder> FusedIterator for Split<'_, E> {}
