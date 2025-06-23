//! Things pertaining to the DATA segment (mostly)
//!
//! Basic overview: DATA is arranged according to version specific "layouts".
//! Each layout will enumerate all possible combinations that may be represented
//! in version, which directly correspond to all valid combinations of $BYTEORD,
//! $DATATYPE, $PnB, $PnR, and $PnDATATYPE in the case of 3.2.
//!
//! Each layout may then be projected in a "reader" or "writer." Readers are
//! essentially blank vectors waiting to accept the data from disk. Writers are
//! iterators that read values from the dataframe, possibly convert them, and
//! emit the resulting bytes for writing to disk.
//!
//! Now for the ugly bits.
//!
//! Performance is critical since files can be large, and we want to possibly
//! pass data into Python, R, etc. Therefore, no dynamic dispatch. This is also
//! sensible to avoid given that the types should represent *valid* layout
//! configurations only, which trait objects obscure.
//!
//! For layouts this isn't so bad; the main rub is that floats have two widths
//! (32 and 64), integers have eight widths (1-8 bytes), and each of these can
//! have their bytes as big/little endian or using byte order where the bytes
//! may not be strictly monotonic in either direction. The former is refereed to
//! as "Endian" and the latter "Ordered" throughout.
//!
//! To make this extra confusing, Endian is a subset of Ordered, since all
//! possible byte orders include the two corresponding to big/little endian.
//! This is important, because if we allowed Ordered in all versions, then it
//! would be theoretically possible to create a 3.1 or 3.2 layout with a
//! non-big/little endian byte order, which is bad design.
//!
//! For readers/writers, it is sensible to use one type for all layouts, since
//! the readers/writers do not directly correspond to keywords in TEXT. It would
//! also be a giant pain to make version-specific readers/writers, and the gain
//! would be minimal. Thus each layout for each version will be non-surjectively
//! mapped into a reader or writer. Principally, this means that Endian layouts
//! will get mapped into Ordered layouts, since the latter includes the former.
//!
//! Lastly, writers are extra fun because they encode iterators that map from
//! all possible types in the dataframe (six) to all possible types that may be
//! written (twelve).

use crate::config::{ReaderConfig, SharedConfig};
use crate::core::*;
use crate::error::*;
use crate::macros::{enum_from, enum_from_disp, match_many_to_one, newtype_disp, newtype_from};
use crate::segment::*;
use crate::text::byteord::*;
use crate::text::float_or_int::*;
use crate::text::index::{IndexFromOne, MeasIndex};
use crate::text::keywords::*;
use crate::text::parser::*;
use crate::validated::dataframe::*;
use crate::validated::standard::*;

use itertools::Itertools;
use nonempty::NonEmpty;
use serde::ser::SerializeStruct;
use serde::Serialize;
use std::convert::Infallible;
use std::fmt;
use std::io;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
use std::num::ParseIntError;
use std::str;
use std::str::FromStr;

/// All possible byte layouts for the DATA segment in 2.0.
///
/// This is identical to 3.0 in every way except that the $TOT keyword in 2.0
/// is optional, which requires a different interface.
#[derive(Clone, Serialize)]
pub struct Layout2_0(pub AnyOrderedLayout<MaybeTot>);

newtype_from!(Layout2_0, AnyOrderedLayout<MaybeTot>);

/// All possible byte layouts for the DATA segment in 2.0.
#[derive(Clone, Serialize)]
pub struct Layout3_0(pub AnyOrderedLayout<KnownTot>);

newtype_from!(Layout3_0, AnyOrderedLayout<KnownTot>);

/// All possible byte layouts for the DATA segment in 3.1.
///
/// Unlike 2.0 and 3.0, the integer layout allows the column widths to be
/// different. This is a consequence of making BYTEORD only mean "big or little
/// endian" and have nothing to do with number of bytes.
#[derive(Clone, Serialize)]
pub struct Layout3_1(pub NonMixedEndianLayout);

newtype_from!(Layout3_1, NonMixedEndianLayout);

enum_from!(
    /// All possible byte layouts for the DATA segment in 3.2.
    ///
    /// In addition to the loosened integer layouts in 3.1, 3.2 additionally allows
    /// each column to have a different type and size (hence "Mixed").
    #[derive(Clone, Serialize)]
    pub Layout3_2,
    [Mixed, EndianLayout<NullMixedType>],
    [NonMixed, NonMixedEndianLayout]
);

/// All possible byte layouts for the DATA segment in 2.0 and 3.0.
///
/// It is so named "Ordered" because the BYTEORD keyword represents any possible
/// byte ordering that may occur rather than simply little or big endian.
#[derive(Clone, Serialize)]
pub enum AnyOrderedLayout<T> {
    Ascii(AnyAsciiLayout<T>),
    Integer(AnyOrderedUintLayout<T>),
    F32(OrderedLayout<F32Type, T>),
    F64(OrderedLayout<F64Type, T>),
}

enum_from!(
    #[derive(Clone, Serialize)]
    pub NonMixedEndianLayout,
    [Ascii, AnyAsciiLayout<KnownTot>],
    [Integer, EndianLayout<NullAnyUintType>],
    [F32, EndianLayout<F32Type>],
    [F64, EndianLayout<F64Type>]
);

type EndianLayout<C> = FixedLayout<C, Endian, KnownTot>;

/// Byte layouts for ASCII data.
///
/// This may either be fixed (ie columns have the same number of characters)
/// or variable (ie columns have have different number of characters and are
/// separated by delimiters).
#[derive(Clone, Serialize)]
pub enum AnyAsciiLayout<T> {
    Delimited(DelimAsciiLayout<T>),
    Fixed(FixedAsciiLayout<T>),
}

type FixedAsciiLayout<T> = FixedLayout<AsciiType, (), T>;

/// Byte layout for delimited ASCII.
#[derive(Clone)]
pub struct DelimAsciiLayout<T> {
    pub ranges: NonEmpty<u64>,
    tot_action: PhantomData<T>,
}

/// Byte layout where each column has a fixed width.
#[derive(Clone)]
struct FixedLayout<C, L, T> {
    byte_layout: L,
    columns: NonEmpty<C>,
    tot_action: PhantomData<T>,
}

/// Byte layout for integers that may be in any byte order.
#[derive(Clone, Serialize)]
pub enum AnyOrderedUintLayout<T> {
    // TODO the first two don't need to be ordered
    Uint08(OrderedLayout<Uint08Type, T>),
    Uint16(OrderedLayout<Uint16Type, T>),
    Uint24(OrderedLayout<Uint24Type, T>),
    Uint32(OrderedLayout<Uint32Type, T>),
    Uint40(OrderedLayout<Uint40Type, T>),
    Uint48(OrderedLayout<Uint48Type, T>),
    Uint56(OrderedLayout<Uint56Type, T>),
    Uint64(OrderedLayout<Uint64Type, T>),
}

type OrderedLayout<C, T> = FixedLayout<C, <C as HasNativeWidth>::Order, T>;

macro_rules! into_any_ordered_layout {
    ($var:ident, $inner:ident) => {
        impl<T> From<OrderedLayout<$inner, T>> for AnyOrderedUintLayout<T> {
            fn from(value: OrderedLayout<$inner, T>) -> Self {
                Self::$var(value)
            }
        }
    };
}

into_any_ordered_layout!(Uint08, Uint08Type);
into_any_ordered_layout!(Uint16, Uint16Type);
into_any_ordered_layout!(Uint24, Uint24Type);
into_any_ordered_layout!(Uint32, Uint32Type);
into_any_ordered_layout!(Uint40, Uint40Type);
into_any_ordered_layout!(Uint48, Uint48Type);
into_any_ordered_layout!(Uint56, Uint56Type);
into_any_ordered_layout!(Uint64, Uint64Type);

/// The type of a non-delimited column in the DATA segment for 3.2
pub enum MixedType<F: ColumnFamily> {
    Ascii(F::ColumnWrapper<AsciiType, u64, ()>),
    Uint(AnyUintType<F>),
    F32(NativeWrapper<F, F32Type>),
    F64(NativeWrapper<F, F64Type>),
}

/// A big or little-endian integer column of some size (1-8 bytes)
pub enum AnyUintType<F: ColumnFamily> {
    Uint08(NativeWrapper<F, Uint08Type>),
    Uint16(NativeWrapper<F, Uint16Type>),
    Uint24(NativeWrapper<F, Uint24Type>),
    Uint32(NativeWrapper<F, Uint32Type>),
    Uint40(NativeWrapper<F, Uint40Type>),
    Uint48(NativeWrapper<F, Uint48Type>),
    Uint56(NativeWrapper<F, Uint56Type>),
    Uint64(NativeWrapper<F, Uint64Type>),
}

// enum_from!(
//     /// The type of a non-delimited column in the DATA segment for 3.2
//     #[derive(Clone, Copy, Serialize)]
//     pub MixedType,
//     [Ascii, AsciiType],
//     [Integer, AnyUintType],
//     [Float, F32Type],
//     [Double, F64Type]
// );

// enum_from!(
//     /// A big or little-endian integer column of some size (1-8 bytes)
//     #[derive(Clone, Copy, Serialize)]
//     pub AnyUintType,
//     [Uint08, Uint08Type],
//     [Uint16, Uint16Type],
//     [Uint24, Uint24Type],
//     [Uint32, Uint32Type],
//     [Uint40, Uint40Type],
//     [Uint48, Uint48Type],
//     [Uint56, Uint56Type],
//     [Uint64, Uint64Type]
// );

macro_rules! impl_null_layout {
    ($t:path, $($var:ident),*) => {
        impl Clone for $t {
            fn clone(&self) -> Self {
                match self {
                    $(
                        Self::$var(x) => Self::$var(x.clone()),
                    )*
                }
            }
        }

        impl Serialize for $t {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                match self {
                    $(
                        Self::$var(x) => x.serialize(serializer),
                    )*
                }
            }
        }
    };
}

impl_null_layout!(NullMixedType, Ascii, Uint, F32, F64);

impl_null_layout!(
    NullAnyUintType,
    Uint08,
    Uint16,
    Uint24,
    Uint32,
    Uint40,
    Uint48,
    Uint56,
    Uint64
);

macro_rules! any_uint_from {
    ($var:ident, $inner:path) => {
        impl From<$inner> for NullAnyUintType {
            fn from(value: $inner) -> Self {
                Self::$var(value)
            }
        }

        impl From<UintColumnReader0<$inner>> for ReaderAnyUintType {
            fn from(value: UintColumnReader0<$inner>) -> Self {
                Self::$var(value)
            }
        }

        impl<'a> From<UintColumnWriter0<'a, $inner>> for WriterAnyUintType<'a> {
            fn from(value: UintColumnWriter0<'a, $inner>) -> Self {
                Self::$var(value)
            }
        }
    };
}

type UintColumnReader0<C> = ColumnReader0<C, <C as HasNativeType>::Native, Endian>;
type UintColumnWriter0<'a, C> = ColumnWriter0<'a, C, <C as HasNativeType>::Native, Endian>;

any_uint_from!(Uint08, Uint08Type);
any_uint_from!(Uint16, Uint16Type);
any_uint_from!(Uint24, Uint24Type);
any_uint_from!(Uint32, Uint32Type);
any_uint_from!(Uint40, Uint40Type);
any_uint_from!(Uint48, Uint48Type);
any_uint_from!(Uint56, Uint56Type);
any_uint_from!(Uint64, Uint64Type);

type NullMixedType = MixedType<ColumnNullFamily>;
type NullAnyUintType = AnyUintType<ColumnNullFamily>;

type ReaderMixedType = MixedType<ColumnReaderFamily>;
type ReaderAnyUintType = AnyUintType<ColumnReaderFamily>;

type WriterMixedType<'a> = MixedType<ColumnWriterFamily<'a>>;
type WriterAnyUintType<'a> = AnyUintType<ColumnWriterFamily<'a>>;

impl Copy for NullMixedType {}
impl Copy for NullAnyUintType {}

type NativeWrapper<F, C> =
    <F as ColumnFamily>::ColumnWrapper<C, <C as HasNativeType>::Native, Endian>;

struct ColumnNullFamily;

struct ColumnReaderFamily;

struct ColumnWriterFamily<'a>(std::marker::PhantomData<&'a ()>);

#[derive(Clone, Serialize)]
struct MaybeTot;

#[derive(Clone, Serialize)]
struct KnownTot;

trait TotDefinition {
    type Tot;

    fn with_tot<F, G, I, X>(input: I, tot: Self::Tot, tot_f: F, notot_f: G) -> X
    where
        F: FnOnce(I, Tot) -> X,
        G: FnOnce(I) -> X;

    fn check_tot(
        total_events: usize,
        tot: Self::Tot,
        allow_mismatch: bool,
    ) -> BiTentative<(), TotEventMismatch> {
        Self::with_tot(
            (),
            tot,
            |_, t| Self::check_tot_inner(total_events, t, allow_mismatch),
            |_| Tentative::new1(()),
        )
    }

    fn check_tot_inner(
        total_events: usize,
        tot: Tot,
        allow_mismatch: bool,
    ) -> BiTentative<(), TotEventMismatch> {
        if tot.0 != total_events {
            let i = TotEventMismatch { tot, total_events };
            Tentative::new_either((), vec![i], !allow_mismatch)
        } else {
            Tentative::new1(())
        }
    }
}

impl TotDefinition for MaybeTot {
    type Tot = Option<Tot>;

    fn with_tot<F, G, I, X>(input: I, tot: Self::Tot, tot_f: F, notot_f: G) -> X
    where
        F: FnOnce(I, Tot) -> X,
        G: FnOnce(I) -> X,
    {
        if let Some(t) = tot {
            tot_f(input, t)
        } else {
            notot_f(input)
        }
    }
}

impl TotDefinition for KnownTot {
    type Tot = Tot;

    fn with_tot<F, G, I, X>(input: I, tot: Self::Tot, tot_f: F, _: G) -> X
    where
        F: FnOnce(I, Tot) -> X,
        G: FnOnce(I) -> X,
    {
        tot_f(input, tot)
    }
}

trait ColumnFamily {
    type ColumnWrapper<C, T, S>;

    // fn column_type<C, T, S>(x: Self::ColumnWrapper<C, T, S>) -> C;
}

impl ColumnFamily for ColumnNullFamily {
    type ColumnWrapper<C, T, S> = C;

    // fn column_type<C, T>(x: Self::ColumnWrapper<C, T>) -> C {
    //     x
    // }
}

impl ColumnFamily for ColumnReaderFamily {
    type ColumnWrapper<C, T, S> = ColumnReader0<C, T, S>;

    // fn column_type<C, T>(x: Self::ColumnWrapper<C, T, S>) -> C {
    //     x.column_type
    // }
}

impl<'a> ColumnFamily for ColumnWriterFamily<'a> {
    type ColumnWrapper<C, T, S> = ColumnWriter0<'a, C, T, S>;

    // fn column_type<C, T>(x: Self::ColumnWrapper<C, T, S>) -> C {
    //     x.column_type
    // }
}

macro_rules! any_uint_to_width {
    ($from:ident, $to:ident) => {
        impl TryFrom<NullAnyUintType> for $to {
            type Error = UintToUintError;
            fn try_from(value: NullAnyUintType) -> Result<Self, Self::Error> {
                let w = value.nbytes();
                if let AnyUintType::$from(x) = value {
                    Ok(x)
                } else {
                    Err(UintToUintError {
                        from: w,
                        to: Self::BYTES.into(),
                    })
                }
            }
        }
    };
}

pub struct UintToUintError {
    from: u8,
    to: u8,
}

impl fmt::Display for UintToUintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "could not convert integer from {} bytes to {} bytes",
            self.from, self.to,
        )
    }
}

any_uint_to_width!(Uint08, Uint08Type);
any_uint_to_width!(Uint16, Uint16Type);
any_uint_to_width!(Uint24, Uint24Type);
any_uint_to_width!(Uint32, Uint32Type);
any_uint_to_width!(Uint40, Uint40Type);
any_uint_to_width!(Uint48, Uint48Type);
any_uint_to_width!(Uint56, Uint56Type);
any_uint_to_width!(Uint64, Uint64Type);

macro_rules! mixed_to_width {
    ($from:ident, $to:ident) => {
        impl TryFrom<NullMixedType> for $to {
            type Error = MixedToOrderedUintError;
            fn try_from(value: NullMixedType) -> Result<Self, Self::Error> {
                let w = value.nbytes();
                match value {
                    MixedType::Uint(x) => {
                        if let AnyUintType::$from(y) = x {
                            Ok(y)
                        } else {
                            Err(UintToUintError {
                                from: w,
                                to: Self::BYTES.into(),
                            }
                            .into())
                        }
                    }
                    MixedType::Ascii(_) => Err(MixedIsAscii.into()),
                    MixedType::F32(_) => Err(MixedIsFloat.into()),
                    MixedType::F64(_) => Err(MixedIsDouble.into()),
                }
            }
        }
    };
}

mixed_to_width!(Uint08, Uint08Type);
mixed_to_width!(Uint16, Uint16Type);
mixed_to_width!(Uint24, Uint24Type);
mixed_to_width!(Uint32, Uint32Type);
mixed_to_width!(Uint40, Uint40Type);
mixed_to_width!(Uint48, Uint48Type);
mixed_to_width!(Uint56, Uint56Type);
mixed_to_width!(Uint64, Uint64Type);

impl TryFrom<NullMixedType> for AsciiType {
    type Error = MixedToAsciiError;
    fn try_from(value: NullMixedType) -> Result<Self, Self::Error> {
        match value {
            MixedType::Ascii(x) => Ok(x),
            MixedType::Uint(x) => Err(MixedIsInteger { width: x.nbytes() }.into()),
            MixedType::F32(_) => Err(MixedIsFloat.into()),
            MixedType::F64(_) => Err(MixedIsDouble.into()),
        }
    }
}

impl TryFrom<NullMixedType> for NullAnyUintType {
    type Error = MixedToEndianUintError;
    fn try_from(value: NullMixedType) -> Result<Self, Self::Error> {
        match value {
            MixedType::Ascii(_) => Err(MixedIsAscii.into()),
            MixedType::Uint(x) => Ok(x),
            MixedType::F32(_) => Err(MixedIsFloat.into()),
            MixedType::F64(_) => Err(MixedIsDouble.into()),
        }
    }
}

impl TryFrom<NullMixedType> for F32Type {
    type Error = MixedToFloatError;
    fn try_from(value: NullMixedType) -> Result<Self, Self::Error> {
        match value {
            MixedType::Ascii(_) => Err(MixedIsAscii.into()),
            MixedType::Uint(x) => Err(MixedIsInteger { width: x.nbytes() }.into()),
            MixedType::F32(x) => Ok(x),
            MixedType::F64(_) => Err(MixedIsDouble.into()),
        }
    }
}

impl TryFrom<NullMixedType> for F64Type {
    type Error = MixedToDoubleError;
    fn try_from(value: NullMixedType) -> Result<Self, Self::Error> {
        match value {
            MixedType::Ascii(_) => Err(MixedIsAscii.into()),
            MixedType::Uint(x) => Err(MixedIsInteger { width: x.nbytes() }.into()),
            MixedType::F32(_) => Err(MixedIsFloat.into()),
            MixedType::F64(x) => Ok(x),
        }
    }
}

enum_from!(
    pub MixedToOrderedUintError,
    [IsAscii, MixedIsAscii],
    [IsWrongInteger, UintToUintError],
    [IsFloat, MixedIsFloat],
    [IsDouble, MixedIsDouble]
);

impl fmt::Display for MixedToOrderedUintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            Self::IsWrongInteger(e) => write!(
                f,
                "could not convert mixed from {}- to {}-byte integer",
                e.from, e.to
            ),
            Self::IsAscii(e) => write!(f, "could not convert mixed from {e} to integer"),
            Self::IsFloat(e) => write!(f, "could not convert mixed from {e} to integer"),
            Self::IsDouble(e) => write!(f, "could not convert mixed from {e} to integer"),
        }
    }
}

enum_from!(
    pub MixedToEndianUintError,
    [IsAscii, MixedIsAscii],
    [IsFloat, MixedIsFloat],
    [IsDouble, MixedIsDouble]
);

impl fmt::Display for MixedToEndianUintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            Self::IsAscii(e) => write!(f, "could not convert mixed from {e} to integer"),
            Self::IsFloat(e) => write!(f, "could not convert mixed from {e} to integer"),
            Self::IsDouble(e) => write!(f, "could not convert mixed from {e} to integer"),
        }
    }
}

enum_from!(
    pub MixedToAsciiError,
    [IsInteger, MixedIsInteger],
    [IsFloat, MixedIsFloat],
    [IsDouble, MixedIsDouble]
);

impl fmt::Display for MixedToAsciiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        let e = match self {
            Self::IsInteger(e) => e.to_string(),
            Self::IsFloat(e) => e.to_string(),
            Self::IsDouble(e) => e.to_string(),
        };
        write!(f, "could not convert mixed from {e} to ASCII")
    }
}

enum_from!(
    pub MixedToFloatError,
    [IsAscii, MixedIsAscii],
    [IsInteger, MixedIsInteger],
    [IsDouble, MixedIsDouble]
);

impl fmt::Display for MixedToFloatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        let e = match self {
            Self::IsAscii(e) => e.to_string(),
            Self::IsInteger(e) => e.to_string(),
            Self::IsDouble(e) => e.to_string(),
        };
        write!(f, "could not convert mixed from {e} to float")
    }
}

enum_from!(
    pub MixedToDoubleError,
    [IsAscii, MixedIsAscii],
    [IsInteger, MixedIsInteger],
    [IsFloat, MixedIsFloat]
);

impl fmt::Display for MixedToDoubleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        let e = match self {
            Self::IsAscii(e) => e.to_string(),
            Self::IsInteger(e) => e.to_string(),
            Self::IsFloat(e) => e.to_string(),
        };
        write!(f, "could not convert mixed from {e} to double")
    }
}

pub struct MixedIsInteger {
    width: u8,
}

pub struct MixedIsAscii;

pub struct MixedIsFloat;

pub struct MixedIsDouble;

impl fmt::Display for MixedIsInteger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "{}-byte integer", self.width)
    }
}

impl fmt::Display for MixedIsAscii {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "fixed-width ASCII")
    }
}

impl fmt::Display for MixedIsFloat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "32-bit float")
    }
}

impl fmt::Display for MixedIsDouble {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "64-bit float")
    }
}

pub struct AnalysisReader {
    pub seg: AnyAnalysisSegment,
}

pub struct OthersReader<'a> {
    pub segs: &'a [OtherSegment],
}

impl AnalysisReader {
    pub(crate) fn h_read<R: Read + Seek>(&self, h: &mut BufReader<R>) -> io::Result<Analysis> {
        let mut buf = vec![];
        self.seg.inner.h_read_contents(h, &mut buf)?;
        Ok(buf.into())
    }
}

impl OthersReader<'_> {
    pub(crate) fn h_read<R: Read + Seek>(&self, h: &mut BufReader<R>) -> io::Result<Others> {
        let mut buf = vec![];
        let mut others = vec![];
        for s in self.segs.iter() {
            s.inner.h_read_contents(h, &mut buf)?;
            others.push(Other(buf.clone()));
            buf.clear();
        }
        Ok(Others(others))
    }
}

/// A version-specific data layout
pub trait VersionedDataLayout: Sized {
    type S;
    type D;
    type T;

    fn try_new(
        dt: AlphaNumType,
        size: Self::S,
        cs: NonEmpty<ColumnLayoutValues<Self::D>>,
        conf: &SharedConfig,
    ) -> DeferredResult<Self, ColumnError<BitmaskError>, NewDataLayoutError>;

    fn lookup(
        kws: &mut StdKeywords,
        conf: &SharedConfig,
        par: Par,
    ) -> LookupLayoutResult<Option<Self>>;

    fn lookup_ro(kws: &StdKeywords, conf: &SharedConfig) -> FromRawResult<Option<Self>>;

    fn ncols(&self) -> usize;

    fn h_read_dataframe<R: Read>(
        &self,
        h: &mut BufReader<R>,
        tot: Self::T,
        seg: AnyDataSegment,
        conf: &ReaderConfig,
    ) -> IODeferredResult<FCSDataFrame, ReadWarning, ReadDataError0>;

    fn as_analysis_reader(
        kws: &mut StdKeywords,
        seg: HeaderAnalysisSegment,
        conf: &ReaderConfig,
    ) -> AnalysisReaderResult<AnalysisReader>;

    fn as_analysis_reader_raw(
        kws: &StdKeywords,
        seg: HeaderAnalysisSegment,
        conf: &ReaderConfig,
    ) -> AnalysisReaderResult<AnalysisReader>;

    fn check_writer<'a>(&self, df: &'a FCSDataFrame) -> MultiResult<(), AnyLossError>;

    fn h_write_df<'a, W: Write>(
        &self,
        h: &mut BufWriter<W>,
        df: &'a FCSDataFrame,
    ) -> io::Result<()> {
        // The dataframe should be encapsulated such that a) the column number
        // matches the number of measurements. If these are not true, the code
        // is wrong.
        let par = self.ncols();
        let ncols = df.ncols();
        if ncols != par {
            panic!("datafame columns ({ncols}) unequal to number of measurements ({par})");
        }
        self.h_write_df(h, df)
    }

    fn h_write_df_inner<'a, W: Write>(
        &self,
        h: &mut BufWriter<W>,
        df: &'a FCSDataFrame,
    ) -> io::Result<()>;

    fn layout_values(&self) -> LayoutValues<Self::S, Self::D>;
}

pub trait HasDatatype {
    const DATATYPE: AlphaNumType;
}

/// A type which has a width that may vary
pub trait IsFixed {
    fn nbytes(&self) -> u8;

    fn fixed_width(&self) -> BitsOrChars;

    fn range(&self) -> Range;
}

struct ColumnReader0<C, T, S> {
    column_type: C,
    data: Vec<T>,
    byte_layout: PhantomData<S>,
}

struct ColumnWriter0<'a, C, T, S> {
    column_type: C,
    data: AnySource<'a, T>,
    byte_layout: PhantomData<S>,
}

trait ToNativeReader: HasNativeType {
    fn into_reader<S>(self, nrows: usize) -> ColumnReader0<Self, Self::Native, S>
    where
        Self::Native: Default + Copy,
    {
        ColumnReader0 {
            column_type: self,
            data: vec![Self::Native::default(); nrows],
            byte_layout: PhantomData,
        }
    }
}

trait ToNativeWriter
where
    Self: HasNativeType,
{
    type Error;

    fn into_writer<'a, S>(self, c: &'a AnyFCSColumn) -> ColumnWriter0<'a, Self, Self::Native, S>
    where
        Self::Native: Default + Copy + AllFCSCast,
        AnySource<'a, Self::Native>: From<FCSColIter<'a, u8, Self::Native>>
            + From<FCSColIter<'a, u16, Self::Native>>
            + From<FCSColIter<'a, u32, Self::Native>>
            + From<FCSColIter<'a, u64, Self::Native>>
            + From<FCSColIter<'a, f32, Self::Native>>
            + From<FCSColIter<'a, f64, Self::Native>>,
    {
        ColumnWriter0 {
            column_type: self,
            data: AnySource::new::<Self::Native>(c),
            byte_layout: PhantomData,
        }
    }

    fn check_writer(&self, col: &AnyFCSColumn) -> Result<(), LossError<Self::Error>>
    where
        Self::Native: Default + Copy + AllFCSCast,
    {
        col.check_writer(|x| Self::check_other_loss(self, x))
    }

    fn check_other_loss(&self, x: Self::Native) -> Option<Self::Error>;
}

trait NativeReadable<S, E>: HasNativeType {
    type Buf;

    fn h_read<R: Read>(
        &self,
        h: &mut BufReader<R>,
        byte_layout: S,
        buf: &mut Self::Buf,
    ) -> IOResult<Self::Native, E>;
}

trait Readable<S, E> {
    type Inner;
    type Buf;

    fn new(column_type: Self::Inner, nrows: usize) -> Self;

    fn into_column(self) -> AnyFCSColumn;

    fn h_read_row<R: Read>(
        &mut self,
        h: &mut BufReader<R>,
        row: usize,
        byte_layout: S,
        buf: &mut Self::Buf,
    ) -> IOResult<(), E>;
}

trait NativeWritable<S>: HasNativeType {
    fn h_write<W: Write>(
        &self,
        h: &mut BufWriter<W>,
        x: CastResult<Self::Native>,
        byte_layout: S,
    ) -> io::Result<()>;
}

trait Writable<'a, S> {
    type Inner;

    fn new(column_type: Self::Inner, col: &'a AnyFCSColumn) -> Self;

    fn check_writer(column_type: Self::Inner, col: &'a AnyFCSColumn) -> Result<(), AnyLossError>;

    fn h_write<W: Write>(&mut self, h: &mut BufWriter<W>, byte_layout: S) -> io::Result<()>;
}

impl<T, const LEN: usize> ToNativeReader for UintType<T, LEN> where Self: HasNativeType<Native = T> {}

impl<T, const LEN: usize> ToNativeReader for FloatType<T, LEN> where Self: HasNativeType<Native = T> {}

impl ToNativeReader for AsciiType {}

impl<T, const LEN: usize, E> NativeReadable<Endian, E> for UintType<T, LEN>
where
    UintType<T, LEN>: HasNativeType<Native = T>,
    <UintType<T, LEN> as HasNativeType>::Native: Ord + Copy + IntFromBytes<LEN>,
{
    type Buf = ();

    fn h_read<R: Read>(
        &self,
        h: &mut BufReader<R>,
        byte_layout: Endian,
        _: &mut (),
    ) -> IOResult<Self::Native, E> {
        let x = Self::Native::h_read_endian(h, byte_layout)?;
        Ok(x)
    }
}

impl<T, const LEN: usize, E> NativeReadable<SizedByteOrd<LEN>, E> for UintType<T, LEN>
where
    UintType<T, LEN>: HasNativeType<Native = T>,
    <UintType<T, LEN> as HasNativeType>::Native: Ord + Copy + IntFromBytes<LEN>,
{
    type Buf = ();

    fn h_read<R: Read>(
        &self,
        h: &mut BufReader<R>,
        byte_layout: SizedByteOrd<LEN>,
        _: &mut (),
    ) -> IOResult<Self::Native, E> {
        let x = Self::Native::h_read_ordered(h, byte_layout)?;
        Ok(x)
    }
}

impl<T, const LEN: usize, E> NativeReadable<Endian, E> for FloatType<T, LEN>
where
    FloatType<T, LEN>: HasNativeType<Native = T>,
    <FloatType<T, LEN> as HasNativeType>::Native: Copy + FloatFromBytes<LEN>,
{
    type Buf = ();

    fn h_read<R: Read>(
        &self,
        h: &mut BufReader<R>,
        byte_layout: Endian,
        _: &mut (),
    ) -> IOResult<Self::Native, E> {
        let x = Self::Native::h_read_endian(h, byte_layout)?;
        Ok(x)
    }
}

impl<T, const LEN: usize, E> NativeReadable<SizedByteOrd<LEN>, E> for FloatType<T, LEN>
where
    FloatType<T, LEN>: HasNativeType<Native = T>,
    <FloatType<T, LEN> as HasNativeType>::Native: Copy + FloatFromBytes<LEN>,
{
    type Buf = ();

    fn h_read<R: Read>(
        &self,
        h: &mut BufReader<R>,
        byte_layout: SizedByteOrd<LEN>,
        _: &mut (),
    ) -> IOResult<Self::Native, E> {
        let x = Self::Native::h_read_ordered(h, byte_layout)?;
        Ok(x)
    }
}

impl NativeReadable<(), AsciiToUintError> for AsciiType {
    type Buf = Vec<u8>;

    fn h_read<R: Read>(
        &self,
        h: &mut BufReader<R>,
        _: (),
        buf: &mut Vec<u8>,
    ) -> IOResult<Self::Native, AsciiToUintError> {
        buf.clear();
        h.take(u8::from(self.chars).into()).read_to_end(buf)?;
        ascii_to_uint(&buf).map_err(ImpureError::Pure)
    }
}

impl<C, T, S, E> Readable<S, E> for ColumnReader0<C, T, S>
where
    T: Copy + Default,
    C: NativeReadable<S, E> + HasNativeType<Native = T> + ToNativeReader,
    AnyFCSColumn: From<FCSColumn<T>>,
{
    type Inner = C;
    type Buf = <C as NativeReadable<S, E>>::Buf;
    // type Error = <C as NativeReadable<S>>::Error;

    fn new(column_type: Self::Inner, nrows: usize) -> Self {
        column_type.into_reader(nrows)
    }

    fn into_column(self) -> AnyFCSColumn {
        FCSColumn::from(self.data).into()
    }

    fn h_read_row<R: Read>(
        &mut self,
        h: &mut BufReader<R>,
        row: usize,
        byte_layout: S,
        buf: &mut Self::Buf,
    ) -> IOResult<(), E> {
        self.data[row] = self.column_type.h_read(h, byte_layout, buf)?;
        Ok(())
    }
}

impl Readable<Endian, AsciiToUintError> for ReaderMixedType {
    type Inner = NullMixedType;
    type Buf = Vec<u8>;

    fn new(column_type: Self::Inner, nrows: usize) -> Self {
        match column_type {
            MixedType::Ascii(c) => Self::Ascii(c.into_reader(nrows)),
            MixedType::Uint(c) => Self::Uint(Readable::<_, AsciiToUintError>::new(c, nrows)),
            MixedType::F32(c) => Self::F32(c.into_reader(nrows)),
            MixedType::F64(c) => Self::F64(c.into_reader(nrows)),
        }
    }

    fn into_column(self) -> AnyFCSColumn {
        match self {
            MixedType::Ascii(c) => c.into_column(),
            MixedType::Uint(c) => Readable::<_, AsciiToUintError>::into_column(c),
            MixedType::F32(c) => Readable::<_, AsciiToUintError>::into_column(c),
            MixedType::F64(c) => Readable::<_, AsciiToUintError>::into_column(c),
        }
    }

    fn h_read_row<R: Read>(
        &mut self,
        h: &mut BufReader<R>,
        row: usize,
        byte_layout: Endian,
        buf: &mut Self::Buf,
    ) -> IOResult<(), AsciiToUintError> {
        match self {
            MixedType::Ascii(c) => c.h_read_row(h, row, (), buf),
            MixedType::Uint(c) => c
                .h_read_row(h, row, byte_layout, &mut ())
                .map_err(|e| e.infallible()),
            MixedType::F32(c) => c
                .h_read_row(h, row, byte_layout, &mut ())
                .map_err(|e| e.infallible()),
            MixedType::F64(c) => c
                .h_read_row(h, row, byte_layout, &mut ())
                .map_err(|e| e.infallible()),
        }
    }
}

impl<E> Readable<Endian, E> for ReaderAnyUintType {
    type Inner = NullAnyUintType;
    type Buf = ();

    fn new(column_type: Self::Inner, nrows: usize) -> Self {
        match_many_to_one!(
            column_type,
            AnyUintType,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            c,
            { c.into_reader(nrows).into() }
        )
    }

    fn into_column(self) -> AnyFCSColumn {
        match_many_to_one!(
            self,
            AnyUintType,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            c,
            { Readable::<_, E>::into_column(c) }
        )
    }

    fn h_read_row<R: Read>(
        &mut self,
        h: &mut BufReader<R>,
        row: usize,
        byte_layout: Endian,
        buf: &mut Self::Buf,
    ) -> IOResult<(), E> {
        match_many_to_one!(
            self,
            AnyUintType,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            c,
            { c.h_read_row(h, row, byte_layout, buf) }
        )
    }
}

impl<T, const LEN: usize> NativeWritable<Endian> for UintType<T, LEN>
where
    UintType<T, LEN>: HasNativeType<Native = T>,
    <UintType<T, LEN> as HasNativeType>::Native: Ord + Copy + IntFromBytes<LEN>,
{
    fn h_write<W: Write>(
        &self,
        h: &mut BufWriter<W>,
        x: CastResult<Self::Native>,
        byte_layout: Endian,
    ) -> io::Result<()> {
        x.new.min(self.bitmask).h_write_endian(h, byte_layout)
    }
}

impl<T, const LEN: usize> NativeWritable<SizedByteOrd<LEN>> for UintType<T, LEN>
where
    UintType<T, LEN>: HasNativeType<Native = T>,
    <UintType<T, LEN> as HasNativeType>::Native: Ord + Copy + IntFromBytes<LEN>,
{
    fn h_write<W: Write>(
        &self,
        h: &mut BufWriter<W>,
        x: CastResult<Self::Native>,
        byte_layout: SizedByteOrd<LEN>,
    ) -> io::Result<()> {
        x.new.min(self.bitmask).h_write_ordered(h, byte_layout)
    }
}

impl<T, const LEN: usize> NativeWritable<Endian> for FloatType<T, LEN>
where
    FloatType<T, LEN>: HasNativeType<Native = T>,
    <FloatType<T, LEN> as HasNativeType>::Native: Copy + FloatFromBytes<LEN>,
{
    fn h_write<W: Write>(
        &self,
        h: &mut BufWriter<W>,
        x: CastResult<Self::Native>,
        byte_layout: Endian,
    ) -> io::Result<()> {
        x.new.h_write_endian(h, byte_layout)
    }
}

impl<T, const LEN: usize> NativeWritable<SizedByteOrd<LEN>> for FloatType<T, LEN>
where
    FloatType<T, LEN>: HasNativeType<Native = T>,
    <FloatType<T, LEN> as HasNativeType>::Native: Copy + FloatFromBytes<LEN>,
{
    fn h_write<W: Write>(
        &self,
        h: &mut BufWriter<W>,
        x: CastResult<Self::Native>,
        byte_layout: SizedByteOrd<LEN>,
    ) -> io::Result<()> {
        x.new.h_write_ordered(h, byte_layout)
    }
}

impl NativeWritable<()> for AsciiType {
    fn h_write<W: Write>(
        &self,
        h: &mut BufWriter<W>,
        x: CastResult<Self::Native>,
        _: (),
    ) -> io::Result<()> {
        let s = x.new.to_string();
        let w: usize = u8::from(self.chars).into();
        if s.len() > w {
            // if string is greater than allocated chars, only write a fraction
            // starting from the left
            let offset = s.len() - w;
            h.write_all(&s.as_bytes()[offset..])
        } else {
            // if string less than allocated chars, pad left side with zero before
            // writing number
            for _ in 0..(w - s.len()) {
                h.write_all(&[30])?;
            }
            h.write_all(s.as_bytes())
        }
    }
}

impl<'a, C, T, S> Writable<'a, S> for ColumnWriter0<'a, C, T, S>
where
    C: NativeWritable<S> + HasNativeType<Native = T> + ToNativeWriter,
    C::Native: AllFCSCast + Copy + Default,
    AnyLossError: From<LossError<<C as ToNativeWriter>::Error>>,
    AnySource<'a, C::Native>: From<FCSColIter<'a, u8, C::Native>>
        + From<FCSColIter<'a, u16, C::Native>>
        + From<FCSColIter<'a, u32, C::Native>>
        + From<FCSColIter<'a, u64, C::Native>>
        + From<FCSColIter<'a, f32, C::Native>>
        + From<FCSColIter<'a, f64, C::Native>>,
{
    type Inner = C;

    fn new(column_type: Self::Inner, col: &'a AnyFCSColumn) -> Self {
        column_type.into_writer(col)
    }

    fn check_writer(column_type: Self::Inner, col: &'a AnyFCSColumn) -> Result<(), AnyLossError> {
        column_type.check_writer(col).map_err(|e| e.into())
    }

    fn h_write<W: Write>(&mut self, h: &mut BufWriter<W>, byte_layout: S) -> io::Result<()> {
        let x = self.data.next().unwrap();
        self.column_type.h_write(h, x, byte_layout)
    }
}

impl<'a> Writable<'a, Endian> for WriterMixedType<'a> {
    type Inner = NullMixedType;

    fn new(column_type: Self::Inner, col: &'a AnyFCSColumn) -> Self {
        match column_type {
            MixedType::Ascii(c) => Self::Ascii(c.into_writer(col)),
            MixedType::Uint(c) => Self::Uint(WriterAnyUintType::new(c, col)),
            MixedType::F32(c) => Self::F32(c.into_writer(col)),
            MixedType::F64(c) => Self::F64(c.into_writer(col)),
        }
    }

    fn check_writer(column_type: Self::Inner, col: &'a AnyFCSColumn) -> Result<(), AnyLossError> {
        match column_type {
            MixedType::Ascii(c) => c.check_writer(col).map_err(|e| e.into()),
            MixedType::Uint(c) => WriterAnyUintType::check_writer(c, col),
            MixedType::F32(c) => c.check_writer(col).map_err(|e| e.into()),
            MixedType::F64(c) => c.check_writer(col).map_err(|e| e.into()),
        }
    }

    fn h_write<W: Write>(&mut self, h: &mut BufWriter<W>, byte_layout: Endian) -> io::Result<()> {
        match self {
            Self::Ascii(c) => {
                let x = c.data.next().unwrap();
                c.column_type.h_write(h, x, ())
            }
            Self::Uint(c) => c.h_write(h, byte_layout),
            Self::F32(c) => {
                let x = c.data.next().unwrap();
                c.column_type.h_write(h, x, byte_layout)
            }
            Self::F64(c) => {
                let x = c.data.next().unwrap();
                c.column_type.h_write(h, x, byte_layout)
            }
        }
    }
}

impl<'a> Writable<'a, Endian> for WriterAnyUintType<'a> {
    type Inner = NullAnyUintType;

    fn new(column_type: Self::Inner, col: &'a AnyFCSColumn) -> Self {
        match_many_to_one!(
            column_type,
            AnyUintType,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            c,
            { c.into_writer(col).into() }
        )
    }

    fn check_writer(column_type: Self::Inner, col: &'a AnyFCSColumn) -> Result<(), AnyLossError> {
        match_many_to_one!(
            column_type,
            AnyUintType,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            c,
            { c.check_writer(col).map_err(|e| e.into()) }
        )
    }

    fn h_write<W: Write>(&mut self, h: &mut BufWriter<W>, byte_layout: Endian) -> io::Result<()> {
        match_many_to_one!(
            self,
            Self,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            c,
            {
                let x = c.data.next().unwrap();
                c.column_type.h_write(h, x, byte_layout)
            }
        )
    }
}

impl<T, const LEN: usize> ToNativeWriter for UintType<T, LEN>
where
    Self: HasNativeType<Native = T>,
    u64: From<Self::Native>,
    Self::Native: Ord + Copy,
{
    type Error = BitmaskLossError;

    fn check_other_loss(&self, x: Self::Native) -> Option<Self::Error> {
        if x > self.bitmask {
            Some(BitmaskLossError(u64::from(self.bitmask)))
        } else {
            None
        }
    }
}

impl<T, const LEN: usize> ToNativeWriter for FloatType<T, LEN>
where
    Self: HasNativeType<Native = T>,
{
    type Error = Infallible;

    fn check_other_loss(&self, _: Self::Native) -> Option<Self::Error> {
        None
    }
}

impl ToNativeWriter for AsciiType {
    type Error = AsciiLossError;

    fn check_other_loss(&self, x: Self::Native) -> Option<Self::Error> {
        let width = u8::from(self.chars);
        if ascii_nbytes(x) > width.into() {
            Some(AsciiLossError(width))
        } else {
            None
        }
    }
}

// /// A type which is may read bytes in a fixed width
// pub trait IsFixedReader {
//     type S;
//     fn into_col_reader(self, nrows: usize, byte_layout: Self::S) -> AlphaNumColumnReader;
// }

// /// A type which is may write bytes in a fixed width
// pub trait IsFixedWriter {
//     type S;
//     fn into_col_writer(
//         self,
//         c: &AnyFCSColumn,
//         check: bool,
//         byte_layout: Self::S,
//     ) -> Result<AnyFixedColumnWriter, AnyLossError>;
// }

impl NullAnyUintType {
    fn try_new<D>(
        c: ColumnLayoutValues<D>,
        notrunc: bool,
    ) -> DeferredResult<Self, BitmaskError, NewUintTypeError> {
        let r = c.range;
        c.width
            .try_into()
            .into_deferred()
            .def_and_tentatively(|bytes: Bytes| {
                // ASSUME this can only be 1-8
                match u8::from(bytes) {
                    1 => u8::column_type(r, notrunc).map(Self::Uint08),
                    2 => u16::column_type(r, notrunc).map(Self::Uint16),
                    3 => u32::column_type(r, notrunc).map(Self::Uint24),
                    4 => u32::column_type(r, notrunc).map(Self::Uint32),
                    5 => u64::column_type(r, notrunc).map(Self::Uint40),
                    6 => u64::column_type(r, notrunc).map(Self::Uint48),
                    7 => u64::column_type(r, notrunc).map(Self::Uint56),
                    8 => u64::column_type(r, notrunc).map(Self::Uint64),
                    _ => unreachable!(),
                }
                .errors_into()
            })
    }

    pub(crate) fn try_into_one_size<X, E, T>(
        self,
        tail: Vec<X>,
        endian: Endian,
        starting_index: usize,
    ) -> MultiResult<AnyOrderedUintLayout<T>, (MeasIndex, E)>
    where
        Uint08Type: TryFrom<X, Error = E>,
        Uint16Type: TryFrom<X, Error = E>,
        Uint24Type: TryFrom<X, Error = E>,
        Uint32Type: TryFrom<X, Error = E>,
        Uint40Type: TryFrom<X, Error = E>,
        Uint48Type: TryFrom<X, Error = E>,
        Uint56Type: TryFrom<X, Error = E>,
        Uint64Type: TryFrom<X, Error = E>,
    {
        match_many_to_one!(
            self,
            Self,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            x,
            {
                UintType::try_from_many(tail, starting_index)
                    .map(|xs| FixedLayout::new1(x, xs, endian.into()).into())
            }
        )
    }

    fn as_reader(&self, nrows: usize) -> ReaderAnyUintType {
        match_many_to_one!(
            self,
            Self,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            c,
            { c.into_reader(nrows).into() }
        )
    }

    // fn as_writer<'a>(
    //     &self,
    //     xs: &'a AnyFCSColumn,
    // ) -> Result<WriterAnyUintType<'a>, LossError<BitmaskLossError>> {
    //     match_many_to_one!(
    //         self,
    //         Self,
    //         [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
    //         c,
    //         { c.into_writer(xs, false).map(|w| w.into()) }
    //     )
    // }
}

impl<T, const LEN: usize> UintType<T, LEN> {
    fn try_from_many<E, X>(
        xs: Vec<X>,
        starting_index: usize,
    ) -> MultiResult<Vec<Self>, (MeasIndex, E)>
    where
        Self: TryFrom<X, Error = E>,
    {
        xs.into_iter()
            .enumerate()
            .map(|(i, c)| {
                Self::try_from(c)
                    .map_err(|e| ((i + starting_index).into(), e))
                    .map(UintType::from)
            })
            .gather()
    }
}

macro_rules! uint_to_mixed {
    ($uint:ident, $wrap:ident) => {
        impl From<$uint> for NullMixedType {
            fn from(value: $uint) -> Self {
                MixedType::Uint(AnyUintType::$wrap(value))
            }
        }
    };
}

uint_to_mixed!(Uint08Type, Uint08);
uint_to_mixed!(Uint16Type, Uint16);
uint_to_mixed!(Uint24Type, Uint24);
uint_to_mixed!(Uint32Type, Uint32);
uint_to_mixed!(Uint40Type, Uint40);
uint_to_mixed!(Uint48Type, Uint48);
uint_to_mixed!(Uint56Type, Uint56);
uint_to_mixed!(Uint64Type, Uint64);

/// Instructions for writing measurements to a file.
///
/// This structure can be used with all FCS versions, as each column is treated
/// as it's own separate type. This is in contrast to some FCS versions where
/// we could think of the DATA as a matrix of one uniform type.
///
/// This doesn't store data, but rather stores an iterator pointing to a column
/// which is then called with 'next()' for each row/event. The dataframe is
/// validated to store only u8-64 and f32/64, which are then "coerced" to
/// whatever type the measurement column requires. This might cause data loss,
/// in which case the user will be warned (for instance, a f32 column might be
/// stored as a u32 on disk, which will likely cause loss of precision and/or
/// truncation).
///
/// We could be more draconian about ensuring the column type matches the
/// measurement type, but this would complicate many other operations such as
/// adding/removing columns or changing a measurement type/size/range. The price
/// to pay with this approach is that each combination of to/from types needs to
/// be enumerated (6 and 11 types respectively).
pub enum DataWriter<'a> {
    Delim(DelimWriter<'a>),
    Fixed(FixedWriter<'a>),
}

pub type DelimWriter<'a> = DataWriterInner<DelimColumnWriter<'a>>;
pub type FixedWriter<'a> = DataWriterInner<AnyFixedColumnWriter<'a>>;

pub struct DataWriterInner<C> {
    columns: NonEmpty<C>,
    nrows: usize,
    nbytes: usize,
}

/// Writer for one column.
///
/// This encodes the target type for encoding any FCS measurement, which may be
/// integers of varying sizes, floats, doubles, or a string of numbers. Each
/// type has its own size information. The generic type encodes the source type
/// within the polars column from which the written type will be converted,
/// possibly with data loss.
pub enum AnyFixedColumnWriter<'a> {
    U08(IntColumnWriter<'a, u8, 1>),
    U16(IntColumnWriter<'a, u16, 2>),
    U24(IntColumnWriter<'a, u32, 3>),
    U32(IntColumnWriter<'a, u32, 4>),
    U40(IntColumnWriter<'a, u64, 5>),
    U48(IntColumnWriter<'a, u64, 6>),
    U56(IntColumnWriter<'a, u64, 7>),
    U64(IntColumnWriter<'a, u64, 8>),
    F32(FloatColumnWriter<'a, f32, 4>),
    F64(FloatColumnWriter<'a, f64, 8>),
    Ascii(AsciiColumnWriter<'a>),
}

pub struct OrderedUintType<T, const LEN: usize> {
    bitmask: T,
    byte_layout: SizedByteOrd<LEN>,
}

pub type IntColumnWriter<'a, T, const LEN: usize> = ColumnWriter<'a, T, OrderedUintType<T, LEN>>;

pub type FloatColumnWriter<'a, T, const LEN: usize> = ColumnWriter<'a, T, SizedByteOrd<LEN>>;

pub type AsciiColumnWriter<'a> = ColumnWriter<'a, u64, Chars>;

pub type DelimColumnWriter<'a> = ColumnWriter<'a, u64, ()>;

pub struct ColumnWriter<'a, T, S> {
    pub(crate) data: AnySource<'a, T>,
    pub(crate) size: S,
}

/// A wrapper for any of the 6 source types that can be written.
///
/// Each inner type is an iterator from a different source type which emit
/// the given target type.
pub enum AnySource<'a, TargetType> {
    FromU08(FCSColIter<'a, u8, TargetType>),
    FromU16(FCSColIter<'a, u16, TargetType>),
    FromU32(FCSColIter<'a, u32, TargetType>),
    FromU64(FCSColIter<'a, u64, TargetType>),
    FromF32(FCSColIter<'a, f32, TargetType>),
    FromF64(FCSColIter<'a, f64, TargetType>),
}

impl DataWriter<'_> {
    pub(crate) fn h_write<W: Write>(&mut self, h: &mut BufWriter<W>) -> io::Result<()> {
        match self {
            Self::Delim(d) => d.h_write(h),
            Self::Fixed(f) => f.h_write(h),
        }
    }

    pub(crate) fn nbytes(&self) -> usize {
        match self {
            Self::Delim(d) => d.nbytes,
            Self::Fixed(f) => f.nbytes,
        }
    }
}

impl DelimWriter<'_> {
    fn h_write<W: Write>(&mut self, h: &mut BufWriter<W>) -> io::Result<()> {
        let ncols = self.columns.len();
        let nrows = self.nrows;
        for i in 0..nrows {
            for (j, c) in self.columns.iter_mut().enumerate() {
                c.h_write_delim_ascii(h)?;
                // write delimiter after all but last value
                if !(i == nrows - 1 && j == ncols - 1) {
                    h.write_all(&[32])?; // 32 = space in ASCII
                }
            }
        }
        Ok(())
    }
}

impl FixedWriter<'_> {
    fn h_write<W: Write>(&mut self, h: &mut BufWriter<W>) -> io::Result<()> {
        for _ in 0..self.nrows {
            for c in self.columns.iter_mut() {
                c.h_write(h)?;
            }
        }
        Ok(())
    }
}

impl AnyFixedColumnWriter<'_> {
    fn h_write<W: Write>(&mut self, h: &mut BufWriter<W>) -> io::Result<()> {
        match self {
            Self::U08(c) => c.h_write_int(h),
            Self::U16(c) => c.h_write_int(h),
            Self::U24(c) => c.h_write_int(h),
            Self::U32(c) => c.h_write_int(h),
            Self::U40(c) => c.h_write_int(h),
            Self::U48(c) => c.h_write_int(h),
            Self::U56(c) => c.h_write_int(h),
            Self::U64(c) => c.h_write_int(h),
            Self::F32(c) => c.h_write_float(h),
            Self::F64(c) => c.h_write_float(h),
            Self::Ascii(c) => c.h_write_ascii(h),
        }
    }
}

impl<Y, const INTLEN: usize> IntColumnWriter<'_, Y, INTLEN> {
    fn h_write_int<W: Write>(&mut self, h: &mut BufWriter<W>) -> io::Result<()>
    where
        Y: IntFromBytes<INTLEN>,
        Y: Ord,
    {
        let x = self.data.next().unwrap();
        x.new
            .min(self.size.bitmask)
            .h_write_ordered(h, self.size.byte_layout)
    }
}

impl<Y, const DTLEN: usize> FloatColumnWriter<'_, Y, DTLEN> {
    fn h_write_float<W: Write>(&mut self, h: &mut BufWriter<W>) -> io::Result<()>
    where
        Y: FloatFromBytes<DTLEN>,
    {
        self.data.next().unwrap().new.h_write_ordered(h, self.size)
    }
}

impl AsciiColumnWriter<'_> {
    fn h_write_ascii<W: Write>(&mut self, h: &mut BufWriter<W>) -> io::Result<()> {
        let x = self.data.next().unwrap();
        let s = x.new.to_string();
        let w: usize = u8::from(self.size).into();
        if s.len() > w {
            // if string is greater than allocated chars, only write a fraction
            // starting from the left
            let offset = s.len() - w;
            h.write_all(&s.as_bytes()[offset..])
        } else {
            // if string less than allocated chars, pad left side with zero before
            // writing number
            for _ in 0..(w - s.len()) {
                h.write_all(&[30])?;
            }
            h.write_all(s.as_bytes())
        }
    }
}

impl DelimColumnWriter<'_> {
    fn h_write_delim_ascii<W: Write>(&mut self, h: &mut BufWriter<W>) -> io::Result<()> {
        let x = self.data.next().unwrap();
        let s = x.new.to_string();
        let buf = s.as_bytes();
        h.write_all(buf)
    }
}

impl<'a, T> AnySource<'a, T> {
    fn new<TargetType>(c: &'a AnyFCSColumn) -> Self
    where
        TargetType: AllFCSCast,
        Self: From<FCSColIter<'a, u8, TargetType>>
            + From<FCSColIter<'a, u16, TargetType>>
            + From<FCSColIter<'a, u32, TargetType>>
            + From<FCSColIter<'a, u64, TargetType>>
            + From<FCSColIter<'a, f32, TargetType>>
            + From<FCSColIter<'a, f64, TargetType>>,
    {
        match_many_to_one!(c, AnyFCSColumn, [U08, U16, U32, U64, F32, F64], xs, {
            FCSDataType::as_col_iter(xs).into()
        })
    }

    fn next(&mut self) -> Option<CastResult<T>> {
        match_many_to_one!(
            self,
            Self,
            [FromU08, FromU16, FromU32, FromU64, FromF32, FromF64],
            c,
            { c.next() }
        )
    }
}

/// Instructions and buffers to read the DATA segment
pub struct DataReader {
    pub column_reader: ColumnReader,
    pub seg: AnyDataSegment,
}

/// Instructions to read one column in the DATA segment.
///
/// Each "column" contains a vector to hold the numbers read from DATA. In all
/// but the case of delimited ASCII, this is pre-allocated with the number of
/// rows to make reading faster. Each column has other information necessary to
/// read the column (bitmask, width, etc).
pub enum ColumnReader {
    DelimitedAsciiNoRows(DelimAsciiReaderNoRows),
    DelimitedAscii(DelimAsciiReader),
    AlphaNum(AlphaNumReader),
}

// The only difference b/t these two is that the no-rows version will be
// initialized with zero-length vectors, and the rows version will be
// initialized with row-length vectors. The only purpose of the former is the
// deal with the case in 2.0 where $TOT isn't given
pub struct DelimAsciiReaderNoRows(DelimAsciiReaderInner);
pub struct DelimAsciiReader(DelimAsciiReaderInner);

pub struct DelimAsciiReaderInner {
    pub columns: NonEmpty<Vec<u64>>,
    pub nbytes: usize,
}

pub struct AlphaNumReader {
    pub columns: NonEmpty<AlphaNumColumnReader>,
}

pub enum AlphaNumColumnReader {
    Ascii(AsciiColumnReader),
    Uint(AnyUintColumnReader),
    Float(FloatReader),
}

pub enum FloatReader {
    F32(FloatColumnReader<f32, 4>),
    F64(FloatColumnReader<f64, 8>),
}

pub struct FloatColumnReader<T, const LEN: usize> {
    pub column: Vec<T>,
    pub byte_layout: SizedByteOrd<LEN>,
}

pub struct AsciiColumnReader {
    pub column: Vec<u64>,
    pub width: Chars,
}

pub struct UintColumnReader<B, const LEN: usize> {
    pub column: Vec<B>,
    pub uint_type: UintType<B, LEN>,
    pub size: SizedByteOrd<LEN>,
}

pub enum AnyUintColumnReader {
    Uint08(UintColumnReader<u8, 1>),
    Uint16(UintColumnReader<u16, 2>),
    Uint24(UintColumnReader<u32, 3>),
    Uint32(UintColumnReader<u32, 4>),
    Uint40(UintColumnReader<u64, 5>),
    Uint48(UintColumnReader<u64, 6>),
    Uint56(UintColumnReader<u64, 7>),
    Uint64(UintColumnReader<u64, 8>),
}

impl DataReader {
    pub(crate) fn h_read<R>(self, h: &mut BufReader<R>) -> IOResult<FCSDataFrame, ReadDataError>
    where
        R: Read + Seek,
    {
        // TODO it seems a bit odd that we would have an empty segment this
        // late in the process
        if let Some(begin) = self.seg.inner.try_coords().map(|(x, _)| x) {
            h.seek(SeekFrom::Start(begin))?;
            match self.column_reader {
                ColumnReader::DelimitedAscii(p) => p.h_read(h).map_err(|e| e.inner_into()),
                ColumnReader::DelimitedAsciiNoRows(p) => p.h_read(h).map_err(|e| e.inner_into()),
                ColumnReader::AlphaNum(p) => p.h_read(h).map_err(|e| e.inner_into()),
            }
        } else {
            Ok(FCSDataFrame::default())
        }
    }
}

impl ColumnReader {
    fn into_data_reader(self, seg: AnyDataSegment) -> DataReader {
        DataReader {
            column_reader: self,
            seg,
        }
    }
}

impl FloatReader {
    fn h_read<R: Read>(&mut self, h: &mut BufReader<R>, r: usize) -> io::Result<()> {
        match self {
            Self::F32(t) => t.h_read(h, r),
            Self::F64(t) => t.h_read(h, r),
        }
    }

    fn into_fcs_column(self) -> AnyFCSColumn {
        match self {
            Self::F32(x) => F32Column::from(x.column).into(),
            Self::F64(x) => F64Column::from(x.column).into(),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::F32(x) => x.column.len(),
            Self::F64(x) => x.column.len(),
        }
    }
}

impl DelimAsciiReader {
    fn h_read<R: Read>(
        self,
        h: &mut BufReader<R>,
    ) -> IOResult<FCSDataFrame, ReadDelimWithRowsAsciiError> {
        // FCS 2.0 files have an optional $TOT field, which complicates this a
        // bit. If in this case we have $TOT so the columns have been
        // initialized to the number of rows.
        let mut buf = Vec::new();
        let mut last_was_delim = false;
        let mut data = self.0.columns;
        let nrows = data.head.len();
        let ncols = data.len();
        let mut row = 0;
        let mut col = 0;
        // Delimiters are tab, newline, carriage return, space, or comma. Any
        // consecutive delimiter counts as one, and delimiters can be mixed.
        for b in h.bytes().take(self.0.nbytes) {
            let byte = b?;
            // exit if we encounter more rows than expected.
            if row == nrows {
                let e = ReadDelimWithRowsAsciiError::RowsExceeded(RowsExceededError(nrows));
                return Err(ImpureError::Pure(e));
            }
            if is_ascii_delim(byte) {
                if !last_was_delim {
                    last_was_delim = true;
                    data[col][row] = ascii_to_uint(&buf)
                        .map_err(ReadDelimWithRowsAsciiError::Parse)
                        .map_err(ImpureError::Pure)?;
                    buf.clear();
                    if col == ncols - 1 {
                        col = 0;
                        row += 1;
                    } else {
                        col += 1;
                    }
                }
            } else {
                buf.push(byte);
                last_was_delim = false;
            }
        }
        if !(col == 0 && row == nrows) {
            let e = DelimIncompleteError { col, row, nrows };
            return Err(ImpureError::Pure(ReadDelimWithRowsAsciiError::Incomplete(
                e,
            )));
        }
        // The spec isn't clear if the last value should be a delim or
        // not, so flush the buffer if it has anything in it since we
        // only try to parse if we hit a delim above.
        if !buf.is_empty() {
            data[col][row] = ascii_to_uint(&buf)
                .map_err(ReadDelimWithRowsAsciiError::Parse)
                .map_err(ImpureError::Pure)?;
        }
        let cs: Vec<_> = data
            .into_iter()
            .map(FCSColumn::from)
            .map(AnyFCSColumn::from)
            .collect();
        // ASSUME this will never fail because all columns should be the same
        // length
        Ok(FCSDataFrame::try_new(cs).unwrap())
    }
}

impl DelimAsciiReaderNoRows {
    fn h_read<R: Read>(
        self,
        h: &mut BufReader<R>,
    ) -> IOResult<FCSDataFrame, ReadDelimAsciiWithoutRowsError> {
        let mut buf = Vec::new();
        let mut data = self.0.columns;
        let ncols = data.len();
        let mut col = 0;
        let mut last_was_delim = false;
        let go = |_data: &mut NonEmpty<Vec<u64>>, _col, _buf: &[u8]| {
            ascii_to_uint(_buf)
                .map_err(ReadDelimAsciiWithoutRowsError::Parse)
                .map_err(ImpureError::Pure)
                .map(|x| _data[_col].push(x))
        };
        // Delimiters are tab, newline, carriage return, space, or comma. Any
        // consecutive delimiter counts as one, and delimiters can be mixed.
        // If we don't know the number of rows, the only choice is to push onto
        // the column vectors one at a time. This leads to the possibility that
        // the vectors may not be the same length in the end, in which case,
        // scream loudly and bail.
        for b in h.bytes().take(self.0.nbytes) {
            let byte = b?;
            if is_ascii_delim(byte) {
                if !last_was_delim {
                    last_was_delim = true;
                    buf.clear();
                    go(&mut data, col, &buf)?;
                    if col == ncols - 1 {
                        col = 0;
                    } else {
                        col += 1;
                    }
                }
            } else {
                buf.push(byte);
                last_was_delim = false;
            }
        }
        if data.iter().map(|c| c.len()).unique().count() > 1 {
            return Err(ImpureError::Pure(ReadDelimAsciiWithoutRowsError::Unequal));
        }
        // The spec isn't clear if the last value should be a delim or
        // not, so flush the buffer if it has anything in it since we
        // only try to parse if we hit a delim above.
        if !buf.is_empty() {
            go(&mut data, col, &buf)?;
        }
        let cs: Vec<_> = data
            .into_iter()
            .map(FCSColumn::from)
            .map(AnyFCSColumn::from)
            .collect();
        // ASSUME this will never fail because all columns should be the same
        // length
        Ok(FCSDataFrame::try_new(cs).unwrap())
    }
}

fn is_ascii_delim(x: u8) -> bool {
    // tab, newline, carriage return, space, or comma
    x == 9 || x == 10 || x == 13 || x == 32 || x == 44
}

impl AlphaNumReader {
    fn h_read<R: Read>(mut self, h: &mut BufReader<R>) -> IOResult<FCSDataFrame, AsciiToUintError> {
        let mut buf: Vec<u8> = vec![];
        let nrows = self.columns.head.len();
        for r in 0..nrows {
            for c in self.columns.iter_mut() {
                match c {
                    AlphaNumColumnReader::Float(f) => f.h_read(h, r)?,
                    AlphaNumColumnReader::Uint(u) => u.h_read(h, r)?,
                    AlphaNumColumnReader::Ascii(d) => {
                        buf.clear();
                        h.take(u8::from(d.width).into()).read_to_end(&mut buf)?;
                        d.column[r] = ascii_to_uint(&buf).map_err(ImpureError::Pure)?;
                    }
                }
            }
        }
        let cs: Vec<_> = self
            .columns
            .into_iter()
            .map(|c| c.into_fcs_column())
            .collect();
        Ok(FCSDataFrame::try_new(cs).unwrap())
    }

    // fn check_tot(
    //     &self,
    //     tot: Tot,
    //     allow_mismatch: bool,
    // ) -> Tentative<(), TotEventMismatch, TotEventMismatch> {
    //     let total_events = self.columns.head.len();
    //     if tot.0 != total_events {
    //         let i = TotEventMismatch { tot, total_events };
    //         Tentative::new_either((), vec![i], !allow_mismatch)
    //     } else {
    //         Tentative::new1(())
    //     }
    // }
}

fn check_tot(
    total_events: usize,
    tot: Tot,
    allow_mismatch: bool,
) -> BiTentative<(), TotEventMismatch> {
    if tot.0 != total_events {
        let i = TotEventMismatch { tot, total_events };
        Tentative::new_either((), vec![i], !allow_mismatch)
    } else {
        Tentative::new1(())
    }
}

impl<C: Serialize, L: Serialize, T> Serialize for FixedLayout<C, L, T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("FixedLayout", 2)?;
        state.serialize_field("columns", Vec::from(self.columns.as_ref()).as_slice())?;
        state.serialize_field("byte_layout", &self.byte_layout)?;
        state.end()
    }
}

impl<T> Serialize for DelimAsciiLayout<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("DelimitedLayout", 1)?;
        state.serialize_field("ranges", Vec::from(self.ranges.as_ref()).as_slice())?;
        state.end()
    }
}

impl<T> FixedAsciiLayout<T> {
    fn ascii_layout_values<D: Copy, S: Default>(&self, datatype: D) -> LayoutValues<S, D> {
        LayoutValues {
            datatype: AlphaNumType::Ascii,
            // NOTE BYTEORD is meaningless for ASCII so use dummy
            byte_layout: S::default(),
            columns: self.column_layout_values(datatype).into(),
        }
    }

    fn column_layout_values<D: Copy>(&self, datatype: D) -> NonEmpty<ColumnLayoutValues<D>> {
        self.columns.as_ref().map(|c| ColumnLayoutValues {
            width: Width::Fixed(c.fixed_width()),
            range: Range(c.range().into()),
            datatype,
        })
    }
}

impl EndianLayout<NullAnyUintType> {
    pub(crate) fn endian_uint_try_new<D>(
        cs: NonEmpty<ColumnLayoutValues<D>>,
        e: Endian,
        notrunc: bool,
    ) -> DeferredResult<Self, ColumnError<BitmaskError>, ColumnError<NewUintTypeError>> {
        FixedLayout::try_new(cs, e, |c| {
            AnyUintType::try_new(c, notrunc).def_errors_into()
        })
    }

    pub(crate) fn uint_try_into_ordered<T>(self) -> LayoutConvertResult<AnyOrderedUintLayout<T>> {
        let cs = self.columns;
        cs.head
            .try_into_one_size(cs.tail, self.byte_layout, 1)
            .mult_map_errors(|(index, error)| ConvertWidthError { index, error })
            .mult_errors_into()
    }
}

impl<'a> EndianLayout<NullMixedType> {
    fn mixed_layout_values(&self) -> LayoutValues<Endian, Option<NumType>> {
        let cs: NonEmpty<_> = self.columns.as_ref().map(|c| ColumnLayoutValues {
            width: Width::Fixed(c.fixed_width()),
            range: c.range(),
            datatype: c.as_num_type(),
        });
        // If any numeric types are none, then that means at least one column is
        // ASCII, which means that $DATATYPE needs to be "A" since $PnDATATYPE
        // cannot be "A".
        let (datatype, columns) = if let Ok(mut ds) = cs.as_ref().try_map(|c| c.datatype.ok_or(()))
        {
            // Determine which type appears the most, use that for $DATATYPE
            ds.sort();
            // TODO this should be a general non-empty function
            let mut counts = NonEmpty::new((ds.head, 1));
            for d in ds.tail {
                if counts.last().0 == d {
                    counts.last_mut().1 += 1;
                } else {
                    counts.push((d, 1));
                }
            }
            let mode = counts.maximum_by_key(|x| x.1).0;
            // Set all columns which have same type as $DATATYPE to None
            let new_cs = cs.map(|c| {
                if c.datatype.is_some_and(|x| x == mode) {
                    c.datatype == None;
                }
                c
            });
            (mode.into(), new_cs)
        } else {
            (AlphaNumType::Ascii, cs)
        };
        LayoutValues3_2 {
            datatype,
            byte_layout: self.byte_layout,
            columns: columns.into(),
        }
    }

    pub(crate) fn try_into_ordered<T>(
        self,
    ) -> MultiResult<AnyOrderedLayout<T>, MixedToOrderedLayoutError> {
        let c = self.columns.head;
        let cs = self.columns.tail;
        let endian = self.byte_layout;
        match c {
            MixedType::Ascii(x) => cs
                .into_iter()
                .enumerate()
                .map(|(i, c)| {
                    c.try_into().map_err(|e| MixedColumnConvertError {
                        error: MixedToOrderedConvertError::Ascii(e),
                        index: (i + 1).into(),
                    })
                })
                .gather()
                .map(|xs| {
                    AnyOrderedLayout::Ascii(AnyAsciiLayout::Fixed(FixedLayout::new1(x, xs, ())))
                }),
            MixedType::Uint(x) => x
                .try_into_one_size(cs, endian, 1)
                .map(AnyOrderedLayout::Integer)
                .mult_map_errors(|(index, error)| MixedColumnConvertError {
                    index,
                    error: error.into(),
                }),
            MixedType::F32(x) => cs
                .into_iter()
                .enumerate()
                .map(|(i, c)| {
                    c.try_into().map_err(|e| MixedColumnConvertError {
                        error: MixedToOrderedConvertError::Float(e),
                        index: (i + 1).into(),
                    })
                })
                .gather()
                .map(|xs| AnyOrderedLayout::F32(FixedLayout::new1(x, xs, endian.into()))),
            MixedType::F64(x) => cs
                .into_iter()
                .enumerate()
                .map(|(i, c)| {
                    c.try_into().map_err(|e| MixedColumnConvertError {
                        error: MixedToOrderedConvertError::Double(e),
                        index: (i + 1).into(),
                    })
                })
                .gather()
                .map(|xs| AnyOrderedLayout::F64(FixedLayout::new1(x, xs, endian.into()))),
        }
    }

    pub(crate) fn try_into_non_mixed(
        self,
    ) -> MultiResult<NonMixedEndianLayout, MixedToNonMixedLayoutError> {
        let c = self.columns.head;
        let it = self.columns.tail.into_iter().enumerate();
        let byte_layout = self.byte_layout;
        match c {
            MixedType::Ascii(x) => it
                .map(|(i, c)| {
                    c.try_into()
                        .map_err(|e| (i, MixedToNonMixedConvertError::Ascii(e)))
                })
                .gather()
                .map(|xs| {
                    NonMixedEndianLayout::Ascii(AnyAsciiLayout::Fixed(FixedLayout::new1(x, xs, ())))
                }),
            MixedType::Uint(x) => it
                .map(|(i, c)| {
                    c.try_into()
                        .map_err(|e| (i, MixedToNonMixedConvertError::Integer(e)))
                })
                .gather()
                .map(|xs| NonMixedEndianLayout::Integer(FixedLayout::new1(x, xs, byte_layout))),
            MixedType::F32(x) => it
                .map(|(i, c)| {
                    c.try_into()
                        .map_err(|e| (i, MixedToNonMixedConvertError::Float(e)))
                })
                .gather()
                .map(|xs| NonMixedEndianLayout::F32(FixedLayout::new1(x, xs, byte_layout))),
            MixedType::F64(x) => it
                .map(|(i, c)| {
                    c.try_into()
                        .map_err(|e| (i, MixedToNonMixedConvertError::Double(e)))
                })
                .gather()
                .map(|xs| NonMixedEndianLayout::F64(FixedLayout::new1(x, xs, byte_layout))),
        }
        .mult_map_errors(|(i, error)| MixedColumnConvertError {
            index: (i + 1).into(),
            error,
        })
    }
}

pub type MixedToOrderedLayoutError = MixedColumnConvertError<MixedToOrderedConvertError>;
pub type MixedToNonMixedLayoutError = MixedColumnConvertError<MixedToNonMixedConvertError>;

pub struct MixedColumnConvertError<E> {
    index: MeasIndex,
    error: E,
}

enum_from_disp!(
    pub MixedToOrderedConvertError,
    [Ascii, MixedToAsciiError],
    [Integer, MixedToOrderedUintError],
    [Float, MixedToFloatError],
    [Double, MixedToDoubleError]
);

enum_from_disp!(
    pub MixedToNonMixedConvertError,
    [Ascii, MixedToAsciiError],
    [Integer, MixedToEndianUintError],
    [Float, MixedToFloatError],
    [Double, MixedToDoubleError]
);

impl<E: fmt::Display> fmt::Display for MixedColumnConvertError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "mixed conversion error in column {}: {}",
            self.index, self.error
        )
    }
}

trait IntMath: Sized {
    fn next_bitmask(x: Self) -> Self;
}

// TODO clean this up with https://github.com/rust-lang/rust/issues/76560 once
// it lands in a stable compiler, in theory there is no reason to put the length
// of the type as a parameter, but the current compiler is not smart enough
trait NumProps: Sized + Copy + Default {
    const LEN: usize;
    type BUF: AsRef<[u8]> + AsMut<[u8]> + Default;

    fn read_buf<R: Read>(h: &mut BufReader<R>) -> io::Result<Self::BUF>;

    fn from_big(buf: Self::BUF) -> Self;

    fn from_little(buf: Self::BUF) -> Self;

    fn to_big(self) -> Self::BUF;

    fn to_little(self) -> Self::BUF;

    fn maxval() -> Self;
}

trait OrderedFromBytes<const OLEN: usize>: NumProps {
    fn h_read_from_ordered<R: Read>(h: &mut BufReader<R>, order: [u8; OLEN]) -> io::Result<Self> {
        let mut tmp = [0; OLEN];
        let mut buf = Self::BUF::default();
        h.read_exact(&mut tmp)?;
        for (i, j) in order.iter().enumerate() {
            buf.as_mut()[usize::from(*j)] = tmp[i];
        }
        Ok(Self::from_little(buf))
    }

    fn h_write_from_ordered<W: Write>(
        self,
        h: &mut BufWriter<W>,
        order: [u8; OLEN],
    ) -> io::Result<()> {
        let tmp = Self::to_little(self);
        let mut buf = [0; OLEN];
        for (i, j) in order.iter().enumerate() {
            buf[usize::from(*j)] = tmp.as_ref()[i];
        }
        h.write_all(tmp.as_ref())
    }
}

trait IntFromBytes<const INTLEN: usize>
where
    Self: OrderedFromBytes<INTLEN>,
    Self: TryFrom<FloatOrInt, Error = ToIntError<Self>>,
    Self: IntMath,
{
    fn range_to_bitmask(r: Range, notrunc: bool) -> Tentative<Self, BitmaskError, BitmaskError> {
        let go = |x, e| {
            let y = Self::next_bitmask(x);
            if notrunc {
                Tentative::new(y, vec![], vec![e])
            } else {
                Tentative::new(y, vec![e], vec![])
            }
        };
        r.0.try_into().map_or_else(
            |e| match e {
                ToIntError::IntOverrange(x) => go(Self::maxval(), BitmaskError::IntOverrange(x)),
                ToIntError::FloatOverrange(x) => {
                    go(Self::maxval(), BitmaskError::FloatOverrange(x))
                }
                ToIntError::FloatUnderrange(x) => {
                    go(Self::default(), BitmaskError::FloatUnderrange(x))
                }
                ToIntError::FloatPrecisionLoss(x, y) => go(y, BitmaskError::FloatPrecisionLoss(x)),
            },
            |x| Tentative::new1(Self::next_bitmask(x)),
        )
    }

    fn column_type(
        r: Range,
        notrunc: bool,
    ) -> Tentative<UintType<Self, INTLEN>, BitmaskError, BitmaskError> {
        Self::range_to_bitmask(r, notrunc).map(|bitmask| UintType { bitmask })
    }

    fn h_read_endian<R: Read>(h: &mut BufReader<R>, endian: Endian) -> io::Result<Self> {
        // This will read data that is not a power-of-two bytes long. Start by
        // reading n bytes into a vector, which can take a varying size. Then
        // copy this into the power of 2 buffer and reset all the unused cells
        // to 0. This copy has to go to one or the other end of the buffer
        // depending on endianness.
        //
        // ASSUME for u8 and u16 that these will get heavily optimized away
        // since 'order' is totally meaningless for u8 and the only two possible
        // 'orders' for u16 are big and little.
        let mut tmp = [0; INTLEN];
        let mut buf = Self::BUF::default();
        h.read_exact(&mut tmp)?;
        Ok(if endian == Endian::Big {
            let b = Self::LEN - INTLEN;
            buf.as_mut()[b..].copy_from_slice(&tmp[b..]);
            Self::from_big(buf)
        } else {
            buf.as_mut()[..INTLEN].copy_from_slice(&tmp[..INTLEN]);
            Self::from_little(buf)
        })
    }

    fn h_read_ordered<R: Read>(
        h: &mut BufReader<R>,
        byteord: SizedByteOrd<INTLEN>,
    ) -> io::Result<Self> {
        match byteord {
            SizedByteOrd::Endian(e) => Self::h_read_endian(h, e),
            SizedByteOrd::Order(order) => Self::h_read_from_ordered(h, order),
        }
    }

    fn h_write_endian<W: Write>(self, h: &mut BufWriter<W>, endian: Endian) -> io::Result<()> {
        let mut buf = [0; INTLEN];
        let (start, end, tmp) = if endian == Endian::Big {
            ((Self::LEN - INTLEN), Self::LEN, Self::to_big(self))
        } else {
            (0, INTLEN, Self::to_little(self))
        };
        buf[..].copy_from_slice(&tmp.as_ref()[start..end]);
        h.write_all(&buf)
    }

    fn h_write_ordered<W: Write>(
        self,
        h: &mut BufWriter<W>,
        byteord: SizedByteOrd<INTLEN>,
    ) -> io::Result<()> {
        match byteord {
            SizedByteOrd::Endian(e) => self.h_write_endian(h, e),
            SizedByteOrd::Order(o) => self.h_write_from_ordered(h, o),
        }
    }
}

trait FloatFromBytes<const LEN: usize>
where
    Self: NumProps,
    Self: OrderedFromBytes<LEN>,
    Self: FromStr,
    Self: TryFrom<FloatOrInt, Error = ToFloatError<Self>>,
    Self: Clone,
{
    fn range(r: Range) -> Self {
        // TODO control how this works and/or warn user if we truncate
        r.0.try_into().unwrap_or_else(|e| match e {
            ToFloatError::IntPrecisionLoss(_, x) => x,
            ToFloatError::FloatOverrange(_) => Self::maxval(),
            ToFloatError::FloatUnderrange(_) => Self::default(),
        })
    }

    fn column_type(w: Width, r: Range) -> Result<FloatType<Self, LEN>, FloatWidthError> {
        Bytes::try_from(w).map_err(|e| e.into()).and_then(|bytes| {
            if usize::from(u8::from(bytes)) == LEN {
                let range = Self::range(r);
                Ok(FloatType { range })
            } else {
                Err(FloatWidthError::WrongWidth(WrongFloatWidth {
                    expected: LEN,
                    width: bytes,
                }))
            }
        })
    }

    fn h_read_endian<R: Read>(h: &mut BufReader<R>, endian: Endian) -> io::Result<Self> {
        let buf = Self::read_buf(h)?;
        Ok(if endian == Endian::Big {
            Self::from_big(buf)
        } else {
            Self::from_little(buf)
        })
    }

    fn h_read_ordered<R: Read>(
        h: &mut BufReader<R>,
        byteord: SizedByteOrd<LEN>,
    ) -> io::Result<Self> {
        match byteord {
            SizedByteOrd::Endian(endian) => Self::h_read_endian(h, endian),
            SizedByteOrd::Order(order) => Self::h_read_from_ordered(h, order),
        }
    }

    fn h_write_endian<W: Write>(self, h: &mut BufWriter<W>, endian: Endian) -> io::Result<()> {
        let buf = if endian == Endian::Big {
            Self::to_big(self)
        } else {
            Self::to_little(self)
        };
        h.write_all(buf.as_ref())
    }

    fn h_write_ordered<W: Write>(
        self,
        h: &mut BufWriter<W>,
        byteord: SizedByteOrd<LEN>,
    ) -> io::Result<()> {
        match byteord {
            SizedByteOrd::Endian(endian) => self.h_write_endian(h, endian),
            SizedByteOrd::Order(order) => self.h_write_from_ordered(h, order),
        }
    }
}

macro_rules! impl_num_props {
    ($size:expr, $t:ty) => {
        impl NumProps for $t {
            const LEN: usize = $size;
            type BUF = [u8; $size];

            fn read_buf<R: Read>(h: &mut BufReader<R>) -> io::Result<[u8; $size]> {
                let mut buf = [0; $size];
                h.read_exact(&mut buf)?;
                Ok(buf)
            }

            fn to_big(self) -> [u8; $size] {
                <$t>::to_be_bytes(self)
            }

            fn to_little(self) -> [u8; $size] {
                <$t>::to_le_bytes(self)
            }

            fn from_big(buf: [u8; $size]) -> Self {
                <$t>::from_be_bytes(buf)
            }

            fn from_little(buf: [u8; $size]) -> Self {
                <$t>::from_le_bytes(buf)
            }

            fn maxval() -> Self {
                Self::MAX
            }
        }
    };
}

impl_num_props!(1, u8);
impl_num_props!(2, u16);
impl_num_props!(4, u32);
impl_num_props!(8, u64);
impl_num_props!(4, f32);
impl_num_props!(8, f64);

macro_rules! impl_int_math {
    ($t:ty) => {
        impl IntMath for $t {
            fn next_bitmask(x: Self) -> Self {
                Self::checked_next_power_of_two(x)
                    .map(|x| x - 1)
                    .unwrap_or(Self::MAX)
            }
        }
    };
}

impl_int_math!(u8);
impl_int_math!(u16);
impl_int_math!(u32);
impl_int_math!(u64);

impl OrderedFromBytes<1> for u8 {}
impl OrderedFromBytes<2> for u16 {}
impl OrderedFromBytes<3> for u32 {}
impl OrderedFromBytes<4> for u32 {}
impl OrderedFromBytes<5> for u64 {}
impl OrderedFromBytes<6> for u64 {}
impl OrderedFromBytes<7> for u64 {}
impl OrderedFromBytes<8> for u64 {}
impl OrderedFromBytes<4> for f32 {}
impl OrderedFromBytes<8> for f64 {}

impl FloatFromBytes<4> for f32 {}
impl FloatFromBytes<8> for f64 {}

impl IntFromBytes<1> for u8 {}
impl IntFromBytes<2> for u16 {}
impl IntFromBytes<3> for u32 {}
impl IntFromBytes<4> for u32 {}
impl IntFromBytes<5> for u64 {}
impl IntFromBytes<6> for u64 {}
impl IntFromBytes<7> for u64 {}
impl IntFromBytes<8> for u64 {}

impl AlphaNumColumnReader {
    fn into_fcs_column(self) -> AnyFCSColumn {
        match self {
            Self::Ascii(x) => U64Column::from(x.column).into(),
            Self::Float(x) => x.into_fcs_column(),
            Self::Uint(x) => x.into_fcs_column(),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Ascii(x) => x.column.len(),
            Self::Float(x) => x.len(),
            Self::Uint(x) => x.len(),
        }
    }
}

impl AnyUintColumnReader {
    fn into_fcs_column(self) -> AnyFCSColumn {
        match self {
            AnyUintColumnReader::Uint08(x) => U08Column::from(x.column).into(),
            AnyUintColumnReader::Uint16(x) => U16Column::from(x.column).into(),
            AnyUintColumnReader::Uint24(x) => U32Column::from(x.column).into(),
            AnyUintColumnReader::Uint32(x) => U32Column::from(x.column).into(),
            AnyUintColumnReader::Uint40(x) => U64Column::from(x.column).into(),
            AnyUintColumnReader::Uint48(x) => U64Column::from(x.column).into(),
            AnyUintColumnReader::Uint56(x) => U64Column::from(x.column).into(),
            AnyUintColumnReader::Uint64(x) => U64Column::from(x.column).into(),
        }
    }

    fn len(&self) -> usize {
        match_many_to_one!(
            self,
            Self,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            x,
            { x.column.len() }
        )
    }
}

impl AnyUintColumnReader {
    fn h_read<R: Read>(&mut self, h: &mut BufReader<R>, r: usize) -> io::Result<()> {
        match_many_to_one!(
            self,
            AnyUintColumnReader,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            d,
            { d.h_read(h, r)? }
        );
        Ok(())
    }
}

// TODO also check scale here?
impl NullMixedType {
    pub(crate) fn try_new(
        c: ColumnLayoutValues<Option<NumType>>,
        conf: &SharedConfig,
    ) -> DeferredResult<Self, BitmaskError, NewMixedTypeError> {
        let w = c.width;
        let r = c.range;
        if let Some(dt) = c.datatype {
            match dt {
                NumType::Integer => AnyUintType::try_new(c, conf.disallow_bitmask_truncation)
                    .def_map_value(Self::Uint)
                    .def_errors_into(),
                NumType::Single => f32::column_type(w, r).map(Self::F32).into_deferred(),
                NumType::Double => f64::column_type(w, r).map(Self::F64).into_deferred(),
            }
        } else {
            AsciiType::try_new(w, r)
                .mult_to_deferred()
                .def_map_value(Self::Ascii)
        }
    }

    fn as_num_type(&self) -> Option<NumType> {
        match self {
            Self::Ascii(_) => None,
            Self::Uint(_) => Some(NumType::Integer),
            Self::F32(_) => Some(NumType::Single),
            Self::F64(_) => Some(NumType::Double),
        }
    }

    fn as_reader(&self, nrows: usize) -> ReaderMixedType {
        match self {
            Self::Ascii(c) => MixedType::Ascii(c.into_reader(nrows)),
            Self::Uint(c) => MixedType::Uint(c.as_reader(nrows)),
            Self::F32(c) => MixedType::F32(c.into_reader(nrows)),
            Self::F64(c) => MixedType::F64(c.into_reader(nrows)),
        }
    }

    // fn as_writer<'a>(&self, xs: &'a AnyFCSColumn) -> Result<WriterMixedType<'a>, AnyLossError> {
    //     match self {
    //         Self::Ascii(c) => c
    //             .into_writer(xs, false)
    //             .map(MixedType::Ascii)
    //             .map_err(|e| e.into()),
    //         Self::Uint(c) => c.as_writer(xs).map(MixedType::Uint).map_err(|e| e.into()),
    //         Self::F32(c) => c
    //             .into_writer(xs, false)
    //             .map(MixedType::F32)
    //             .map_err(|e| e.into()),
    //         Self::F64(c) => c
    //             .into_writer(xs, false)
    //             .map(MixedType::F64)
    //             .map_err(|e| e.into()),
    //     }
    // }
}

fn ascii_to_uint(buf: &[u8]) -> Result<u64, AsciiToUintError> {
    if buf.is_ascii() {
        let s = unsafe { str::from_utf8_unchecked(buf) };
        s.parse().map_err(AsciiToUintError::from)
    } else {
        Err(NotAsciiError(buf.to_vec()).into())
    }
}

/// A struct whose fields map 1-1 with keyword values pertaining to data layout.
struct LayoutValues<S, D> {
    datatype: AlphaNumType,
    byte_layout: S,
    columns: Vec<ColumnLayoutValues<D>>,
}

type OrderedLayoutValues = LayoutValues<ByteOrd, ()>;
type LayoutValues3_1 = LayoutValues<Endian, ()>;
type LayoutValues3_2 = LayoutValues<Endian, Option<NumType>>;

/// A struct whose fields map 1-1 with keyword values in one data column
struct ColumnLayoutValues<D> {
    width: Width,
    range: Range,
    datatype: D,
}

impl<S, D> LayoutValues<S, D> {
    fn req_keywords(&self) -> impl Iterator<Item = (String, String)>
    where
        S: ReqMetarootKey,
    {
        [self.datatype.pair(), self.byte_layout.pair()].into_iter()
    }

    fn req_meas_keywords(&self) -> impl Iterator<Item = (String, String, String)>
    where
        ColumnLayoutValues<D>: VersionedColumnLayout,
    {
        self.columns
            .iter()
            .enumerate()
            .map(|(i, c)| c.req_keywords(i.into()))
            .flatten()
    }

    fn opt_meas_keywords(&self) -> impl Iterator<Item = (String, String, Option<String>)>
    where
        ColumnLayoutValues<D>: VersionedColumnLayout,
    {
        self.columns
            .iter()
            .enumerate()
            .map(|(i, c)| c.opt_keywords(i.into()))
            .flatten()
    }
}

impl<S: Default, D> Default for LayoutValues<S, D> {
    fn default() -> Self {
        Self {
            datatype: AlphaNumType::Integer,
            byte_layout: S::default(),
            columns: vec![],
        }
    }
}

type ColumnLayoutValues2_0 = ColumnLayoutValues<()>;
type ColumnLayoutValues3_2 = ColumnLayoutValues<Option<NumType>>;

trait VersionedColumnLayout: Sized {
    fn lookup_all(kws: &mut StdKeywords, par: Par) -> LookupResult<Vec<Self>> {
        (0..par.0)
            .map(|i| Self::lookup(kws, i.into()))
            .gather()
            .map(Tentative::mconcat)
            .map_err(DeferredFailure::mconcat)
    }

    fn get_all(
        kws: &StdKeywords,
    ) -> DeferredResult<Vec<Self>, ParseKeyError<NumTypeError>, RawParsedError> {
        Par::get_metaroot_req(kws)
            .into_deferred()
            .def_and_maybe(|par| {
                (0..par.0)
                    .map(|i| Self::get(kws, i.into()))
                    .gather()
                    .map(Tentative::mconcat)
                    .map_err(DeferredFailure::mconcat)
            })
    }

    fn lookup(kws: &mut StdKeywords, i: MeasIndex) -> LookupResult<Self>;

    fn get(
        kws: &StdKeywords,
        i: MeasIndex,
    ) -> DeferredResult<Self, ParseKeyError<NumTypeError>, RawParsedError>;

    fn req_keywords(&self, i: MeasIndex) -> impl Iterator<Item = (String, String, String)>;

    fn opt_keywords(&self, i: MeasIndex) -> impl Iterator<Item = (String, String, Option<String>)>;
}

impl VersionedColumnLayout for ColumnLayoutValues2_0 {
    fn lookup(kws: &mut StdKeywords, i: MeasIndex) -> LookupResult<Self> {
        let j = i.into();
        let w = Width::lookup_req(kws, j);
        let r = Range::lookup_req(kws, j);
        w.def_zip(r).def_map_value(|(width, range)| Self {
            width,
            range,
            datatype: (),
        })
    }

    fn get(
        kws: &StdKeywords,
        i: MeasIndex,
    ) -> DeferredResult<Self, ParseKeyError<NumTypeError>, RawParsedError> {
        let j = i.into();
        let w = Width::get_meas_req(kws, j).map_err(|e| e.into());
        let r = Range::get_meas_req(kws, j).map_err(|e| e.into());
        w.zip(r)
            .map(|(width, range)| Self {
                width,
                range,
                datatype: (),
            })
            .map(Tentative::new1)
            .map_err(DeferredFailure::new2)
    }

    fn req_keywords(&self, i: MeasIndex) -> impl Iterator<Item = (String, String, String)> {
        let j = i.into();
        [self.range.triple(j), self.width.triple(j)].into_iter()
    }

    fn opt_keywords(&self, _: MeasIndex) -> impl Iterator<Item = (String, String, Option<String>)> {
        [].into_iter()
    }
}

impl VersionedColumnLayout for ColumnLayoutValues3_2 {
    fn lookup(kws: &mut StdKeywords, i: MeasIndex) -> LookupResult<Self> {
        let j = i.into();
        let w = Width::lookup_req(kws, j);
        let r = Range::lookup_req(kws, j);
        w.def_zip(r).def_and_tentatively(|(width, range)| {
            NumType::lookup_opt(kws, j, false)
                .map(|x| x.0)
                .map(|datatype| Self {
                    width,
                    range,
                    datatype,
                })
        })
    }

    fn get(
        kws: &StdKeywords,
        i: MeasIndex,
    ) -> DeferredResult<Self, ParseKeyError<NumTypeError>, RawParsedError> {
        let j = i.into();
        let w = Width::get_meas_req(kws, j).map_err(|e| e.into());
        let r = Range::get_meas_req(kws, j).map_err(|e| e.into());
        w.zip(r)
            .map(Tentative::new1)
            .map_err(DeferredFailure::new2)
            .def_and_tentatively(|(width, range)| {
                NumType::get_meas_opt(kws, j)
                    .map_err(|e| e.into())
                    .map(|x| x.0)
                    .map_or_else(|w| Tentative::new(None, vec![w], vec![]), Tentative::new1)
                    .map(|datatype| Self {
                        width,
                        range,
                        datatype,
                    })
            })
    }

    fn req_keywords(&self, i: MeasIndex) -> impl Iterator<Item = (String, String, String)> {
        let j = i.into();
        [self.range.triple(j), self.width.triple(j)].into_iter()
    }

    fn opt_keywords(&self, i: MeasIndex) -> impl Iterator<Item = (String, String, Option<String>)> {
        [(
            NumType::std(i.into()).to_string(),
            NumType::std_blank(),
            self.datatype.map(|x| x.to_string()),
        )]
        .into_iter()
    }
}

impl From<ColumnLayoutValues3_2> for ColumnLayoutValues2_0 {
    fn from(value: ColumnLayoutValues3_2) -> Self {
        Self {
            width: value.width,
            range: value.range,
            datatype: (),
        }
    }
}

impl<T> DelimAsciiLayout<T> {
    fn new(ranges: NonEmpty<u64>) -> Self {
        Self {
            ranges,
            tot_action: PhantomData,
        }
    }

    fn layout_values<D: Copy, S: Default>(&self, datatype: D) -> LayoutValues<S, D> {
        LayoutValues {
            datatype: AlphaNumType::Ascii,
            // NOTE BYTEORD is meaningless for delimited ASCII so use a dummy
            byte_layout: S::default(),
            columns: self.column_layout_values(datatype).into(),
        }
    }

    fn column_layout_values<D: Copy>(&self, datatype: D) -> NonEmpty<ColumnLayoutValues<D>> {
        self.ranges.as_ref().map(|r| ColumnLayoutValues {
            width: Width::Variable,
            range: Range((*r).into()),
            datatype,
        })
    }

    fn h_read_df<R: Read>(
        &self,
        h: &mut BufReader<R>,
        tot: T::Tot,
        nbytes: usize,
    ) -> IOResult<FCSDataFrame, ReadDelimAsciiError>
    where
        T: TotDefinition,
    {
        let rs = &self.ranges;
        T::with_tot(
            h,
            tot,
            |_h, t| h_read_delim_with_rows(rs, _h, t, nbytes).map_err(|e| e.inner_into()),
            |_h| h_read_delim_without_rows(rs, _h, nbytes).map_err(|e| e.inner_into()),
        )
    }

    fn check_writer<'a>(&self, df: &'a FCSDataFrame) -> MultiResult<(), AnyLossError> {
        df.iter_columns()
            .map(|c| c.check_writer::<_, _, u64>(|_| None))
            .gather()
            .mult_map_errors(AnyLossError::Int)
            .void()
    }

    fn h_write_df<'a, W: Write>(
        &self,
        h: &mut BufWriter<W>,
        df: &'a FCSDataFrame,
    ) -> io::Result<()> {
        let ncols = df.ncols();
        let nrows = df.nrows();
        // ASSUME dataframe has correct number of columns
        let mut column_srcs: Vec<_> = df.iter_columns().map(AnySource::new::<u64>).collect();
        for row in 0..nrows {
            for (col, xs) in column_srcs.iter_mut().enumerate() {
                let x = xs.next().unwrap();
                let s = x.new.to_string();
                let buf = s.as_bytes();
                h.write_all(buf)?;
                // write delimiter after all but last value
                if !(row == nrows - 1 && col == ncols - 1) {
                    h.write_all(&[32])?; // 32 = space in ASCII
                }
            }
        }
        Ok(())
    }
}

fn h_read_delim_with_rows<R: Read>(
    ranges: &NonEmpty<u64>,
    h: &mut BufReader<R>,
    tot: Tot,
    nbytes: usize,
) -> IOResult<FCSDataFrame, ReadDelimWithRowsAsciiError> {
    let mut buf = Vec::new();
    let mut last_was_delim = false;
    let nrows = tot.0;
    let ncols = ranges.len();
    // Here we have $TOT so initialize vectors to required length
    let mut data = ranges.as_ref().map(|_| vec![0; nrows]);
    // let mut data = self.0.columns;
    // let nrows = data.head.len();
    // let ncols = data.len();
    let mut row = 0;
    let mut col = 0;
    // Delimiters are tab, newline, carriage return, space, or comma. Any
    // consecutive delimiter counts as one, and delimiters can be mixed.
    for b in h.bytes().take(nbytes) {
        let byte = b?;
        // exit if we encounter more rows than expected.
        if row == nrows {
            let e = ReadDelimWithRowsAsciiError::RowsExceeded(RowsExceededError(nrows));
            return Err(ImpureError::Pure(e));
        }
        if is_ascii_delim(byte) {
            if !last_was_delim {
                last_was_delim = true;
                data[col][row] = ascii_to_uint(&buf)
                    .map_err(ReadDelimWithRowsAsciiError::Parse)
                    .map_err(ImpureError::Pure)?;
                buf.clear();
                if col == ncols - 1 {
                    col = 0;
                    row += 1;
                } else {
                    col += 1;
                }
            }
        } else {
            buf.push(byte);
            last_was_delim = false;
        }
    }
    if !(col == 0 && row == nrows) {
        let e = DelimIncompleteError { col, row, nrows };
        return Err(ImpureError::Pure(ReadDelimWithRowsAsciiError::Incomplete(
            e,
        )));
    }
    // The spec isn't clear if the last value should be a delim or
    // not, so flush the buffer if it has anything in it since we
    // only try to parse if we hit a delim above.
    if !buf.is_empty() {
        data[col][row] = ascii_to_uint(&buf)
            .map_err(ReadDelimWithRowsAsciiError::Parse)
            .map_err(ImpureError::Pure)?;
    }
    let cs: Vec<_> = data
        .into_iter()
        .map(FCSColumn::from)
        .map(AnyFCSColumn::from)
        .collect();
    // ASSUME this will never fail because all columns should be the same
    // length
    Ok(FCSDataFrame::try_new(cs).unwrap())
}

fn h_read_delim_without_rows<R: Read>(
    ranges: &NonEmpty<u64>,
    h: &mut BufReader<R>,
    nbytes: usize,
) -> IOResult<FCSDataFrame, ReadDelimAsciiWithoutRowsError> {
    let mut buf = Vec::new();
    // Here we don't have $TOT so init to empty vectors
    let mut data = ranges.as_ref().map(|_| vec![]);
    let ncols = data.len();
    let mut col = 0;
    let mut last_was_delim = false;
    let go = |_data: &mut NonEmpty<Vec<u64>>, _col, _buf: &[u8]| {
        ascii_to_uint(_buf)
            .map_err(ReadDelimAsciiWithoutRowsError::Parse)
            .map_err(ImpureError::Pure)
            .map(|x| _data[_col].push(x))
    };
    // Delimiters are tab, newline, carriage return, space, or comma. Any
    // consecutive delimiter counts as one, and delimiters can be mixed.
    // If we don't know the number of rows, the only choice is to push onto
    // the column vectors one at a time. This leads to the possibility that
    // the vectors may not be the same length in the end, in which case,
    // scream loudly and bail.
    for b in h.bytes().take(nbytes) {
        let byte = b?;
        if is_ascii_delim(byte) {
            if !last_was_delim {
                last_was_delim = true;
                buf.clear();
                go(&mut data, col, &buf)?;
                if col == ncols - 1 {
                    col = 0;
                } else {
                    col += 1;
                }
            }
        } else {
            buf.push(byte);
            last_was_delim = false;
        }
    }
    if data.iter().map(|c| c.len()).unique().count() > 1 {
        return Err(ImpureError::Pure(ReadDelimAsciiWithoutRowsError::Unequal));
    }
    // The spec isn't clear if the last value should be a delim or
    // not, so flush the buffer if it has anything in it since we
    // only try to parse if we hit a delim above.
    if !buf.is_empty() {
        go(&mut data, col, &buf)?;
    }
    let cs: Vec<_> = data
        .into_iter()
        .map(FCSColumn::from)
        .map(AnyFCSColumn::from)
        .collect();
    // ASSUME this will never fail because all columns should be the same
    // length
    Ok(FCSDataFrame::try_new(cs).unwrap())
}

impl<C, S, T> FixedLayout<C, S, T> {
    fn new(columns: NonEmpty<C>, byte_layout: S) -> Self {
        Self {
            columns,
            byte_layout,
            tot_action: PhantomData,
        }
    }

    fn new1(head: C, tail: Vec<C>, byte_layout: S) -> Self {
        Self::new((head, tail).into(), byte_layout)
    }

    fn try_new<D, F, W, E, CW, CE>(
        cs: NonEmpty<ColumnLayoutValues<D>>,
        byte_layout: S,
        new_col_f: F,
    ) -> DeferredResult<Self, W, E>
    where
        W: From<ColumnError<CW>>,
        E: From<ColumnError<CE>>,
        F: Fn(ColumnLayoutValues<D>) -> DeferredResult<C, CW, CE>,
    {
        ne_map_results(ne_enumerate(cs), |(i, c)| {
            new_col_f(c)
                .def_map_errors(|error| {
                    ColumnError {
                        error,
                        index: i.into(),
                    }
                    .into()
                })
                .def_map_warnings(|error| {
                    ColumnError {
                        error,
                        index: i.into(),
                    }
                    .into()
                })
        })
        .map_err(DeferredFailure::mconcat)
        .map(Tentative::mconcat_ne)
        .def_map_value(|columns| Self::new(columns, byte_layout))
    }

    fn layout_values<D: Copy, R>(&self, datatype: D) -> LayoutValues<R, D>
    where
        R: From<S>,
        S: Copy,
        C: IsFixed + HasDatatype,
    {
        LayoutValues {
            datatype: C::DATATYPE,
            byte_layout: self.byte_layout.into(),
            columns: self
                .columns
                .as_ref()
                .map(|c| ColumnLayoutValues {
                    width: Width::Fixed(c.fixed_width()),
                    range: c.range(),
                    datatype,
                })
                .into(),
        }
    }

    fn columns_into<X>(self) -> FixedLayout<X, S, T>
    where
        X: From<C>,
    {
        FixedLayout::new(self.columns.map(|c| c.into()), self.byte_layout)
    }

    fn byte_layout_into<X>(self) -> FixedLayout<C, X, T>
    where
        X: From<S>,
    {
        FixedLayout::new(self.columns, self.byte_layout.into())
    }

    fn byte_layout_try_into<X>(self) -> Result<FixedLayout<C, X, T>, X::Error>
    where
        X: TryFrom<S>,
    {
        self.byte_layout
            .try_into()
            .map(|byte_layout| FixedLayout::new(self.columns, byte_layout))
    }

    fn tot_into<X>(self) -> FixedLayout<C, S, X> {
        FixedLayout::new(self.columns, self.byte_layout)
    }

    fn event_width(&self) -> usize
    where
        C: IsFixed,
    {
        self.columns.iter().map(|c| usize::from(c.nbytes())).sum()
    }

    fn ncols(&self) -> usize {
        self.columns.len()
    }

    // pub fn into_col_reader_inner(
    //     self,
    //     seg: AnyDataSegment,
    //     conf: &ReaderConfig,
    // ) -> Tentative<AlphaNumReader, UnevenEventWidth, UnevenEventWidth>
    // where
    //     S: Clone,
    //     C: IsFixedReader<S = S> + IsFixed,
    // {
    //     let n = seg.inner.len() as usize;
    //     let w = self.event_width();
    //     let total_events = n / w;
    //     let remainder = n % w;
    //     let columns = self
    //         .columns
    //         // TODO clone
    //         .map(|c| c.into_col_reader(total_events, self.byte_layout.clone()));
    //     let r = AlphaNumReader { columns };
    //     if remainder > 0 {
    //         let i = UnevenEventWidth {
    //             event_width: w,
    //             nbytes: n,
    //             remainder,
    //         };
    //         Tentative::new_either(r, vec![i], !conf.allow_uneven_event_width)
    //     } else {
    //         Tentative::new1(r)
    //     }
    // }

    // pub fn into_col_reader<W, E>(
    //     self,
    //     seg: AnyDataSegment,
    //     tot: Tot,
    //     conf: &ReaderConfig,
    // ) -> Tentative<ColumnReader, W, E>
    // where
    //     S: Copy,
    //     C: IsFixedReader<S = S> + IsFixed,
    //     W: From<TotEventMismatch> + From<UnevenEventWidth>,
    //     E: From<TotEventMismatch> + From<UnevenEventWidth>,
    // {
    //     self.into_col_reader_inner(seg, conf)
    //         .inner_into()
    //         .and_tentatively(|reader| {
    //             reader
    //                 .check_tot(tot, conf.allow_tot_mismatch)
    //                 .map(|_| reader)
    //                 .inner_into()
    //         })
    //         .map(ColumnReader::AlphaNum)
    // }

    // fn as_writer<'a>(&self, df: &'a FCSDataFrame, conf: &WriteConfig) -> io::Result<()>
    // where
    //     S: Copy,
    //     C: Copy + IsFixedWriter<S = S> + IsFixed,
    // {
    //     let check = conf.check_conversion;
    //     let nrows = df.nrows();
    //     // ASSUME df has same number of columns as layout
    //     let cs = self
    //         .columns
    //         .iter()
    //         .zip(df.iter_columns())
    //         .enumerate()
    //         .map(|(i, (t, c))| {
    //             t.into_col_writer(c, check, self.byte_layout)
    //                 .map_err(|error| {
    //                     ColumnWriterError(ColumnError {
    //                         index: i.into(),
    //                         error,
    //                     })
    //                 })
    //         })
    //         .collect();
    //     for _ in 0..nrows {
    //         for c in cs {
    //             c.h_write(h)?;
    //         }
    //     }
    //     Ok(())
    // }

    pub fn compute_nrows(
        &self,
        seg: AnyDataSegment,
        conf: &ReaderConfig,
    ) -> BiTentative<usize, UnevenEventWidth>
    where
        S: Clone,
        C: IsFixed,
    {
        let n = seg.inner.len() as usize;
        let w = self.event_width();
        let total_events = n / w;
        let remainder = n % w;
        if remainder > 0 {
            let i = UnevenEventWidth {
                event_width: w,
                nbytes: n,
                remainder,
            };
            Tentative::new_either(total_events, vec![i], !conf.allow_uneven_event_width)
        } else {
            Tentative::new1(total_events)
        }
    }

    fn h_read_df_numeric<R: Read, I, W, E>(
        &self,
        h: &mut BufReader<R>,
        tot: T::Tot,
        seg: AnyDataSegment,
        conf: &ReaderConfig,
    ) -> IODeferredResult<FCSDataFrame, W, E>
    where
        W: From<UnevenEventWidth> + From<TotEventMismatch>,
        E: From<UnevenEventWidth> + From<TotEventMismatch>,
        S: Copy,
        C: IsFixed + Copy,
        I: Readable<S, E, Inner = C, Buf = ()>,
        T: TotDefinition,
    {
        self.h_read_df::<_, I, _, _, E, E>(h, &mut (), tot, seg, conf)
    }

    fn h_read_df<R: Read, I, B, W, E, ReadErr>(
        &self,
        h: &mut BufReader<R>,
        buf: &mut B,
        tot: T::Tot,
        seg: AnyDataSegment,
        conf: &ReaderConfig,
    ) -> IODeferredResult<FCSDataFrame, W, E>
    where
        W: From<UnevenEventWidth> + From<TotEventMismatch>,
        E: From<ReadErr> + From<UnevenEventWidth> + From<TotEventMismatch>,
        S: Copy,
        C: IsFixed + Copy,
        I: Readable<S, ReadErr, Inner = C, Buf = B>,
        T: TotDefinition,
    {
        self.compute_nrows(seg, conf)
            .inner_into()
            .errors_liftio()
            .and_tentatively(|nrows| {
                T::check_tot(nrows, tot, conf.allow_tot_mismatch)
                    .map(|_| nrows)
                    .inner_into()
                    .errors_liftio()
            })
            .and_maybe(|nrows| {
                self.h_read_unchecked_df::<R, I, B, ReadErr>(h, nrows, buf)
                    .map_err(|e| e.inner_into())
                    .into_deferred()
            })
    }

    fn h_read_unchecked_df<R: Read, I, B, E>(
        &self,
        h: &mut BufReader<R>,
        nrows: usize,
        buf: &mut B,
    ) -> IOResult<FCSDataFrame, E>
    where
        S: Copy,
        C: IsFixed + Copy,
        I: Readable<S, E, Inner = C, Buf = B>,
    {
        let mut col_readers: Vec<_> = self.columns.iter().map(|c| I::new(*c, nrows)).collect();
        for row in 0..nrows {
            for c in col_readers.iter_mut() {
                c.h_read_row(h, row, self.byte_layout, buf)
                    .map_err(|e| e.inner_into())?;
            }
        }
        let data = col_readers.into_iter().map(|c| c.into_column()).collect();
        Ok(FCSDataFrame::try_new(data).unwrap())
    }

    fn check_writer<'a, I>(&self, df: &'a FCSDataFrame) -> MultiResult<(), AnyLossError>
    where
        C: Copy,
        I: Writable<'a, S, Inner = C>,
    {
        // ASSUME df has same number of columns as layout
        self.columns
            .iter()
            .zip(df.iter_columns())
            .map(|(col_type, col_data)| I::check_writer(*col_type, col_data))
            .gather()
            .void()
    }

    fn h_write_df<'a, W: Write, I>(
        &self,
        h: &mut BufWriter<W>,
        df: &'a FCSDataFrame,
    ) -> io::Result<()>
    where
        S: Copy,
        C: Copy,
        I: Writable<'a, S, Inner = C>,
    {
        let nrows = df.nrows();
        // ASSUME df has same number of columns as layout
        let mut cs: Vec<_> = self
            .columns
            .iter()
            .zip(df.iter_columns())
            .map(|(col_type, col_data)| I::new(*col_type, col_data))
            .collect();
        for _ in 0..nrows {
            for c in cs.iter_mut() {
                c.h_write(h, self.byte_layout)?;
            }
        }
        Ok(())
    }
}

impl<T, const LEN: usize> HasDatatype for UintType<T, LEN> {
    const DATATYPE: AlphaNumType = AlphaNumType::Integer;
}

impl HasDatatype for F32Type {
    const DATATYPE: AlphaNumType = AlphaNumType::Single;
}

impl HasDatatype for F64Type {
    const DATATYPE: AlphaNumType = AlphaNumType::Double;
}

impl HasDatatype for NullAnyUintType {
    const DATATYPE: AlphaNumType = AlphaNumType::Integer;
}

impl<T, const LEN: usize> IsFixed for UintType<T, LEN>
where
    Self: HasNativeWidth,
    u64: From<T>,
    T: Copy,
{
    fn nbytes(&self) -> u8 {
        Self::BYTES.into()
    }

    fn fixed_width(&self) -> BitsOrChars {
        Self::BYTES.into()
    }

    fn range(&self) -> Range {
        let x = u64::from(self.bitmask);
        // TODO fix u64 max
        Range(if x == u64::MAX { x } else { x + 1 }.into())
    }
}

impl IsFixed for NullAnyUintType {
    fn nbytes(&self) -> u8 {
        match_many_to_one!(
            self,
            Self,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            x,
            { x.nbytes() }
        )
    }

    fn fixed_width(&self) -> BitsOrChars {
        match_many_to_one!(
            self,
            Self,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            x,
            { x.fixed_width() }
        )
    }

    fn range(&self) -> Range {
        match_many_to_one!(
            self,
            Self,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            x,
            { x.range() }
        )
    }
}

impl<T, const LEN: usize> IsFixed for FloatType<T, LEN>
where
    Self: HasNativeWidth,
    T: Copy,
    f64: From<T>,
{
    fn nbytes(&self) -> u8 {
        Self::BYTES.into()
    }

    fn fixed_width(&self) -> BitsOrChars {
        Self::BYTES.into()
    }

    // TODO this will fail if NaN
    fn range(&self) -> Range {
        Range(f64::from(self.range).try_into().unwrap())
    }
}

impl IsFixed for AsciiType {
    fn nbytes(&self) -> u8 {
        self.chars.into()
    }

    fn fixed_width(&self) -> BitsOrChars {
        self.chars.into()
    }

    fn range(&self) -> Range {
        Range(self.range.into())
    }
}

impl IsFixed for NullMixedType {
    fn nbytes(&self) -> u8 {
        match_many_to_one!(self, Self, [Ascii, Uint, F32, F64], x, { x.nbytes() })
    }

    fn fixed_width(&self) -> BitsOrChars {
        match_many_to_one!(self, Self, [Ascii, Uint, F32, F64], x, { x.fixed_width() })
    }

    fn range(&self) -> Range {
        match_many_to_one!(self, Self, [Ascii, Uint, F32, F64], x, { x.range() })
    }
}

// impl<T, const LEN: usize> IsFixedReader for UintType<T, LEN>
// where
//     T: Copy,
//     T: Default,
//     AlphaNumColumnReader: From<UintColumnReader<T, LEN>>,
// {
//     type S = SizedByteOrd<LEN>;
//     fn into_col_reader(self, nrows: usize, byte_layout: Self::S) -> AlphaNumColumnReader {
//         UintColumnReader {
//             column: vec![T::default(); nrows],
//             uint_type: self,
//             size: byte_layout,
//         }
//         .into()
//     }
// }

// impl IsFixedReader for NullAnyUintType {
//     type S = Endian;
//     fn into_col_reader(self, nrows: usize, byte_layout: Self::S) -> AlphaNumColumnReader {
//         match_many_to_one!(
//             self,
//             AnyUintType,
//             [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
//             x,
//             { x.into_col_reader(nrows, byte_layout.into()) }
//         )
//     }
// }

// impl<T, const LEN: usize> IsFixedReader for FloatType<T, LEN>
// where
//     T: Clone,
//     T: Default,
//     AlphaNumColumnReader: From<FloatColumnReader<T, LEN>>,
// {
//     type S = SizedByteOrd<LEN>;
//     fn into_col_reader(self, nrows: usize, byte_layout: Self::S) -> AlphaNumColumnReader {
//         FloatColumnReader {
//             column: vec![T::default(); nrows],
//             byte_layout,
//         }
//         .into()
//     }
// }

// impl IsFixedReader for AsciiType {
//     type S = ();
//     fn into_col_reader(self, nrows: usize, _: ()) -> AlphaNumColumnReader {
//         AlphaNumColumnReader::Ascii(AsciiColumnReader {
//             column: vec![0; nrows],
//             width: self.chars,
//         })
//     }
// }

// impl IsFixedReader for NullMixedType {
//     type S = Endian;
//     fn into_col_reader(self, nrows: usize, byte_layout: Self::S) -> AlphaNumColumnReader {
//         match self {
//             Self::Ascii(a) => a.into_col_reader(nrows, ()),
//             Self::Uint(i) => i.into_col_reader(nrows, byte_layout),
//             Self::F32(f) => f.into_col_reader(nrows, byte_layout.into()),
//             Self::F64(d) => d.into_col_reader(nrows, byte_layout.into()),
//         }
//     }
// }

// impl<T, const LEN: usize> IsFixedWriter for UintType<T, LEN>
// where
//     T: Copy + Ord + AllFCSCast,
//     u64: From<T>,
//     for<'b> AnyFixedColumnWriter<'b>: From<IntColumnWriter<'b, T, LEN>>,
//     for<'b> AnySource<'b, T>: From<FCSColIter<'b, u8, T>>
//         + From<FCSColIter<'b, u16, T>>
//         + From<FCSColIter<'b, u32, T>>
//         + From<FCSColIter<'b, u64, T>>
//         + From<FCSColIter<'b, f32, T>>
//         + From<FCSColIter<'b, f64, T>>,
// {
//     type S = SizedByteOrd<LEN>;

//     fn into_col_writer(
//         self,
//         c: &AnyFCSColumn,
//         check: bool,
//         byte_layout: Self::S,
//     ) -> Result<AnyFixedColumnWriter, AnyLossError> {
//         let bitmask = self.bitmask;
//         match_many_to_one!(c, AnyFCSColumn, [U08, U16, U32, U64, F32, F64], xs, {
//             FCSDataType::into_writer(xs, check, |x: T| {
//                 if x > bitmask {
//                     Some(BitmaskLossError(u64::from(bitmask)))
//                 } else {
//                     None
//                 }
//             })
//             .map(|w| {
//                 IntColumnWriter {
//                     data: w.into(),
//                     size: OrderedUintType {
//                         bitmask,
//                         byte_layout,
//                     },
//                 }
//                 .into()
//             })
//             .map_err(|e| e.into())
//         })
//     }
// }

// impl IsFixedWriter for NullAnyUintType {
//     type S = Endian;

//     fn into_col_writer(
//         self,
//         c: &AnyFCSColumn,
//         check: bool,
//         byte_layout: Self::S,
//     ) -> Result<AnyFixedColumnWriter, AnyLossError> {
//         match_many_to_one!(
//             self,
//             AnyUintType,
//             [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
//             x,
//             { x.into_col_writer(c, check, byte_layout.into()) }
//         )
//     }
// }

// impl<T, const LEN: usize> IsFixedWriter for FloatType<T, LEN>
// where
//     T: NumCast<u8> + NumCast<u16> + NumCast<u32> + NumCast<u64> + NumCast<f32> + NumCast<f64>,
//     for<'b> AnyFixedColumnWriter<'b>: From<FloatColumnWriter<'b, T, LEN>>,
//     for<'b> AnySource<'b, T>: From<FCSColIter<'b, u8, T>>
//         + From<FCSColIter<'b, u16, T>>
//         + From<FCSColIter<'b, u32, T>>
//         + From<FCSColIter<'b, u64, T>>
//         + From<FCSColIter<'b, f32, T>>
//         + From<FCSColIter<'b, f64, T>>,
// {
//     type S = SizedByteOrd<LEN>;

//     fn into_col_writer(
//         self,
//         c: &AnyFCSColumn,
//         check: bool,
//         byte_layout: Self::S,
//     ) -> Result<AnyFixedColumnWriter, AnyLossError> {
//         match_many_to_one!(c, AnyFCSColumn, [U08, U16, U32, U64, F32, F64], xs, {
//             FCSDataType::into_writer(xs, check, |_| None)
//                 .map(|w| {
//                     FloatColumnWriter {
//                         data: w.into(),
//                         size: byte_layout,
//                     }
//                     .into()
//                 })
//                 .map_err(AnyLossError::Int)
//         })
//     }
// }

// impl IsFixedWriter for AsciiType {
//     type S = ();
//     fn into_col_writer(
//         self,
//         col: &AnyFCSColumn,
//         check: bool,
//         _: (),
//     ) -> Result<AnyFixedColumnWriter, AnyLossError> {
//         let c = self.chars;
//         let width = u8::from(c);
//         let go = |x: u64| {
//             if ascii_nbytes(x) > width.into() {
//                 Some(AsciiLossError(width))
//             } else {
//                 None
//             }
//         };
//         match_many_to_one!(col, AnyFCSColumn, [U08, U16, U32, U64, F32, F64], xs, {
//             FCSDataType::into_writer(xs, check, go)
//                 .map(|data| data.into())
//                 .map(|data| AsciiColumnWriter { data, size: c })
//         })
//         .map(AnyFixedColumnWriter::Ascii)
//         .map_err(|e| e.into())
//     }
// }

// impl IsFixedWriter for NullMixedType {
//     type S = Endian;
//     fn into_col_writer(
//         self,
//         c: &AnyFCSColumn,
//         check: bool,
//         byte_layout: Endian,
//     ) -> Result<AnyFixedColumnWriter, AnyLossError> {
//         match self {
//             Self::Ascii(a) => a.into_col_writer(c, check, ()),
//             Self::Uint(i) => i.into_col_writer(c, check, byte_layout),
//             Self::F32(f) => f.into_col_writer(c, check, byte_layout.into()),
//             Self::F64(d) => d.into_col_writer(c, check, byte_layout.into()),
//         }
//     }
// }

macro_rules! uint_from_reader {
    ($from:path, $wrap:ident) => {
        impl From<$from> for AlphaNumColumnReader {
            fn from(value: $from) -> Self {
                AlphaNumColumnReader::Uint(AnyUintColumnReader::$wrap(value))
            }
        }
    };
}

uint_from_reader!(UintColumnReader<u8, 1>, Uint08);
uint_from_reader!(UintColumnReader<u16, 2>, Uint16);
uint_from_reader!(UintColumnReader<u32, 3>, Uint24);
uint_from_reader!(UintColumnReader<u32, 4>, Uint32);
uint_from_reader!(UintColumnReader<u64, 5>, Uint40);
uint_from_reader!(UintColumnReader<u64, 6>, Uint48);
uint_from_reader!(UintColumnReader<u64, 7>, Uint56);
uint_from_reader!(UintColumnReader<u64, 8>, Uint64);

macro_rules! source_from_iter {
    ($from:ident, $to:ident, $wrap:ident) => {
        impl<'a> From<FCSColIter<'a, $from, $to>> for AnySource<'a, $to> {
            fn from(value: FCSColIter<'a, $from, $to>) -> Self {
                Self::$wrap(value)
            }
        }
    };
}

source_from_iter!(u8, u8, FromU08);
source_from_iter!(u8, u16, FromU08);
source_from_iter!(u8, u32, FromU08);
source_from_iter!(u8, u64, FromU08);
source_from_iter!(u8, f32, FromU08);
source_from_iter!(u8, f64, FromU08);

source_from_iter!(u16, u8, FromU16);
source_from_iter!(u16, u16, FromU16);
source_from_iter!(u16, u32, FromU16);
source_from_iter!(u16, u64, FromU16);
source_from_iter!(u16, f32, FromU16);
source_from_iter!(u16, f64, FromU16);

source_from_iter!(u32, u8, FromU32);
source_from_iter!(u32, u16, FromU32);
source_from_iter!(u32, u32, FromU32);
source_from_iter!(u32, u64, FromU32);
source_from_iter!(u32, f32, FromU32);
source_from_iter!(u32, f64, FromU32);

source_from_iter!(u64, u8, FromU64);
source_from_iter!(u64, u16, FromU64);
source_from_iter!(u64, u32, FromU64);
source_from_iter!(u64, u64, FromU64);
source_from_iter!(u64, f32, FromU64);
source_from_iter!(u64, f64, FromU64);

source_from_iter!(f32, u8, FromF32);
source_from_iter!(f32, u16, FromF32);
source_from_iter!(f32, u32, FromF32);
source_from_iter!(f32, u64, FromF32);
source_from_iter!(f32, f32, FromF32);
source_from_iter!(f32, f64, FromF32);

source_from_iter!(f64, u8, FromF64);
source_from_iter!(f64, u16, FromF64);
source_from_iter!(f64, u32, FromF64);
source_from_iter!(f64, u64, FromF64);
source_from_iter!(f64, f32, FromF64);
source_from_iter!(f64, f64, FromF64);

macro_rules! uint_from_writer {
    ($totype:ident, $len:expr, $wrap:ident) => {
        impl<'a> From<IntColumnWriter<'a, $totype, $len>> for AnyFixedColumnWriter<'a> {
            fn from(value: IntColumnWriter<'a, $totype, $len>) -> Self {
                Self::$wrap(value)
            }
        }
    };
}

uint_from_writer!(u8, 1, U08);
uint_from_writer!(u16, 2, U16);
uint_from_writer!(u32, 3, U24);
uint_from_writer!(u32, 4, U32);
uint_from_writer!(u64, 5, U40);
uint_from_writer!(u64, 6, U48);
uint_from_writer!(u64, 7, U56);
uint_from_writer!(u64, 8, U64);

macro_rules! float_from_writer {
    ($totype:ident, $len:expr, $wrap:ident) => {
        impl<'a> From<FloatColumnWriter<'a, $totype, $len>> for AnyFixedColumnWriter<'a> {
            fn from(value: FloatColumnWriter<'a, $totype, $len>) -> Self {
                AnyFixedColumnWriter::$wrap(value)
            }
        }
    };
}

float_from_writer!(f32, 4, F32);
float_from_writer!(f64, 8, F64);

impl<T, const INTLEN: usize> UintColumnReader<T, INTLEN> {
    fn h_read<R: Read>(&mut self, h: &mut BufReader<R>, row: usize) -> io::Result<()>
    where
        T: IntFromBytes<INTLEN>,
        T: Ord,
    {
        let x = T::h_read_ordered(h, self.size)?;
        self.column[row] = x.min(self.uint_type.bitmask);
        Ok(())
    }
}

impl<T, const LEN: usize> FloatColumnReader<T, LEN> {
    fn h_read<R: Read>(&mut self, h: &mut BufReader<R>, row: usize) -> io::Result<()>
    where
        T: FloatFromBytes<LEN>,
    {
        self.column[row] = T::h_read_ordered(h, self.byte_layout)?;
        Ok(())
    }
}

impl From<FloatColumnReader<f32, 4>> for AlphaNumColumnReader {
    fn from(value: FloatColumnReader<f32, 4>) -> Self {
        AlphaNumColumnReader::Float(FloatReader::F32(value))
    }
}

impl From<FloatColumnReader<f64, 8>> for AlphaNumColumnReader {
    fn from(value: FloatColumnReader<f64, 8>) -> Self {
        AlphaNumColumnReader::Float(FloatReader::F64(value))
    }
}

impl<T> AnyOrderedUintLayout<T> {
    fn layout_values(&self) -> OrderedLayoutValues {
        match_many_to_one!(
            self,
            Self,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            l,
            { l.layout_values(()) }
        )
    }

    fn tot_into<X>(self) -> AnyOrderedUintLayout<X> {
        match_many_to_one!(
            self,
            Self,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            l,
            { l.tot_into().into() }
        )
    }

    fn into_endian(self) -> Result<EndianLayout<NullAnyUintType>, OrderedToEndianError> {
        match_many_to_one!(
            self,
            Self,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            l,
            {
                l.tot_into()
                    .byte_layout_try_into()
                    .map(|x| x.columns_into())
            }
        )
    }

    pub(crate) fn try_new<D>(
        cs: NonEmpty<ColumnLayoutValues<D>>,
        o: ByteOrd,
        notrunc: bool,
    ) -> DeferredResult<Self, ColumnError<BitmaskError>, NewFixedIntLayoutError> {
        let n = o.nbytes();
        // First, scan through the widths to make sure they are all fixed and
        // are all the same number of bytes as ByteOrd
        cs.iter()
            .map(|c| Bytes::try_from(c.width))
            .gather()
            .mult_map_errors(SingleFixedWidthError::Bytes)
            .and_then(|widths| {
                NonEmpty::collect(widths.into_iter().filter(|w| *w != n).unique())
                    .map_or(Ok(()), |ws| Err(NonEmpty::new(MultiWidthsError(ws).into())))
            })
            .mult_to_deferred()
            .def_and_maybe(|_| {
                // Second, make the layout, and force all columns to the correct
                // type based on ByteOrd. It is necessary to check the columns
                // first because the bitmask won't necessarily fail even if it
                // is larger than the target type.
                match_many_to_one!(o, ByteOrd, [O1, O2, O3, O4, O5, O6, O7, O8], o, {
                    FixedLayout::try_new(cs, o, |c| {
                        Ok(IntFromBytes::column_type(c.range, notrunc).errors_into())
                    })
                    .def_map_value(|x| x.into())
                })
            })
    }

    fn ncols(&self) -> usize {
        match_many_to_one!(
            self,
            Self,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            l,
            { l.columns.len() }
        )
    }

    fn h_read_df<R: Read, W, E>(
        &self,
        h: &mut BufReader<R>,
        tot: T::Tot,
        seg: AnyDataSegment,
        conf: &ReaderConfig,
    ) -> IODeferredResult<FCSDataFrame, W, E>
    where
        W: From<UnevenEventWidth> + From<TotEventMismatch>,
        E: From<UnevenEventWidth> + From<TotEventMismatch>,
        T: TotDefinition,
    {
        match_many_to_one!(
            self,
            Self,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            l,
            { l.h_read_df_numeric::<_, ColumnReader0<_, _, _>, _, E>(h, tot, seg, conf,) }
        )
    }

    fn check_writer<'a>(&self, df: &'a FCSDataFrame) -> MultiResult<(), AnyLossError> {
        match_many_to_one!(
            self,
            Self,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            l,
            { l.check_writer::<ColumnWriter0<_, _, _>>(df) }
        )
    }

    fn h_write_df<'a, W: Write>(
        &self,
        h: &mut BufWriter<W>,
        df: &'a FCSDataFrame,
    ) -> io::Result<()> {
        match_many_to_one!(
            self,
            Self,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            l,
            { l.h_write_df::<_, ColumnWriter0<_, _, _>>(h, df) }
        )
    }

    // fn into_col_reader_inner(
    //     self,
    //     seg: AnyDataSegment,
    //     conf: &ReaderConfig,
    // ) -> Tentative<AlphaNumReader, UnevenEventWidth, UnevenEventWidth> {
    //     match_many_to_one!(
    //         self,
    //         Self,
    //         [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
    //         l,
    //         { l.into_col_reader_inner(seg, conf) }
    //     )
    // }

    // fn into_col_reader<W, E>(
    //     self,
    //     seg: AnyDataSegment,
    //     tot: Tot,
    //     conf: &ReaderConfig,
    // ) -> Tentative<ColumnReader, W, E>
    // where
    //     W: From<TotEventMismatch> + From<UnevenEventWidth>,
    //     E: From<TotEventMismatch> + From<UnevenEventWidth>,
    // {
    //     match_many_to_one!(
    //         self,
    //         Self,
    //         [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
    //         l,
    //         { l.into_col_reader(seg, tot, conf) }
    //     )
    // }

    // fn as_writer<'a>(
    //     &self,
    //     df: &'a FCSDataFrame,
    //     conf: &WriteConfig,
    // ) -> MultiResult<FixedWriter<'a>, ColumnWriterError> {
    //     match_many_to_one!(
    //         self,
    //         Self,
    //         [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
    //         l,
    //         { l.as_writer(df, conf) }
    //     )
    // }
}

impl AsciiType {
    fn try_new(width: Width, range: Range) -> MultiResult<Self, NewAsciiTypeError> {
        let c = Chars::try_from(width).map_err(|e| e.into());
        let r = u64::try_from(range.0).map_err(|e| e.into());
        c.zip(r).map(|(chars, range)| Self { chars, range })
    }
}

impl<T> AnyAsciiLayout<T> {
    fn layout_values<D: Copy, S: Default>(&self, datatype: D) -> LayoutValues<S, D> {
        match self {
            Self::Delimited(x) => x.layout_values(datatype),
            Self::Fixed(x) => x.ascii_layout_values(datatype),
        }
    }

    fn tot_into<X>(self) -> AnyAsciiLayout<X> {
        match self {
            Self::Delimited(x) => AnyAsciiLayout::Delimited(DelimAsciiLayout::new(x.ranges)),
            Self::Fixed(x) => AnyAsciiLayout::Fixed(x.tot_into()),
        }
    }

    pub(crate) fn try_new<D, W: From<ColumnError<X>>, X>(
        cs: NonEmpty<ColumnLayoutValues<D>>,
    ) -> DeferredResult<Self, W, ColumnError<NewAsciiTypeError>> {
        if cs.iter().all(|c| c.width == Width::Variable) {
            ne_map_results(ne_enumerate(cs), |(i, c)| {
                u64::try_from(c.range.0).map_err(|error| {
                    ColumnError {
                        error: error.into(),
                        index: i.into(),
                    }
                    .into()
                })
            })
            .map(|ranges| AnyAsciiLayout::Delimited(DelimAsciiLayout::new(ranges)))
            .mult_to_deferred()
        } else {
            FixedLayout::try_new(cs, (), |c| {
                // dummy type to satisfy constraint
                AsciiType::try_new(c.width, c.range).mult_to_deferred::<_, X>()
            })
            .def_map_value(Self::Fixed)
        }
    }

    fn ncols(&self) -> usize {
        match self {
            Self::Delimited(a) => a.ranges.len(),
            Self::Fixed(l) => l.columns.len(),
        }
    }

    fn h_read_checked_df<R: Read>(
        &self,
        h: &mut BufReader<R>,
        tot: T::Tot,
        seg: AnyDataSegment,
        conf: &ReaderConfig,
    ) -> IODeferredResult<FCSDataFrame, ReadWarning, ReadAsciiError>
    where
        T: TotDefinition,
    {
        match self {
            Self::Fixed(c) => {
                let mut buf = vec![];
                c.h_read_df::<_, ColumnReader0<_, _, _>, _, _, ReadFixedAsciiError, _>(
                    h, &mut buf, tot, seg, conf,
                )
                .def_map_errors(|e| e.inner_into())
            }
            Self::Delimited(l) => l
                .h_read_df(h, tot, seg.inner.len() as usize)
                .map_err(|e| e.inner_into::<ReadDelimAsciiError>().inner_into())
                .into_deferred(),
        }
    }

    fn check_writer<'a>(&self, df: &'a FCSDataFrame) -> MultiResult<(), AnyLossError> {
        match self {
            Self::Fixed(l) => l.check_writer::<ColumnWriter0<_, _, _>>(df),
            Self::Delimited(l) => l.check_writer(df),
        }
    }

    fn h_write_df<'a, W: Write>(
        &self,
        h: &mut BufWriter<W>,
        df: &'a FCSDataFrame,
    ) -> io::Result<()> {
        match self {
            Self::Fixed(l) => l.h_write_df::<_, ColumnWriter0<_, _, _>>(h, df),
            Self::Delimited(l) => l.h_write_df(h, df),
        }
    }

    // fn as_writer<'a>(
    //     &self,
    //     df: &'a FCSDataFrame,
    //     conf: &WriteConfig,
    // ) -> MultiResult<DataWriter<'a>, ColumnWriterError> {
    //     match self {
    //         AnyAsciiLayout::Fixed(a) => a.as_writer(df, conf).map(DataWriter::Fixed),
    //         AnyAsciiLayout::Delimited(_) => {
    //             let ch = conf.check_conversion;
    //             let go = |c: &'a AnyFCSColumn| {
    //                 match_many_to_one!(c, AnyFCSColumn, [U08, U16, U32, U64, F32, F64], xs, {
    //                     FCSDataType::into_writer(xs, ch, |_| None).map(|data| data.into())
    //                 })
    //                 .map(|data| DelimColumnWriter { data, size: () })
    //             };
    //             df.iter_columns()
    //                 .enumerate()
    //                 .map(|(i, c)| {
    //                     go(c).map_err(|error| {
    //                         ColumnWriterError(ColumnError {
    //                             index: i.into(),
    //                             error: AnyLossError::Int(error),
    //                         })
    //                     })
    //                 })
    //                 .gather()
    //                 .map(|columns| {
    //                     DataWriter::Delim(DataWriterInner {
    //                         // TODO not dry
    //                         columns: NonEmpty::from_vec(columns).unwrap(),
    //                         nrows: df.nrows(),
    //                         nbytes: df.ascii_nbytes(),
    //                     })
    //                 })
    //         }
    //     }
    // }

    // fn into_col_reader_maybe_rows(
    //     self,
    //     seg: AnyDataSegment,
    //     kw_tot: Option<Tot>,
    //     conf: &ReaderConfig,
    // ) -> Tentative<ColumnReader, UnevenEventWidth, UnevenEventWidth> {
    //     let nbytes = seg.inner.len() as usize;
    //     match self {
    //         AnyAsciiLayout::Delimited(dl) => {
    //             Tentative::new1(dl.into_col_reader_maybe_rows(nbytes, kw_tot))
    //         }
    //         AnyAsciiLayout::Fixed(fl) => fl
    //             .into_col_reader_inner(seg, conf)
    //             .map(ColumnReader::AlphaNum),
    //     }
    // }

    // fn into_col_reader<W, E>(
    //     self,
    //     seg: AnyDataSegment,
    //     tot: Tot,
    //     conf: &ReaderConfig,
    // ) -> Tentative<ColumnReader, W, E>
    // where
    //     W: From<UnevenEventWidth>,
    //     E: From<UnevenEventWidth>,
    //     W: From<TotEventMismatch>,
    //     E: From<TotEventMismatch>,
    // {
    //     let nbytes = seg.inner.len() as usize;
    //     match self {
    //         AnyAsciiLayout::Delimited(dl) => Tentative::new1(dl.into_col_reader(nbytes, tot)),
    //         AnyAsciiLayout::Fixed(fl) => fl.into_col_reader(seg, tot, conf),
    //     }
    // }
}

impl VersionedDataLayout for Layout2_0 {
    type S = ByteOrd;
    type D = ();
    type T = Option<Tot>;

    fn try_new(
        datatype: AlphaNumType,
        byteord: Self::S,
        columns: NonEmpty<ColumnLayoutValues<Self::D>>,
        conf: &SharedConfig,
    ) -> DeferredResult<Self, ColumnError<BitmaskError>, NewDataLayoutError> {
        AnyOrderedLayout::try_new(datatype, byteord, columns, conf).def_map_value(|x| x.into())
    }

    fn lookup(
        kws: &mut StdKeywords,
        conf: &SharedConfig,
        par: Par,
    ) -> LookupLayoutResult<Option<Self>> {
        AnyOrderedLayout::lookup(kws, conf, par).def_map_value(|x| x.map(|y| y.into()))
    }

    fn lookup_ro(kws: &StdKeywords, conf: &SharedConfig) -> FromRawResult<Option<Self>> {
        AnyOrderedLayout::lookup_ro(kws, conf).def_map_value(|x| x.map(|y| y.into()))
    }

    fn ncols(&self) -> usize {
        self.0.ncols()
    }

    fn h_read_dataframe<R: Read>(
        &self,
        h: &mut BufReader<R>,
        tot: Self::T,
        seg: AnyDataSegment,
        conf: &ReaderConfig,
    ) -> IODeferredResult<FCSDataFrame, ReadWarning, ReadDataError0> {
        self.0.h_read_checked_df(h, tot, seg, conf)
    }

    fn check_writer<'a>(&self, df: &'a FCSDataFrame) -> MultiResult<(), AnyLossError> {
        self.0.check_writer(df)
    }

    fn h_write_df_inner<'a, W: Write>(
        &self,
        h: &mut BufWriter<W>,
        df: &'a FCSDataFrame,
    ) -> io::Result<()> {
        self.0.h_write_df(h, df)
    }

    // fn as_writer_inner<'a>(
    //     &self,
    //     df: &'a FCSDataFrame,
    //     conf: &WriteConfig,
    // ) -> MultiResult<DataWriter<'a>, ColumnWriterError> {
    //     self.0.as_writer_inner(df, conf)
    // }

    // fn into_data_reader(
    //     self,
    //     kws: &mut StdKeywords,
    //     seg: HeaderDataSegment,
    //     conf: &ReaderConfig,
    // ) -> DataReaderResult<DataReader> {
    //     let out = Tot::remove_metaroot_opt(kws)
    //         .map(|x| x.0)
    //         .map_or_else(
    //             |w| Tentative::new(None, vec![w.into()], vec![]),
    //             Tentative::new1,
    //         )
    //         .and_tentatively(|maybe_tot| self.into_reader(maybe_tot, seg.into_any(), conf));
    //     Ok(out)
    // }

    // fn into_data_reader_raw(
    //     self,
    //     kws: &StdKeywords,
    //     seg: HeaderDataSegment,
    //     conf: &ReaderConfig,
    // ) -> DataReaderResult<DataReader> {
    //     let out = Tot::get_metaroot_opt(kws)
    //         .map(|x| x.0)
    //         .map_or_else(
    //             |w| Tentative::new(None, vec![w.into()], vec![]),
    //             Tentative::new1,
    //         )
    //         .and_tentatively(|maybe_tot| self.into_reader(maybe_tot, seg.into_any(), conf));
    //     Ok(out)
    // }

    fn as_analysis_reader(
        _: &mut StdKeywords,
        seg: HeaderAnalysisSegment,
        _: &ReaderConfig,
    ) -> AnalysisReaderResult<AnalysisReader> {
        Ok(Tentative::new1(AnalysisReader {
            seg: seg.into_any(),
        }))
    }

    fn as_analysis_reader_raw(
        _: &StdKeywords,
        seg: HeaderAnalysisSegment,
        _: &ReaderConfig,
    ) -> AnalysisReaderResult<AnalysisReader> {
        Ok(Tentative::new1(AnalysisReader {
            seg: seg.into_any(),
        }))
    }

    fn layout_values(&self) -> OrderedLayoutValues {
        self.0.layout_values()
    }
}

impl VersionedDataLayout for Layout3_0 {
    type S = ByteOrd;
    type D = ();
    type T = Tot;

    fn try_new(
        datatype: AlphaNumType,
        byteord: Self::S,
        columns: NonEmpty<ColumnLayoutValues<Self::D>>,
        conf: &SharedConfig,
    ) -> DeferredResult<Self, ColumnError<BitmaskError>, NewDataLayoutError> {
        AnyOrderedLayout::try_new(datatype, byteord, columns, conf).def_map_value(|x| x.into())
    }

    fn lookup(
        kws: &mut StdKeywords,
        conf: &SharedConfig,
        par: Par,
    ) -> LookupLayoutResult<Option<Self>> {
        AnyOrderedLayout::lookup(kws, conf, par).def_map_value(|x| x.map(|y| y.into()))
    }

    fn lookup_ro(kws: &StdKeywords, conf: &SharedConfig) -> FromRawResult<Option<Self>> {
        AnyOrderedLayout::lookup_ro(kws, conf).def_map_value(|x| x.map(|y| y.into()))
    }

    fn ncols(&self) -> usize {
        self.0.ncols()
    }

    fn h_read_dataframe<R: Read>(
        &self,
        h: &mut BufReader<R>,
        tot: Self::T,
        seg: AnyDataSegment,
        conf: &ReaderConfig,
    ) -> IODeferredResult<FCSDataFrame, ReadWarning, ReadDataError0> {
        self.0.h_read_checked_df(h, tot, seg, conf)
    }

    fn check_writer<'a>(&self, df: &'a FCSDataFrame) -> MultiResult<(), AnyLossError> {
        self.0.check_writer(df)
    }

    fn h_write_df_inner<'a, W: Write>(
        &self,
        h: &mut BufWriter<W>,
        df: &'a FCSDataFrame,
    ) -> io::Result<()> {
        self.0.h_write_df(h, df)
    }

    // fn as_writer_inner<'a>(
    //     &self,
    //     df: &'a FCSDataFrame,
    //     conf: &WriteConfig,
    // ) -> MultiResult<DataWriter<'a>, ColumnWriterError> {
    //     self.0.as_writer_inner(df, conf)
    // }

    // fn into_data_reader(
    //     self,
    //     kws: &mut StdKeywords,
    //     seg: HeaderDataSegment,
    //     conf: &ReaderConfig,
    // ) -> DataReaderResult<DataReader> {
    //     remove_tot_data_seg(kws, seg, conf)
    //         .def_and_tentatively(|(tot, any_seg)| self.into_reader(tot, any_seg, conf))
    // }

    // fn into_data_reader_raw(
    //     self,
    //     kws: &StdKeywords,
    //     seg: HeaderDataSegment,
    //     conf: &ReaderConfig,
    // ) -> DataReaderResult<DataReader> {
    //     get_tot_data_seg(kws, seg, conf)
    //         .def_and_tentatively(|(tot, any_seg)| self.into_reader(tot, any_seg, conf))
    // }

    fn as_analysis_reader(
        kws: &mut StdKeywords,
        seg: HeaderAnalysisSegment,
        conf: &ReaderConfig,
    ) -> AnalysisReaderResult<AnalysisReader> {
        remove_analysis_seg_req(kws, seg, conf)
    }

    fn as_analysis_reader_raw(
        kws: &StdKeywords,
        seg: HeaderAnalysisSegment,
        conf: &ReaderConfig,
    ) -> AnalysisReaderResult<AnalysisReader> {
        get_analysis_seg_req(kws, seg, conf)
    }

    fn layout_values(&self) -> OrderedLayoutValues {
        self.0.layout_values()
    }
}

impl VersionedDataLayout for Layout3_1 {
    type S = Endian;
    type D = ();
    type T = Tot;

    fn try_new(
        datatype: AlphaNumType,
        endian: Self::S,
        columns: NonEmpty<ColumnLayoutValues<Self::D>>,
        conf: &SharedConfig,
    ) -> DeferredResult<Self, ColumnError<BitmaskError>, NewDataLayoutError> {
        NonMixedEndianLayout::try_new(datatype, endian, columns, conf).def_map_value(|x| x.into())
    }

    fn lookup(
        kws: &mut StdKeywords,
        conf: &SharedConfig,
        par: Par,
    ) -> LookupLayoutResult<Option<Self>> {
        let cs = ColumnLayoutValues2_0::lookup_all(kws, par);
        let d = AlphaNumType::lookup_req(kws);
        let n = Endian::lookup_req(kws);
        // TODO not DRY
        d.def_zip3(n, cs)
            .def_inner_into()
            .def_and_maybe(|(datatype, byteord, columns)| {
                def_transpose(
                    NonEmpty::from_vec(columns)
                        .map(|cs| Self::try_new(datatype, byteord, cs, conf)),
                )
                .def_inner_into()
            })
    }

    fn lookup_ro(kws: &StdKeywords, conf: &SharedConfig) -> FromRawResult<Option<Self>> {
        let cs = ColumnLayoutValues2_0::get_all(kws);
        let d = AlphaNumType::get_metaroot_req(kws).into_deferred();
        let n = Endian::get_metaroot_req(kws).into_deferred();
        d.def_zip3(n, cs)
            .def_inner_into()
            .def_and_maybe(|(datatype, byteord, columns)| {
                def_transpose(
                    NonEmpty::from_vec(columns)
                        .map(|cs| Self::try_new(datatype, byteord, cs, conf)),
                )
                .def_inner_into()
            })
    }

    fn ncols(&self) -> usize {
        self.0.ncols()
    }

    fn h_read_dataframe<R: Read>(
        &self,
        h: &mut BufReader<R>,
        tot: Self::T,
        seg: AnyDataSegment,
        conf: &ReaderConfig,
    ) -> IODeferredResult<FCSDataFrame, ReadWarning, ReadDataError0> {
        self.0.h_read_df(h, tot, seg, conf)
    }

    fn check_writer<'a>(&self, df: &'a FCSDataFrame) -> MultiResult<(), AnyLossError> {
        self.0.check_writer(df)
    }

    fn h_write_df_inner<'a, W: Write>(
        &self,
        h: &mut BufWriter<W>,
        df: &'a FCSDataFrame,
    ) -> io::Result<()> {
        self.0.h_write_df(h, df)
    }

    // fn as_writer_inner<'a>(
    //     &self,
    //     df: &'a FCSDataFrame,
    //     conf: &WriteConfig,
    // ) -> MultiResult<DataWriter<'a>, ColumnWriterError> {
    //     self.0.as_writer_inner(df, conf)
    // }

    // fn into_data_reader(
    //     self,
    //     kws: &mut StdKeywords,
    //     seg: HeaderDataSegment,
    //     conf: &ReaderConfig,
    // ) -> DataReaderResult<DataReader> {
    //     remove_tot_data_seg(kws, seg, conf)
    //         .def_and_tentatively(|(tot, any_seg)| self.into_reader(tot, any_seg, conf))
    // }

    // fn into_data_reader_raw(
    //     self,
    //     kws: &StdKeywords,
    //     seg: HeaderDataSegment,
    //     conf: &ReaderConfig,
    // ) -> DataReaderResult<DataReader> {
    //     get_tot_data_seg(kws, seg, conf)
    //         .def_and_tentatively(|(tot, any_seg)| self.into_reader(tot, any_seg, conf))
    // }

    fn as_analysis_reader(
        kws: &mut StdKeywords,
        seg: HeaderAnalysisSegment,
        conf: &ReaderConfig,
    ) -> AnalysisReaderResult<AnalysisReader> {
        remove_analysis_seg_req(kws, seg, conf)
    }

    fn as_analysis_reader_raw(
        kws: &StdKeywords,
        seg: HeaderAnalysisSegment,
        conf: &ReaderConfig,
    ) -> AnalysisReaderResult<AnalysisReader> {
        get_analysis_seg_req(kws, seg, conf)
    }

    fn layout_values(&self) -> LayoutValues3_1 {
        self.0.layout_values(())
    }
}

impl VersionedDataLayout for Layout3_2 {
    type S = Endian;
    type D = Option<NumType>;
    type T = Tot;

    fn try_new(
        datatype: AlphaNumType,
        endian: Self::S,
        cs: NonEmpty<ColumnLayoutValues<Self::D>>,
        conf: &SharedConfig,
    ) -> DeferredResult<Self, ColumnError<BitmaskError>, NewDataLayoutError> {
        let unique_dt: Vec<_> = cs
            .iter()
            .map(|c| c.datatype.map(|x| x.into()).unwrap_or(datatype))
            .unique()
            .collect();
        match unique_dt[..] {
            [dt] => {
                let ds = cs
                    // TODO lame...
                    .map(|c| ColumnLayoutValues {
                        width: c.width,
                        range: c.range,
                        datatype: (),
                    });
                NonMixedEndianLayout::try_new(dt, endian, ds, conf).def_map_value(Self::NonMixed)
            }
            _ => FixedLayout::try_new(cs, endian, |c| MixedType::try_new(c, conf))
                .def_map_value(Self::Mixed),
        }
    }

    fn lookup(
        kws: &mut StdKeywords,
        conf: &SharedConfig,
        par: Par,
    ) -> LookupLayoutResult<Option<Self>> {
        let d = AlphaNumType::lookup_req(kws);
        let e = Endian::lookup_req(kws);
        let cs = ColumnLayoutValues3_2::lookup_all(kws, par);
        d.def_zip3(e, cs)
            .def_inner_into()
            .def_and_maybe(|(datatype, endian, columns)| {
                def_transpose(
                    NonEmpty::from_vec(columns).map(|cs| Self::try_new(datatype, endian, cs, conf)),
                )
                .def_inner_into()
            })
    }

    fn lookup_ro(kws: &StdKeywords, conf: &SharedConfig) -> FromRawResult<Option<Self>> {
        let d = AlphaNumType::get_metaroot_req(kws)
            .map_err(RawParsedError::from)
            .into_deferred();
        let e = Endian::get_metaroot_req(kws)
            .map_err(RawParsedError::from)
            .into_deferred();
        let cs = ColumnLayoutValues3_2::get_all(kws).def_inner_into();
        d.def_zip3(e, cs)
            .def_and_maybe(|(datatype, endian, columns)| {
                def_transpose(
                    NonEmpty::from_vec(columns).map(|cs| Self::try_new(datatype, endian, cs, conf)),
                )
                .def_inner_into()
            })
    }

    fn ncols(&self) -> usize {
        match self {
            Self::NonMixed(x) => x.ncols(),
            Self::Mixed(m) => m.ncols(),
        }
    }

    fn h_read_dataframe<R: Read>(
        &self,
        h: &mut BufReader<R>,
        tot: Self::T,
        seg: AnyDataSegment,
        conf: &ReaderConfig,
    ) -> IODeferredResult<FCSDataFrame, ReadWarning, ReadDataError0> {
        match self {
            Self::NonMixed(x) => x.h_read_df(h, tot, seg, conf),
            Self::Mixed(m) => {
                let mut buf = vec![];
                m.h_read_df::<_, ReaderMixedType, _, _, _, _>(h, &mut buf, tot, seg, conf)
            }
        }
    }

    fn check_writer<'a>(&self, df: &'a FCSDataFrame) -> MultiResult<(), AnyLossError> {
        match self {
            Self::NonMixed(x) => x.check_writer(df),
            Self::Mixed(m) => m.check_writer::<WriterMixedType>(df),
        }
    }

    fn h_write_df_inner<'a, W: Write>(
        &self,
        h: &mut BufWriter<W>,
        df: &'a FCSDataFrame,
    ) -> io::Result<()> {
        match self {
            Self::NonMixed(x) => x.h_write_df(h, df),
            Self::Mixed(m) => m.h_write_df::<_, WriterMixedType>(h, df),
        }
    }

    // fn as_writer_inner<'a>(
    //     &self,
    //     df: &'a FCSDataFrame,
    //     conf: &WriteConfig,
    // ) -> MultiResult<DataWriter<'a>, ColumnWriterError> {
    //     match self {
    //         Self::NonMixed(x) => x.as_writer_inner(df, conf),
    //         Self::Mixed(m) => m.as_writer(df, conf).map(DataWriter::Fixed),
    //     }
    // }

    // fn into_data_reader(
    //     self,
    //     kws: &mut StdKeywords,
    //     seg: HeaderDataSegment,
    //     conf: &ReaderConfig,
    // ) -> DataReaderResult<DataReader> {
    //     remove_tot_data_seg(kws, seg, conf)
    //         .def_and_tentatively(|(tot, any_seg)| self.into_reader(tot, any_seg, conf))
    // }

    // fn into_data_reader_raw(
    //     self,
    //     kws: &StdKeywords,
    //     seg: HeaderDataSegment,
    //     conf: &ReaderConfig,
    // ) -> DataReaderResult<DataReader> {
    //     get_tot_data_seg(kws, seg, conf)
    //         .def_and_tentatively(|(tot, any_seg)| self.into_reader(tot, any_seg, conf))
    // }

    fn as_analysis_reader(
        kws: &mut StdKeywords,
        seg: HeaderAnalysisSegment,
        conf: &ReaderConfig,
    ) -> AnalysisReaderResult<AnalysisReader> {
        let ret = KeyedOptSegment::remove_or(
            kws,
            conf.analysis,
            seg,
            conf.allow_header_text_offset_mismatch,
        )
        .map(|s| AnalysisReader { seg: s })
        .inner_into();
        Ok(ret)
    }

    fn as_analysis_reader_raw(
        kws: &StdKeywords,
        seg: HeaderAnalysisSegment,
        conf: &ReaderConfig,
    ) -> AnalysisReaderResult<AnalysisReader> {
        let ret = KeyedOptSegment::get_or(
            kws,
            conf.analysis,
            seg,
            conf.allow_header_text_offset_mismatch,
        )
        .map(|s| AnalysisReader { seg: s })
        .inner_into();
        Ok(ret)
    }

    fn layout_values(&self) -> LayoutValues3_2 {
        match self {
            Self::NonMixed(x) => x.layout_values(None),
            Self::Mixed(x) => x.mixed_layout_values(),
        }
    }
}

fn remove_analysis_seg_req(
    kws: &mut StdKeywords,
    seg: HeaderAnalysisSegment,
    conf: &ReaderConfig,
) -> AnalysisReaderResult<AnalysisReader> {
    KeyedReqSegment::remove_or(
        kws,
        conf.analysis,
        seg,
        conf.allow_header_text_offset_mismatch,
        conf.allow_missing_required_offsets,
    )
    .def_inner_into()
    .def_map_value(|s| AnalysisReader { seg: s })
}

fn get_analysis_seg_req(
    kws: &StdKeywords,
    seg: HeaderAnalysisSegment,
    conf: &ReaderConfig,
) -> AnalysisReaderResult<AnalysisReader> {
    KeyedReqSegment::get_or(
        kws,
        conf.analysis,
        seg,
        conf.allow_header_text_offset_mismatch,
        conf.allow_missing_required_offsets,
    )
    .def_inner_into()
    .def_map_value(|s| AnalysisReader { seg: s })
}

fn remove_tot_data_seg(
    kws: &mut StdKeywords,
    seg: HeaderDataSegment,
    conf: &ReaderConfig,
) -> DataReaderResult<(Tot, AnyDataSegment)> {
    let tot_res = Tot::remove_metaroot_req(kws).into_deferred();
    let seg_res = KeyedReqSegment::remove_or(
        kws,
        conf.data,
        seg,
        conf.allow_header_text_offset_mismatch,
        conf.allow_missing_required_offsets,
    )
    .def_inner_into();
    tot_res.def_zip(seg_res)
}

// impl Layout2_0 {
//     fn into_reader<W, E>(
//         self,
//         tot: Option<Tot>,
//         seg: AnyDataSegment,
//         conf: &ReaderConfig,
//     ) -> Tentative<DataReader, W, E>
//     where
//         W: From<TotEventMismatch> + From<UnevenEventWidth>,
//         E: From<TotEventMismatch> + From<UnevenEventWidth>,
//     {
//         let go = |tnt: Tentative<AlphaNumReader, _, _>, maybe_tot| {
//             tnt.inner_into()
//                 .and_tentatively(|reader| {
//                     if let Some(_tot) = maybe_tot {
//                         reader
//                             .check_tot(_tot, conf.allow_tot_mismatch)
//                             .inner_into()
//                             .map(|_| reader)
//                     } else {
//                         Tentative::new1(reader)
//                     }
//                 })
//                 .map(ColumnReader::AlphaNum)
//         };
//         match self.0 {
//             AnyOrderedLayout::Ascii(a) => a.into_col_reader_maybe_rows(seg, tot, conf).inner_into(),
//             AnyOrderedLayout::Integer(fl) => go(fl.into_col_reader_inner(seg, conf), tot),
//             AnyOrderedLayout::F32(fl) => go(fl.into_col_reader_inner(seg, conf), tot),
//             AnyOrderedLayout::F64(fl) => go(fl.into_col_reader_inner(seg, conf), tot),
//         }
//         .map(|r| r.into_data_reader(seg))
//     }
// }

// impl Layout3_0 {
//     fn into_reader<W, E>(
//         self,
//         tot: Tot,
//         seg: AnyDataSegment,
//         conf: &ReaderConfig,
//     ) -> Tentative<DataReader, W, E>
//     where
//         W: From<UnevenEventWidth>,
//         E: From<UnevenEventWidth>,
//         W: From<TotEventMismatch>,
//         E: From<TotEventMismatch>,
//     {
//         match self.0 {
//             AnyOrderedLayout::Ascii(a) => a.into_col_reader(seg, tot, conf),
//             AnyOrderedLayout::Integer(fl) => fl.into_col_reader(seg, tot, conf),
//             AnyOrderedLayout::F32(fl) => fl.into_col_reader(seg, tot, conf),
//             AnyOrderedLayout::F64(fl) => fl.into_col_reader(seg, tot, conf),
//         }
//         .map(|r| r.into_data_reader(seg))
//     }
// }

impl Layout3_1 {
    // fn into_reader<W, E>(
    //     self,
    //     tot: Tot,
    //     seg: AnyDataSegment,
    //     conf: &ReaderConfig,
    // ) -> Tentative<DataReader, W, E>
    // where
    //     W: From<UnevenEventWidth>,
    //     E: From<UnevenEventWidth>,
    //     W: From<TotEventMismatch>,
    //     E: From<TotEventMismatch>,
    // {
    //     self.0.into_reader(tot, seg, conf)
    // }

    pub(crate) fn into_ordered<T>(self) -> LayoutConvertResult<AnyOrderedLayout<T>> {
        self.0.into_ordered()
    }
}

impl Layout3_2 {
    // fn into_reader<W, E>(
    //     self,
    //     tot: Tot,
    //     seg: AnyDataSegment,
    //     conf: &ReaderConfig,
    // ) -> Tentative<DataReader, W, E>
    // where
    //     W: From<UnevenEventWidth>,
    //     E: From<UnevenEventWidth>,
    //     W: From<TotEventMismatch>,
    //     E: From<TotEventMismatch>,
    // {
    //     match self {
    //         Self::NonMixed(x) => x.into_reader(tot, seg, conf),
    //         Self::Mixed(fl) => fl
    //             .into_col_reader(seg, tot, conf)
    //             .map(|r| r.into_data_reader(seg)),
    //     }
    // }

    pub(crate) fn into_ordered<T>(self) -> LayoutConvertResult<AnyOrderedLayout<T>> {
        match self {
            Self::NonMixed(x) => x.into_ordered(),
            Self::Mixed(x) => x.try_into_ordered().mult_errors_into(),
        }
    }
}

impl<T> AnyOrderedLayout<T> {
    fn layout_values(&self) -> OrderedLayoutValues {
        match self {
            Self::Ascii(x) => x.layout_values(()),
            Self::Integer(x) => x.layout_values(),
            Self::F32(x) => x.layout_values(()),
            Self::F64(x) => x.layout_values(()),
        }
    }

    fn try_new(
        datatype: AlphaNumType,
        byteord: ByteOrd,
        columns: NonEmpty<ColumnLayoutValues<()>>,
        conf: &SharedConfig,
    ) -> DeferredResult<Self, ColumnError<BitmaskError>, NewDataLayoutError> {
        match datatype {
            AlphaNumType::Ascii => AnyAsciiLayout::try_new(columns)
                .def_map_value(Self::Ascii)
                .def_errors_into(),
            AlphaNumType::Integer => {
                AnyOrderedUintLayout::try_new(columns, byteord, conf.disallow_bitmask_truncation)
                    .def_map_value(Self::Integer)
                    .def_inner_into()
            }
            AlphaNumType::Single => byteord.try_into().into_deferred().def_and_maybe(|b| {
                FixedLayout::try_new(columns, b, |c| {
                    f32::column_type(c.width, c.range).into_deferred::<FloatWidthError, _>()
                })
                .def_map_value(Self::F32)
            }),
            AlphaNumType::Double => byteord.try_into().into_deferred().def_and_maybe(|b| {
                FixedLayout::try_new(columns, b, |c| {
                    f64::column_type(c.width, c.range).into_deferred::<FloatWidthError, _>()
                })
                .def_map_value(Self::F64)
            }),
        }
    }

    fn lookup(
        kws: &mut StdKeywords,
        conf: &SharedConfig,
        par: Par,
    ) -> LookupLayoutResult<Option<Self>> {
        let cs = ColumnLayoutValues2_0::lookup_all(kws, par);
        let d = AlphaNumType::lookup_req(kws);
        let b = ByteOrd::lookup_req(kws);
        d.def_zip3(b, cs)
            .def_inner_into()
            .def_and_maybe(|(datatype, byteord, columns)| {
                def_transpose(
                    NonEmpty::from_vec(columns)
                        .map(|cs| Self::try_new(datatype, byteord, cs, conf)),
                )
                .def_inner_into()
            })
    }

    fn lookup_ro(kws: &StdKeywords, conf: &SharedConfig) -> FromRawResult<Option<Self>> {
        let cs = ColumnLayoutValues2_0::get_all(kws);
        let d = AlphaNumType::get_metaroot_req(kws).into_deferred();
        let b = ByteOrd::get_metaroot_req(kws).into_deferred();
        d.def_zip3(b, cs)
            .def_inner_into()
            .def_and_maybe(|(datatype, byteord, columns)| {
                def_transpose(
                    NonEmpty::from_vec(columns)
                        .map(|cs| Self::try_new(datatype, byteord, cs, conf)),
                )
                .def_inner_into()
            })
    }

    fn ncols(&self) -> usize {
        match self {
            Self::Ascii(a) => a.ncols(),
            Self::Integer(i) => i.ncols(),
            Self::F32(f) => f.ncols(),
            Self::F64(f) => f.ncols(),
        }
    }

    pub(crate) fn tot_into<X>(self) -> AnyOrderedLayout<X> {
        match self {
            Self::Ascii(a) => AnyOrderedLayout::Ascii(a.tot_into()),
            Self::Integer(i) => AnyOrderedLayout::Integer(i.tot_into()),
            Self::F32(f) => AnyOrderedLayout::F32(f.tot_into()),
            Self::F64(f) => AnyOrderedLayout::F64(f.tot_into()),
        }
    }

    fn h_read_checked_df<R: Read>(
        &self,
        h: &mut BufReader<R>,
        tot: T::Tot,
        seg: AnyDataSegment,
        conf: &ReaderConfig,
    ) -> IODeferredResult<FCSDataFrame, ReadWarning, ReadDataError0>
    where
        T: TotDefinition,
    {
        match self {
            Self::Ascii(x) => x
                .h_read_checked_df(h, tot, seg, conf)
                .def_map_errors(|e| e.inner_into()),
            Self::Integer(x) => x.h_read_df(h, tot, seg, conf),
            Self::F32(x) => {
                x.h_read_df_numeric::<_, ColumnReader0<_, _, _>, _, _>(h, tot, seg, conf)
            }
            Self::F64(x) => {
                x.h_read_df_numeric::<_, ColumnReader0<_, _, _>, _, _>(h, tot, seg, conf)
            }
        }
    }

    fn check_writer<'a>(&self, df: &'a FCSDataFrame) -> MultiResult<(), AnyLossError>
    where
        T: TotDefinition,
    {
        match self {
            Self::Ascii(x) => x.check_writer(df),
            Self::Integer(x) => x.check_writer(df),
            Self::F32(x) => x.check_writer::<ColumnWriter0<_, _, _>>(df),
            Self::F64(x) => x.check_writer::<ColumnWriter0<_, _, _>>(df),
        }
    }

    fn h_write_df<'a, W: Write>(&self, h: &mut BufWriter<W>, df: &'a FCSDataFrame) -> io::Result<()>
    where
        T: TotDefinition,
    {
        match self {
            Self::Ascii(x) => x.h_write_df(h, df),
            Self::Integer(x) => x.h_write_df(h, df),
            Self::F32(x) => x.h_write_df::<_, ColumnWriter0<_, _, _>>(h, df),
            Self::F64(x) => x.h_write_df::<_, ColumnWriter0<_, _, _>>(h, df),
        }
    }

    // fn as_writer_inner<'a>(
    //     &self,
    //     df: &'a FCSDataFrame,
    //     conf: &WriteConfig,
    // ) -> MultiResult<DataWriter<'a>, ColumnWriterError> {
    //     match self {
    //         Self::Ascii(a) => a.as_writer(df, conf),
    //         Self::Integer(i) => i.as_writer(df, conf).map(DataWriter::Fixed),
    //         Self::F32(f) => f.as_writer(df, conf).map(DataWriter::Fixed),
    //         Self::F64(f) => f.as_writer(df, conf).map(DataWriter::Fixed),
    //     }
    // }

    pub fn into_unmixed(self) -> LayoutConvertResult<NonMixedEndianLayout> {
        match self {
            Self::Ascii(x) => Ok(NonMixedEndianLayout::Ascii(x.tot_into())),
            Self::Integer(x) => x.into_endian().map(NonMixedEndianLayout::Integer),
            Self::F32(x) => x
                .tot_into()
                .byte_layout_try_into()
                .map(NonMixedEndianLayout::F32),
            Self::F64(x) => x
                .tot_into()
                .byte_layout_try_into()
                .map(NonMixedEndianLayout::F64),
        }
        .into_mult()
    }

    pub(crate) fn into_3_1(self) -> LayoutConvertResult<Layout3_1> {
        self.into_unmixed().map(|x| x.into())
    }

    pub(crate) fn into_3_2(self) -> LayoutConvertResult<Layout3_2> {
        self.into_unmixed().map(Layout3_2::NonMixed)
    }
}

impl NonMixedEndianLayout {
    fn layout_values<D: Copy>(&self, datatype: D) -> LayoutValues<Endian, D> {
        match self {
            Self::Ascii(x) => x.layout_values(datatype),
            Self::Integer(x) => x.layout_values(datatype),
            Self::F32(x) => x.layout_values(datatype),
            Self::F64(x) => x.layout_values(datatype),
        }
    }

    fn try_new(
        datatype: AlphaNumType,
        endian: Endian,
        columns: NonEmpty<ColumnLayoutValues<()>>,
        conf: &SharedConfig,
    ) -> DeferredResult<Self, ColumnError<BitmaskError>, NewDataLayoutError> {
        match datatype {
            AlphaNumType::Ascii => AnyAsciiLayout::try_new(columns)
                .def_map_value(Self::Ascii)
                .def_errors_into(),
            AlphaNumType::Integer => {
                FixedLayout::endian_uint_try_new(columns, endian, conf.disallow_bitmask_truncation)
                    .def_map_value(Self::Integer)
                    .def_inner_into()
            }
            AlphaNumType::Single => FixedLayout::try_new(columns, endian, |c| {
                f32::column_type(c.width, c.range).into_deferred::<FloatWidthError, _>()
            })
            .def_map_value(Self::F32),
            AlphaNumType::Double => FixedLayout::try_new(columns, endian, |c| {
                f64::column_type(c.width, c.range).into_deferred::<FloatWidthError, _>()
            })
            .def_map_value(Self::F64),
        }
    }

    fn h_read_df<R: Read>(
        &self,
        h: &mut BufReader<R>,
        tot: Tot,
        seg: AnyDataSegment,
        conf: &ReaderConfig,
    ) -> IODeferredResult<FCSDataFrame, ReadWarning, ReadDataError0> {
        match self {
            Self::Ascii(x) => x
                .h_read_checked_df(h, tot, seg, conf)
                .def_map_errors(|e| e.inner_into()),
            Self::Integer(x) => {
                x.h_read_df_numeric::<_, ReaderAnyUintType, _, _>(h, tot, seg, conf)
            }
            Self::F32(x) => {
                x.h_read_df_numeric::<_, ColumnReader0<_, _, _>, _, _>(h, tot, seg, conf)
            }
            Self::F64(x) => {
                x.h_read_df_numeric::<_, ColumnReader0<_, _, _>, _, _>(h, tot, seg, conf)
            }
        }
    }

    fn check_writer<'a>(&self, df: &'a FCSDataFrame) -> MultiResult<(), AnyLossError> {
        match self {
            Self::Ascii(x) => x.check_writer(df),
            Self::Integer(x) => x.check_writer::<WriterAnyUintType>(df),
            Self::F32(x) => x.check_writer::<ColumnWriter0<_, _, _>>(df),
            Self::F64(x) => x.check_writer::<ColumnWriter0<_, _, _>>(df),
        }
    }

    fn h_write_df<'a, W: Write>(
        &self,
        h: &mut BufWriter<W>,
        df: &'a FCSDataFrame,
    ) -> io::Result<()> {
        match self {
            Self::Ascii(x) => x.h_write_df(h, df),
            Self::Integer(x) => x.h_write_df::<_, WriterAnyUintType>(h, df),
            Self::F32(x) => x.h_write_df::<_, ColumnWriter0<_, _, _>>(h, df),
            Self::F64(x) => x.h_write_df::<_, ColumnWriter0<_, _, _>>(h, df),
        }
    }

    // fn into_reader<W, E>(
    //     self,
    //     tot: Tot,
    //     seg: AnyDataSegment,
    //     conf: &ReaderConfig,
    // ) -> Tentative<DataReader, W, E>
    // where
    //     W: From<UnevenEventWidth>,
    //     E: From<UnevenEventWidth>,
    //     W: From<TotEventMismatch>,
    //     E: From<TotEventMismatch>,
    // {
    //     match self {
    //         Self::Ascii(a) => a.into_col_reader(seg, tot, conf),
    //         Self::Integer(fl) => fl.into_col_reader(seg, tot, conf),
    //         Self::F32(fl) => fl.byte_layout_into().into_col_reader(seg, tot, conf),
    //         Self::F64(fl) => fl.byte_layout_into().into_col_reader(seg, tot, conf),
    //     }
    //     .map(|r| r.into_data_reader(seg))
    // }

    fn ncols(&self) -> usize {
        match self {
            Self::Ascii(a) => a.ncols(),
            Self::Integer(i) => i.ncols(),
            Self::F32(f) => f.ncols(),
            Self::F64(f) => f.ncols(),
        }
    }

    // fn as_writer_inner<'a>(
    //     &self,
    //     df: &'a FCSDataFrame,
    //     conf: &WriteConfig,
    // ) -> MultiResult<DataWriter<'a>, ColumnWriterError> {
    //     match self {
    //         Self::Ascii(a) => a.as_writer(df, conf),
    //         Self::Integer(i) => i.as_writer(df, conf).map(DataWriter::Fixed),
    //         // TODO clone
    //         Self::F32(f) => f
    //             .clone()
    //             .byte_layout_into()
    //             .as_writer(df, conf)
    //             .map(DataWriter::Fixed),
    //         Self::F64(f) => f
    //             .clone()
    //             .byte_layout_into()
    //             .as_writer(df, conf)
    //             .map(DataWriter::Fixed),
    //     }
    // }

    pub(crate) fn into_ordered<T>(self) -> LayoutConvertResult<AnyOrderedLayout<T>> {
        match self {
            Self::Ascii(x) => Ok(AnyOrderedLayout::Ascii(x.tot_into())),
            Self::Integer(x) => x.uint_try_into_ordered().map(AnyOrderedLayout::Integer),
            Self::F32(x) => Ok(AnyOrderedLayout::F32(x.tot_into().byte_layout_into())),
            Self::F64(x) => Ok(AnyOrderedLayout::F64(x.tot_into().byte_layout_into())),
        }
    }
}

fn get_tot_data_seg(
    kws: &StdKeywords,
    seg: HeaderDataSegment,
    conf: &ReaderConfig,
) -> DataReaderResult<(Tot, AnyDataSegment)> {
    let tot_res = Tot::get_metaroot_req(kws).into_deferred();
    let seg_res = KeyedReqSegment::get_or(
        kws,
        conf.data,
        seg,
        conf.allow_header_text_offset_mismatch,
        conf.allow_missing_required_offsets,
    )
    .def_inner_into();
    tot_res.def_zip(seg_res)
}

pub(crate) fn h_read_data_and_analysis<R: Read + Seek>(
    h: &mut BufReader<R>,
    data_reader: DataReader,
    analysis_reader: AnalysisReader,
    others_reader: OthersReader,
) -> IOResult<
    (
        FCSDataFrame,
        Analysis,
        Others,
        AnyDataSegment,
        AnyAnalysisSegment,
    ),
    ReadDataError,
> {
    let dseg = data_reader.seg;
    let data = data_reader.h_read(h)?;
    let analysis = analysis_reader.h_read(h)?;
    let others = others_reader.h_read(h)?;
    Ok((data, analysis, others, dseg, analysis_reader.seg))
}

enum_from_disp!(
    pub AsciiToUintError,
    [NotAscii, NotAsciiError],
    [Int, ParseIntError]
);

pub struct NotAsciiError(Vec<u8>);

enum_from_disp!(
    pub NewDataLayoutError,
    [Ascii,       ColumnError<NewAsciiTypeError>],
    [FixedInt,    NewFixedIntLayoutError],
    [Float,       ColumnError<FloatWidthError>],
    [VariableInt, ColumnError<NewUintTypeError>],
    [Mixed,       ColumnError<NewMixedTypeError>],
    [ByteOrd,     ByteOrdToSizedError]
);

enum_from_disp!(
    pub NewAsciiTypeError,
    [Width, WidthToCharsError],
    [Range, ToIntError<u64>]
);

enum_from_disp!(
    pub NewFixedIntLayoutError,
    [Width, SingleFixedWidthError],
    [Column, ColumnError<IntOrderedColumnError>]
);

enum_from_disp!(
    pub IntOrderedColumnError,
    [Order, ByteOrdToSizedError],
    // TODO sloppy nesting
    [Endian, ByteOrdToSizedEndianError],
    [Size,  BitmaskError]
);

pub enum BitmaskError {
    IntOverrange(u64),
    FloatOverrange(f64),
    FloatUnderrange(f64),
    FloatPrecisionLoss(f64),
}

enum_from_disp!(
    pub SingleFixedWidthError,
    [Bytes, WidthToBytesError],
    [Multi, MultiWidthsError]
);

pub struct MultiWidthsError(pub NonEmpty<Bytes>);

enum_from_disp!(
    pub NewMixedTypeError,
    [Ascii, NewAsciiTypeError],
    [Uint, NewUintTypeError],
    [Float, FloatWidthError]
);

enum_from_disp!(
    pub NewUintTypeError,
    [Bitmask, BitmaskError],
    [Bytes, WidthToBytesError]
);

enum_from_disp!(
    pub NewOrderedUintLayoutError,
    [Column, ColumnError<OrderedFloatError>],
    [ByteOrd, ByteOrdToSizedError]
);

enum_from_disp!(
    pub OrderedFloatError,
    [Order,      ByteOrdToSizedError],
    [WrongWidth, FloatWidthError]
);

enum_from_disp!(
    pub FloatWidthError,
    [Bytes,      WidthToBytesError],
    [WrongWidth, WrongFloatWidth]
);

pub struct WrongFloatWidth {
    pub width: Bytes,
    pub expected: usize,
}

pub type DataReaderResult<T> = DeferredResult<T, NewDataReaderWarning, NewDataReaderError>;

enum_from_disp!(
    pub NewDataReaderError,
    [TotMismatch, TotEventMismatch],
    [ParseTot, ReqKeyError<ParseIntError>],
    [ParseSeg, ReqSegmentWithDefaultError<DataSegmentId>],
    [Width, UnevenEventWidth],
    [Mismatch, SegmentMismatchWarning<DataSegmentId>]
);

enum_from_disp!(
    pub NewDataReaderWarning,
    [TotMismatch, TotEventMismatch],
    [ParseTot, ParseKeyError<ParseIntError>],
    [Layout, ColumnError<BitmaskError>],
    [Width, UnevenEventWidth],
    [Segment, ReqSegmentWithDefaultWarning<DataSegmentId>]
);

pub(crate) type AnalysisReaderResult<T> =
    DeferredResult<T, NewAnalysisReaderWarning, NewAnalysisReaderError>;

enum_from_disp!(
    pub NewAnalysisReaderError,
    [ParseSeg, ReqSegmentWithDefaultError<AnalysisSegmentId>],
    [Mismatch, SegmentMismatchWarning<AnalysisSegmentId>]
);

enum_from_disp!(
    pub NewAnalysisReaderWarning,
    [Opt, OptSegmentWithDefaultWarning<AnalysisSegmentId>],
    [Req, ReqSegmentWithDefaultWarning<AnalysisSegmentId>]
);

pub struct TotEventMismatch {
    tot: Tot,
    total_events: usize,
}

pub struct UnevenEventWidth {
    event_width: usize,
    nbytes: usize,
    remainder: usize,
}

pub struct ColumnWriterError(ColumnError<AnyLossError>);

newtype_disp!(ColumnWriterError);

enum_from_disp!(
    pub AnyLossError,
    [Int, LossError<BitmaskLossError>],
    [Float, LossError<Infallible>],
    [Ascii, LossError<AsciiLossError>]
);

pub struct AsciiLossError(u8);

impl fmt::Display for AsciiLossError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "ASCII data was too big and truncated into {} chars",
            self.0
        )
    }
}

pub struct BitmaskLossError(pub u64);

impl fmt::Display for BitmaskLossError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "integer data was too big and truncated to bitmask {}",
            self.0
        )
    }
}

pub struct ColumnError<E> {
    index: IndexFromOne,
    error: E,
}

type LookupLayoutResult<T> = DeferredResult<T, LookupLayoutWarning, LookupLayoutError>;

enum_from_disp!(
    pub LookupLayoutError,
    [New, NewDataLayoutError],
    [Raw, LookupKeysError]
);

enum_from_disp!(
    pub LookupLayoutWarning,
    [New, ColumnError<BitmaskError>],
    [Raw, LookupKeysWarning]
);

type FromRawResult<T> = DeferredResult<T, RawToLayoutWarning, RawToLayoutError>;

enum_from_disp!(
    pub RawToLayoutError,
    [New, NewDataLayoutError],
    [Raw, RawParsedError]
);

enum_from_disp!(
    pub RawToLayoutWarning,
    [New, ColumnError<BitmaskError>],
    [Raw, ParseKeyError<NumTypeError>]
);

enum_from_disp!(
    pub RawParsedError,
    [AlphaNumType, ReqKeyError<AlphaNumTypeError>],
    [Endian, ReqKeyError<NewEndianError>],
    [ByteOrd, ReqKeyError<ParseByteOrdError>],
    [Int, ReqKeyError<ParseIntError>],
    [Range, ReqKeyError<ParseFloatOrIntError>]
);

enum_from_disp!(
    pub ReadDataError,
    [Delim, ReadDelimWithRowsAsciiError],
    [DelimNoRows, ReadDelimAsciiWithoutRowsError],
    [AlphaNum, AsciiToUintError]
);

enum_from_disp!(
    pub ReadDataError0,
    [Ascii, ReadAsciiError],
    [Uneven, UnevenEventWidth],
    [TotMismatch, TotEventMismatch],
    [Delim, ReadDelimWithRowsAsciiError],
    [DelimNoRows, ReadDelimAsciiWithoutRowsError],
    [AlphaNum, AsciiToUintError]
);

enum_from_disp!(
    pub ReadAsciiError,
    [Delim, ReadDelimAsciiError],
    [Fixed, ReadFixedAsciiError]
);

enum_from_disp!(
    pub ReadFixedAsciiError,
    [Uneven, UnevenEventWidth],
    [Tot, TotEventMismatch],
    [ToUint, AsciiToUintError]
);

enum_from_disp!(
    pub ReadWarning,
    [Uneven, UnevenEventWidth],
    [Tot, TotEventMismatch]
);

enum_from_disp!(
    pub ReadDelimAsciiError,
    [Rows, ReadDelimWithRowsAsciiError],
    [NoRows, ReadDelimAsciiWithoutRowsError]
);

enum_from_disp!(
    pub ReadDelimWithRowsAsciiError,
    [RowsExceeded, RowsExceededError],
    [Incomplete, DelimIncompleteError],
    [Parse, AsciiToUintError]
);

// signify that parsing exceeded max rows
pub struct RowsExceededError(usize);

// signify that a parsing ended in the middle of a row
pub struct DelimIncompleteError {
    col: usize,
    row: usize,
    nrows: usize,
}

pub enum ReadDelimAsciiWithoutRowsError {
    Unequal,
    Parse(AsciiToUintError),
}

impl fmt::Display for BitmaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            // TODO what is the target type?
            Self::IntOverrange(x) => {
                write!(
                    f,
                    "integer range {x} is larger than target unsigned integer can hold"
                )
            }
            Self::FloatOverrange(x) => {
                write!(
                    f,
                    "float range {x} is larger than target unsigned integer can hold"
                )
            }
            Self::FloatUnderrange(x) => {
                write!(
                    f,
                    "float range {x} is less than zero and \
                     could not be converted to unsigned integer"
                )
            }
            Self::FloatPrecisionLoss(x) => {
                write!(
                    f,
                    "float range {x} lost precision when converting to unsigned integer"
                )
            }
        }
    }
}

impl fmt::Display for RowsExceededError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "Exceeded expected number of rows: {}", self.0)
    }
}

impl fmt::Display for DelimIncompleteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "Parsing ended in column {} and row {}, where expected number of rows is {}",
            self.col, self.row, self.nrows
        )
    }
}

impl<E> fmt::Display for ColumnError<E>
where
    E: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "error when processing measurement {}: {}",
            self.index, self.error
        )
    }
}

impl fmt::Display for ReadDelimAsciiWithoutRowsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            Self::Unequal => write!(
                f,
                "parsing delimited ASCII without $TOT \
                 resulted in columns with unequal length"
            ),
            Self::Parse(x) => x.fmt(f),
        }
    }
}

impl fmt::Display for NotAsciiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "bytestring is not valid ASCII: {}",
            self.0.iter().join(",")
        )
    }
}

impl fmt::Display for MultiWidthsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "multiple measurement widths found when only one is needed: {}",
            self.0.iter().join(", ")
        )
    }
}

impl fmt::Display for WrongFloatWidth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "expected width to be {} but got {} when determining float type",
            self.expected, self.width,
        )
    }
}

impl fmt::Display for TotEventMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "$TOT field is {} but number of events that evenly fit into DATA is {}",
            self.tot, self.total_events,
        )
    }
}

impl fmt::Display for UnevenEventWidth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "Events are {} bytes wide, but this does not evenly \
             divide DATA segment which is {} bytes long \
             (remainder of {})",
            self.event_width, self.nbytes, self.remainder,
        )
    }
}

pub struct ConvertWidthError {
    index: MeasIndex,
    error: UintToUintError,
}

impl fmt::Display for ConvertWidthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "integer conversion error in column {}: {}",
            self.index, self.error,
        )
    }
}
