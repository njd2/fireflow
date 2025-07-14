//! Things pertaining to the DATA segment (mostly)
//!
//! Basic overview: DATA is arranged according to version-specific "layouts".
//! Each layout will enumerate all possible combinations for a given version,
//! which directly correspond to all valid combinations of $BYTEORD, $DATATYPE,
//! $PnB, $PnR, and $PnDATATYPE in the case of 3.2.
//!
//! Each layout may then be projected in a "reader" or "writer." Readers are
//! blank vectors waiting to accept data from disk. Writers are iterators that
//! read values from a dataframe and possibly convert them before writing.
//!
//! Now for the ugly bits.
//!
//! Layouts can first be classified by column width, where "fixed" layouts have
//! a single width per column and "delimited" layouts have a variable width. The
//! latter only corresponds to one layout: the case where $DATATYPE=A and all
//! $PnB=*. Values in such layouts will always be read as u64.
//!
//! Fixed layouts can further be classified by the type in each column:
//! 1) Single-type numeric layouts (aka "matrices")
//! 2) Fixed ASCII layouts
//! 3) Variable-width integer layouts
//! 4) Mixed layouts
//!
//! (1) is the simplest; each column is the same type which corresponds directly
//! with a native Rust type. This includes f32, f64, and uint ranging from 1 to
//! 8 bytes (including those that aren't powers of 2). Each type has a slightly
//! different reader/writer corresponding to distinct byte interpretations on
//! disk. (2) is similar in that the entire layout is one type; however, each
//! number is always read as u64 subject to the chars allowed by $PnB. (1)/(2)
//! are the only possibilities for FCS 2.0/3.0 since $BYTEORD restricts all $PnB
//! to the same width in the case of numeric $DATATYPE.
//!
//! (3) is a weird layout that almost nobody likely uses but is nonetheless
//! permitted starting with 3.1. Since $BYTEORD was changed to only mean
//! endian-ness, its relation to $PnB was severed. When DATATYPE=I, this means
//! $PnB may be changed freely, which allows different integer widths in each
//! column. In practice this makes the resulting data structure a dataframe (vs
//! a matrix).
//!
//! (4) was newly added to 3.2 by way of the PnDATATYPE keywords which now
//! allows the data layout to include any type. This obviously more complex but
//! is not computationally very different from (3).
//!
//! In addition to width, layouts may also be classified by whether $TOT is
//! known. In 2.0, $TOT is optional and may not be given. For delimited ASCII
//! layouts, not have $TOT means we need to parse until we reach the end of
//! DATA, hoping that all columns have the same length. For fixed layouts, we
//! can compute $TOT using $PnB and the length of DATA.

use crate::config::{ReaderConfig, StdTextReadConfig};
use crate::core::*;
use crate::error::*;
use crate::macros::match_many_to_one;
use crate::nonempty::NonEmptyExt;
use crate::segment::*;
use crate::text::byteord::*;
use crate::text::float_decimal::{FloatDecimal, FloatToDecimalError, HasFloatBounds};
use crate::text::index::{IndexFromOne, MeasIndex};
use crate::text::keywords::*;
use crate::text::optional::ClearOptional;
use crate::text::parser::*;
use crate::text::ranged_float::PositiveFloat;
use crate::text::scale::LogScale;
use crate::validated::ascii_range;
use crate::validated::ascii_range::{AsciiRange, Chars};
use crate::validated::bitmask::{Bitmask, BitmaskError};
use crate::validated::dataframe::*;
use crate::validated::keys::*;

use ambassador::{delegatable_trait, Delegate};
use bigdecimal::{BigDecimal, ParseBigDecimalError};
use derive_more::{Display, From};
use itertools::Itertools;
use nonempty::NonEmpty;
use num_traits::identities::One;
use num_traits::PrimInt;
use serde::ser::SerializeStruct;
use serde::Serialize;
use std::convert::Infallible;
use std::fmt;
use std::io;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
use std::mem;
use std::num::ParseIntError;
use std::str;

/// All possible byte layouts for the DATA segment in 2.0.
///
/// This is identical to 3.0 in every way except that the $TOT keyword in 2.0
/// is optional, which requires a different interface.
#[derive(Clone, Serialize, From, Delegate)]
#[delegate(LayoutOps<'a, T>, generics = "'a, T")]
#[delegate(InterLayoutOps<D>, generics = "D")]
#[delegate(OrderedLayoutOps)]
pub struct DataLayout2_0(pub AnyOrderedLayout<MaybeTot>);

/// All possible byte layouts for the DATA segment in 2.0.
#[derive(Clone, Serialize, From, Delegate)]
#[delegate(LayoutOps<'a, T>, generics = "'a, T")]
#[delegate(InterLayoutOps<D>, generics = "D")]
#[delegate(OrderedLayoutOps)]
pub struct DataLayout3_0(pub AnyOrderedLayout<KnownTot>);

/// All possible byte layouts for the DATA segment in 3.1.
///
/// Unlike 2.0 and 3.0, the integer layout allows the column widths to be
/// different. This is a consequence of making BYTEORD only mean "big or little
/// endian" and have nothing to do with number of bytes.
#[derive(Clone, Serialize, From, Delegate)]
#[delegate(LayoutOps<'a, T>, generics = "'a, T")]
#[delegate(InterLayoutOps<D>, generics = "D")]
#[delegate(EndianLayoutOps)]
pub struct DataLayout3_1(pub NonMixedEndianLayout<NoMeasDatatype>);

/// All possible byte layouts for the DATA segment in 3.2.
///
/// In addition to the loosened integer layouts in 3.1, 3.2 additionally allows
/// each column to have a different type and size (hence "Mixed").
#[derive(Clone, Serialize, From, Delegate)]
#[delegate(LayoutOps<'a, T>, generics = "'a, T")]
#[delegate(EndianLayoutOps)]
pub enum DataLayout3_2 {
    Mixed(EndianLayout<NullMixedType, HasMeasDatatype>),
    NonMixed(NonMixedEndianLayout<HasMeasDatatype>),
}

/// All possible byte layouts for the DATA segment in 2.0 and 3.0.
///
/// It is so named "Ordered" because the BYTEORD keyword represents any possible
/// byte ordering that may occur rather than simply little or big endian.
#[derive(Clone, Serialize, From, Delegate)]
#[delegate(LayoutOps<'a, Tot>, generics = "'a, Tot")]
#[delegate(InterLayoutOps<DT>, generics = "DT")]
#[delegate(OrderedLayoutOps)]
pub enum AnyOrderedLayout<T> {
    Ascii(AnyAsciiLayout<T, NoMeasDatatype, true>),
    Integer(AnyOrderedUintLayout<T>),
    F32(OrderedLayout<F32Type, T>),
    F64(OrderedLayout<F64Type, T>),
}

// TODO make an integer layout which has only one width, which will cover the
// vast majority of cases and make certain operations easier.
#[derive(Clone, Serialize, From, Delegate)]
#[delegate(LayoutOps<'a, Tot>, generics = "'a, Tot")]
#[delegate(InterLayoutOps<DT>, generics = "DT")]
#[delegate(EndianLayoutOps)]
pub enum NonMixedEndianLayout<D> {
    Ascii(AnyAsciiLayout<KnownTot, D, false>),
    Integer(EndianLayout<AnyNullBitmask, D>),
    F32(EndianLayout<F32Type, D>),
    F64(EndianLayout<F64Type, D>),
}

type EndianLayout<C, D> = FixedLayout<C, Endian, KnownTot, D>;

/// Byte layouts for ASCII data.
///
/// This may either be fixed (ie columns have the same number of characters)
/// or variable (ie columns have have different number of characters and are
/// separated by delimiters).
#[derive(Clone, Serialize, From, Delegate)]
#[delegate(LayoutOps<'a, Tot>, generics = "'a, Tot")]
#[delegate(InterLayoutOps<DT>, generics = "DT")]
pub enum AnyAsciiLayout<T, D, const ORD: bool> {
    Delimited(DelimAsciiLayout<T, D, ORD>),
    Fixed(FixedAsciiLayout<T, D, ORD>),
}

type FixedAsciiLayout<T, D, const ORD: bool> = FixedLayout<AsciiRange, NoByteOrd<ORD>, T, D>;

/// Byte layout for delimited ASCII.
#[derive(Clone)]
pub struct DelimAsciiLayout<T, D, const ORD: bool> {
    pub ranges: NonEmpty<u64>,
    _tot_def: PhantomData<T>,
    _meas_data_def: PhantomData<D>,
}

/// Byte layout where each column has a fixed width.
#[derive(Clone)]
pub struct FixedLayout<C, L, T, D> {
    byte_layout: L,
    columns: NonEmpty<C>,
    _tot_def: PhantomData<T>,
    _meas_data_def: PhantomData<D>,
}

/// Byte layout for integers that may be in any byte order.
#[derive(Clone, Serialize, From, Delegate)]
#[delegate(LayoutOps<'a, Tot>, generics = "'a, Tot")]
#[delegate(InterLayoutOps<DT>, generics = "DT")]
#[delegate(OrderedLayoutOps)]
pub enum AnyOrderedUintLayout<T> {
    // TODO the first two don't need to be ordered
    Uint08(OrderedLayout<UintType08, T>),
    Uint16(OrderedLayout<UintType16, T>),
    Uint24(OrderedLayout<UintType24, T>),
    Uint32(OrderedLayout<UintType32, T>),
    Uint40(OrderedLayout<UintType40, T>),
    Uint48(OrderedLayout<UintType48, T>),
    Uint56(OrderedLayout<UintType56, T>),
    Uint64(OrderedLayout<UintType64, T>),
}

type OrderedLayout<C, T> = FixedLayout<C, <C as HasNativeWidth>::Order, T, NoMeasDatatype>;

/// The type of a non-delimited column in the DATA segment for 3.2
pub enum MixedType<F: ColumnFamily> {
    Ascii(F::ColumnWrapper<AsciiRange, u64, NoByteOrd3_1>),
    Uint(AnyBitmask<F>),
    F32(NativeWrapper<F, F32Type>),
    F64(NativeWrapper<F, F64Type>),
}

pub type NullMixedType = MixedType<ColumnNullFamily>;
type ReaderMixedType = MixedType<ColumnReaderFamily>;
type WriterMixedType<'a> = MixedType<ColumnWriterFamily<'a>>;

/// A big or little-endian integer column of some size (1-8 bytes)
pub enum AnyBitmask<F: ColumnFamily> {
    Uint08(NativeWrapper<F, UintType08>),
    Uint16(NativeWrapper<F, UintType16>),
    Uint24(NativeWrapper<F, UintType24>),
    Uint32(NativeWrapper<F, UintType32>),
    Uint40(NativeWrapper<F, UintType40>),
    Uint48(NativeWrapper<F, UintType48>),
    Uint56(NativeWrapper<F, UintType56>),
    Uint64(NativeWrapper<F, UintType64>),
}

pub type AnyNullBitmask = AnyBitmask<ColumnNullFamily>;
type AnyReaderBitmask = AnyBitmask<ColumnReaderFamily>;
type AnyWriterBitmask<'a> = AnyBitmask<ColumnWriterFamily<'a>>;

/// The type of any floating point column in all versions
///
/// Scale is always linear, and gain is always 1.0/nil, so these values are
/// merely implied.
#[derive(PartialEq, Clone, Serialize)]
pub struct FloatType<T, const LEN: usize> {
    pub range: FloatDecimal<T>,
    _t: PhantomData<T>,
}

pub type F32Type = FloatType<f32, 4>;
pub type F64Type = FloatType<f64, 8>;

/// The type of any non-ASCII int column in all versions.
#[derive(Clone, Copy, Serialize, PartialEq)]
pub struct UintType<T, const LEN: usize> {
    /// The value of $PnR as a bitmask
    pub range: Bitmask<T, LEN>,

    /// The values of $PnE/$PnG which may imply a linear or log transform.
    pub xform: UintXform,
}

type UintType08 = UintType<u8, 1>;
type UintType16 = UintType<u16, 2>;
type UintType24 = UintType<u32, 3>;
type UintType32 = UintType<u32, 4>;
type UintType40 = UintType<u64, 5>;
type UintType48 = UintType<u64, 6>;
type UintType56 = UintType<u64, 7>;
type UintType64 = UintType<u64, 8>;

/// A scale transform that may be applied to an integer column.
#[derive(Clone, Copy, Serialize, PartialEq)]
pub enum UintXform {
    /// Linear transformation, given by $PnG!=1.0 and $PnE=0,0
    Lin(PositiveFloat),

    /// Log transformation, given by $PnG=1.0 or nil and $PnE!=0,0
    Log(LogScale),
}

/// Instructions to read one column and store in a vector
struct ColumnReader<C, T, S> {
    column_type: C,
    data: Vec<T>,
    byte_layout: PhantomData<S>,
}

type UintColumnReader<C> = ColumnReader<C, <C as HasNativeType>::Native, Endian>;

/// Instructions to write one column using an iterator
struct ColumnWriter<'a, C, T, S> {
    column_type: C,
    data: AnySource<'a, T>,
    byte_layout: PhantomData<S>,
}

type UintColumnWriter<'a, C> = ColumnWriter<'a, C, <C as HasNativeType>::Native, Endian>;

/// Marker type for columns which are used in a layout (non-reader/writer)
pub struct ColumnNullFamily;

/// Marker type for columns which are in a layout and have data for reading
struct ColumnReaderFamily;

/// Marker type for columns which are in a layout and have data for writing
struct ColumnWriterFamily<'a>(std::marker::PhantomData<&'a ()>);

/// Marker type for layouts that might have $TOT
#[derive(Clone, Serialize)]
pub struct MaybeTot;

/// Marker type for layouts that always have $TOT
#[derive(Clone, Serialize)]
pub struct KnownTot;

/// Marker type for layouts without $PnDATATYPE.
#[derive(Clone, Serialize)]
pub struct NoMeasDatatype;

/// Marker type for layouts with $PnDATATYPE.
#[derive(Clone, Serialize)]
pub struct HasMeasDatatype;

/// Marker type representing absence of column datatype.
pub struct NullMeasDatatype;

/// A struct whose fields map 1-1 with keyword values in one data column
pub struct ColumnLayoutValues<D> {
    width: Width,
    range: Range,
    datatype: D,
}

type ColumnLayoutValues2_0 = ColumnLayoutValues<NullMeasDatatype>;
type ColumnLayoutValues3_2 = ColumnLayoutValues<Option<NumType>>;

/// A type which represents a column which may have associated data.
///
/// Used to implement a higher-kinded type interface for columns that can be
/// by themselves or associated with reader or writer data.
pub trait ColumnFamily {
    type ColumnWrapper<C, T, S>;
}

type NativeWrapper<F, C> =
    <F as ColumnFamily>::ColumnWrapper<C, <C as HasNativeType>::Native, Endian>;

pub trait MeasDatatypeDef {
    type MeasDatatype;

    fn lookup_datatype(
        kws: &mut StdKeywords,
        i: MeasIndex,
    ) -> LookupTentative<Self::MeasDatatype, LookupKeysError>;

    fn lookup_datatype_ro(
        kws: &StdKeywords,
        i: MeasIndex,
    ) -> Tentative<Self::MeasDatatype, ParseKeyError<NumTypeError>, RawParsedError>;

    fn lookup_all(
        kws: &mut StdKeywords,
        par: Par,
    ) -> LookupResult<Vec<ColumnLayoutValues<Self::MeasDatatype>>> {
        (0..par.0)
            .map(|i| Self::lookup_one(kws, i.into()))
            .gather()
            .map(Tentative::mconcat)
            .map_err(DeferredFailure::mconcat)
    }

    fn lookup_ro_all(
        kws: &StdKeywords,
    ) -> DeferredResult<
        Vec<ColumnLayoutValues<Self::MeasDatatype>>,
        ParseKeyError<NumTypeError>,
        RawParsedError,
    > {
        Par::get_metaroot_req(kws)
            .into_deferred()
            .def_and_maybe(|par| {
                (0..par.0)
                    .map(|i| Self::lookup_one_ro(kws, i.into()))
                    .gather()
                    .map(Tentative::mconcat)
                    .map_err(DeferredFailure::mconcat)
            })
    }

    fn lookup_one(
        kws: &mut StdKeywords,
        i: MeasIndex,
    ) -> LookupResult<ColumnLayoutValues<Self::MeasDatatype>> {
        let j = i.into();
        let w = Width::lookup_req(kws, j);
        let r = Range::lookup_req(kws, j);
        w.def_zip(r).def_and_tentatively(|(width, range)| {
            Self::lookup_datatype(kws, i).map(|datatype| ColumnLayoutValues {
                width,
                range,
                datatype,
            })
        })
    }

    fn lookup_one_ro(
        kws: &StdKeywords,
        i: MeasIndex,
    ) -> DeferredResult<
        ColumnLayoutValues<Self::MeasDatatype>,
        ParseKeyError<NumTypeError>,
        RawParsedError,
    > {
        let j = i.into();
        let w = Width::get_meas_req(kws, j).map_err(|e| e.into());
        let r = Range::get_meas_req(kws, j).map_err(|e| e.into());
        w.zip(r)
            .map(Tentative::new1)
            .map_err(DeferredFailure::new2)
            .def_and_tentatively(|(width, range)| {
                Self::lookup_datatype_ro(kws, i).map(|datatype| ColumnLayoutValues {
                    width,
                    range,
                    datatype,
                })
            })
    }
}

/// Methods for a type which may or may not have $TOT
pub trait TotDefinition {
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

/// Standardized operations on layouts
#[delegatable_trait]
pub trait LayoutOps<'a, T>: Sized {
    fn ncols(&self) -> usize;

    fn nbytes(&self, df: &FCSDataFrame) -> u64;

    fn widths(&self) -> Vec<BitsOrChars>;

    fn ranges(&self) -> NonEmpty<Range>;

    fn datatype(&self) -> AlphaNumType;

    fn byteord_keyword(&self) -> (String, String);

    fn req_keywords(&self) -> [(String, String); 2] {
        [self.datatype().pair(), self.byteord_keyword()]
    }

    fn req_meas_keywords(&self) -> NonEmpty<[(String, String); 2]>;

    fn remove_nocheck(&mut self, index: MeasIndex) -> Result<(), ClearOptional>;

    fn h_read_df_inner<R: Read>(
        &self,
        h: &mut BufReader<R>,
        buf: &mut Vec<u8>,
        tot: <T as TotDefinition>::Tot,
        seg: AnyDataSegment,
        conf: &ReaderConfig,
    ) -> IODeferredResult<FCSDataFrame, ReadDataframeWarning, ReadDataframeError>
    where
        T: TotDefinition;

    fn check_writer(&self, df: &'a FCSDataFrame) -> MultiResult<(), ColumnError<AnyLossError>>;

    fn h_write_df_inner<W: Write>(
        &self,
        h: &mut BufWriter<W>,
        df: &'a FCSDataFrame,
    ) -> io::Result<()>;
}

#[delegatable_trait]
pub trait InterLayoutOps<D> {
    fn opt_meas_headers(&self) -> Vec<MeasHeader>;

    fn opt_meas_keywords(&self) -> NonEmpty<Vec<(String, Option<String>)>>;

    // no need to check since this will be done after validating that the index
    // is within the measurement vector, which has its own check and should
    // always be the same length
    fn insert_nocheck(
        &mut self,
        index: MeasIndex,
        range: Range,
        notrunc: bool,
    ) -> BiTentative<(), AnyRangeError>;

    fn push(&mut self, range: Range, notrunc: bool) -> BiTentative<(), AnyRangeError>;
}

/// Standardized operations on layouts
#[delegatable_trait]
pub trait OrderedLayoutOps: Sized {
    fn byte_order(&self) -> Option<ByteOrd2_0>;

    fn endianness(&self) -> Option<Endian> {
        self.byte_order().and_then(|x| x.try_into().ok())
    }
}

/// Standardized operations on layouts
#[delegatable_trait]
pub trait EndianLayoutOps: Sized {
    fn endianness(&self) -> Option<Endian>;
}

/// A version-specific data layout
pub trait VersionedDataLayout
where
    for<'a> Self: Sized + LayoutOps<'a, Self::TotDef> + InterLayoutOps<Self::MeasDTDef>,
{
    type ByteLayout;
    type MeasDTDef: MeasDatatypeDef;
    type TotDef: TotDefinition;

    fn lookup(
        kws: &mut StdKeywords,
        conf: &StdTextReadConfig,
        par: Par,
    ) -> LookupLayoutResult<Option<Self>>;

    fn lookup_ro(kws: &StdKeywords, conf: &StdTextReadConfig) -> FromRawResult<Option<Self>>;

    fn try_new(
        dt: AlphaNumType,
        size: Self::ByteLayout,
        columns: NonEmpty<ColumnLayoutValues<<Self::MeasDTDef as MeasDatatypeDef>::MeasDatatype>>,
        conf: &StdTextReadConfig,
    ) -> DeferredResult<Self, ColumnError<NewMixedTypeWarning>, NewDataLayoutError>;

    fn h_read_df<R: Read + Seek>(
        &self,
        h: &mut BufReader<R>,
        tot: <Self::TotDef as TotDefinition>::Tot,
        seg: AnyDataSegment,
        conf: &ReaderConfig,
    ) -> IODeferredResult<FCSDataFrame, ReadDataframeWarning, ReadDataframeError> {
        // The only purpose of this buffer is to read ASCII since we don't
        // hardcode the buffer width into the type (unlike integers and floats).
        // It's passed down to each layer of the read stack to avoid making the
        // buffer argument generic, which would make this implementation much
        // more complex. Good enough to pass the buffer and only use it when
        // needed.
        let mut buf = vec![];
        seg.inner.as_u64().try_coords().map_or(
            Ok(Tentative::new1(FCSDataFrame::default())),
            |(begin, _)| {
                h.seek(SeekFrom::Start(begin)).into_deferred()?;
                self.h_read_df_inner(h, &mut buf, tot, seg, conf)
            },
        )
    }

    fn h_write_df<W>(&self, h: &mut BufWriter<W>, df: &FCSDataFrame) -> io::Result<()>
    where
        W: Write,
    {
        // The dataframe should be encapsulated such that a) the column number
        // matches the number of measurements. If these are not true, the code
        // is wrong.
        let par = self.ncols();
        let ncols = df.ncols();
        if ncols != par {
            panic!("datafame columns ({ncols}) unequal to number of measurements ({par})");
        }
        self.h_write_df_inner(h, df)
    }
}

pub trait HasNativeType: Sized {
    /// The native rust type
    type Native: Default + Copy;
}

/// A type which uses a defined number of bytes
pub trait HasNativeWidth: HasNativeType {
    /// The length of the type in an FCS file (may be less than native)
    const BYTES: Bytes;

    /// The length of the native Rust type
    const LEN: usize;

    /// The sized byte order to be used with this type
    type Order;
}

/// A column which has a $DATATYPE keyword
pub trait HasDatatype: Sized {
    fn datatype_from_columns(cs: &NonEmpty<Self>) -> AlphaNumType;
}

trait FromRange: Sized {
    type Error;

    fn from_range(range: Range, notrunc: bool) -> BiTentative<Self, Self::Error>;
}

/// A type which has a width that may vary
pub trait IsFixed {
    fn nbytes(&self) -> u8;

    fn fixed_width(&self) -> BitsOrChars;

    fn range(&self) -> Range;

    fn req_meas_keywords(&self, i: MeasIndex) -> [(String, String); 2] {
        [
            Width::Fixed(self.fixed_width()).pair(i.into()),
            self.range().pair(i.into()),
        ]
    }
}

/// A column which may be transformed into a reader for a rust numeric type
trait ToNativeReader: HasNativeType {
    fn into_native_reader<S>(self, nrows: usize) -> ColumnReader<Self, Self::Native, S>
    where
        Self::Native: Default + Copy,
    {
        ColumnReader {
            column_type: self,
            data: vec![Self::Native::default(); nrows],
            byte_layout: PhantomData,
        }
    }
}

// TODO can't this just be with the native reader type?
trait NativeReadable<S>: HasNativeType {
    fn h_read_native<R: Read>(
        &self,
        h: &mut BufReader<R>,
        byte_layout: S,
        buf: &mut Vec<u8>,
    ) -> IOResult<Self::Native, ReadDataframeError>;
}

/// A column which may be transformed into a writer for a rust numeric type
trait ToNativeWriter
where
    Self: HasNativeType,
{
    type Error;

    fn into_native_writer<'a, S>(
        self,
        c: &'a AnyFCSColumn,
    ) -> ColumnWriter<'a, Self, Self::Native, S>
    where
        Self::Native: Default + Copy + AllFCSCast,
        AnySource<'a, Self::Native>: From<FCSColIter<'a, u8, Self::Native>>
            + From<FCSColIter<'a, u16, Self::Native>>
            + From<FCSColIter<'a, u32, Self::Native>>
            + From<FCSColIter<'a, u64, Self::Native>>
            + From<FCSColIter<'a, f32, Self::Native>>
            + From<FCSColIter<'a, f64, Self::Native>>,
    {
        ColumnWriter {
            column_type: self,
            data: AnySource::new(c),
            byte_layout: PhantomData,
        }
    }

    fn check_native_writer(&self, col: &AnyFCSColumn) -> Result<(), LossError<Self::Error>>
    where
        Self::Native: Default + Copy + AllFCSCast,
    {
        col.check_writer(|x| Self::check_other_loss(self, x))
    }

    fn check_other_loss(&self, x: Self::Native) -> Option<Self::Error>;
}

trait IntoReader<S> {
    type Target: Readable<S>;

    fn into_reader(self, nrows: usize) -> Self::Target;
}

trait Readable<S> {
    fn into_dataframe_column(self) -> AnyFCSColumn;

    fn h_read<R: Read>(
        &mut self,
        h: &mut BufReader<R>,
        row: usize,
        byte_layout: S,
        buf: &mut Vec<u8>,
    ) -> IOResult<(), ReadDataframeError>;
}

trait NativeWritable<S>: HasNativeType {
    fn h_write<W: Write>(
        &self,
        h: &mut BufWriter<W>,
        x: CastResult<Self::Native>,
        byte_layout: S,
    ) -> io::Result<()>;
}

trait IntoWriter<'a, S> {
    type Target: Writable<'a, S>;

    fn into_writer(self, col: &'a AnyFCSColumn) -> Self::Target;

    fn check_writer(&self, col: &'a AnyFCSColumn) -> Result<(), AnyLossError>;
}

trait Writable<'a, S> {
    fn h_write<W: Write>(&mut self, h: &mut BufWriter<W>, byte_layout: S) -> io::Result<()>;
}

/// General methods for each numeric type.
///
/// This is mostly for converting to/from bytes with various endian-ness.
// TODO clean this up with https://github.com/rust-lang/rust/issues/76560 once
// it lands in a stable compiler, in theory there is no reason to put the length
// of the type as a parameter, but the current compiler is not smart enough
trait NumProps: Sized + Copy + Default {
    const LEN: usize;
    type BUF: AsRef<[u8]> + AsMut<[u8]> + Default;

    fn read_buf<R: Read>(h: &mut BufReader<R>) -> io::Result<Self::BUF>;

    fn from_big(buf: Self::BUF) -> Self;

    fn from_little(buf: Self::BUF) -> Self;

    fn from_endian(buf: Self::BUF, endian: Endian) -> Self {
        match endian {
            Endian::Big => Self::from_big(buf),
            Endian::Little => Self::from_little(buf),
        }
    }

    fn to_big(self) -> Self::BUF;

    fn to_little(self) -> Self::BUF;

    fn to_endian(self, endian: Endian) -> Self::BUF {
        match endian {
            Endian::Big => self.to_big(),
            Endian::Little => self.to_little(),
        }
    }
}

/// Methods for reading numbers which may be in arbitrary byte orders.
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

/// Methods for reading/writing integers (1-8 bytes) from FCS files.
trait IntFromBytes<const INTLEN: usize>: NumProps + OrderedFromBytes<INTLEN> {
    fn h_read_endian<R: Read>(h: &mut BufReader<R>, endian: Endian) -> io::Result<Self> {
        // Read data that is not a power-of-two bytes long. Start by reading n
        // bytes into a vector, which can take a varying size. Then copy this
        // into the power of 2 buffer which will go to one or the other end of
        // the buffer depending on endianness.
        let mut tmp = [0; INTLEN];
        let mut buf = Self::BUF::default();
        h.read_exact(&mut tmp)?;
        Ok(if endian == Endian::Big {
            let b = Self::LEN - INTLEN;
            buf.as_mut()[b..].copy_from_slice(&tmp);
            Self::from_big(buf)
        } else {
            buf.as_mut()[..INTLEN].copy_from_slice(&tmp);
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

/// Methods for reading/writing floats (32 and 64 bit) from FCS files.
trait FloatFromBytes<const LEN: usize>: NumProps + OrderedFromBytes<LEN> {
    fn h_read_endian<R: Read>(h: &mut BufReader<R>, endian: Endian) -> io::Result<Self> {
        let buf = Self::read_buf(h)?;
        Ok(Self::from_endian(buf, endian))
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
        let buf = Self::to_endian(self, endian);
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

macro_rules! impl_null_layout {
    ($t:ident, $($var:ident),*) => {
        // impl Copy for $t {}

        impl Clone for $t {
            fn clone(&self) -> Self {
                match self {
                    $(
                        Self::$var(x) => $t::$var(x.clone()),
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
    AnyNullBitmask,
    Uint08,
    Uint16,
    Uint24,
    Uint32,
    Uint40,
    Uint48,
    Uint56,
    Uint64
);

macro_rules! impl_any_uint {
    ($var:ident, $bitmask:path) => {
        impl From<$bitmask> for AnyNullBitmask {
            fn from(value: $bitmask) -> Self {
                Self::$var(value)
            }
        }

        impl From<UintColumnReader<$bitmask>> for AnyReaderBitmask {
            fn from(value: UintColumnReader<$bitmask>) -> Self {
                Self::$var(value)
            }
        }

        impl<'a> From<UintColumnWriter<'a, $bitmask>> for AnyWriterBitmask<'a> {
            fn from(value: UintColumnWriter<'a, $bitmask>) -> Self {
                Self::$var(value)
            }
        }

        impl TryFrom<AnyNullBitmask> for $bitmask {
            type Error = UintToUintError;
            fn try_from(value: AnyNullBitmask) -> Result<Self, Self::Error> {
                let w = value.nbytes();
                if let AnyBitmask::$var(x) = value {
                    Ok(x)
                } else {
                    Err(UintToUintError {
                        from: w,
                        to: Self::BYTES.into(),
                    })
                }
            }
        }

        impl TryFrom<NullMixedType> for $bitmask {
            type Error = MixedToOrderedUintError;
            fn try_from(value: NullMixedType) -> Result<Self, Self::Error> {
                let w = value.nbytes();
                if let MixedType::Uint(x) = value {
                    if let AnyBitmask::$var(y) = x {
                        Ok(y)
                    } else {
                        Err(UintToUintError {
                            from: w,
                            to: Self::BYTES.into(),
                        }
                        .into())
                    }
                } else {
                    let dest_type = value.as_alpha_num_type();
                    Err(MixedToInnerError {
                        dest_type,
                        src: value,
                    }
                    .into())
                }
            }
        }

        impl From<$bitmask> for NullMixedType {
            fn from(value: $bitmask) -> Self {
                MixedType::Uint(value.into())
            }
        }
    };
}

impl_any_uint!(Uint08, UintType08);
impl_any_uint!(Uint16, UintType16);
impl_any_uint!(Uint24, UintType24);
impl_any_uint!(Uint32, UintType32);
impl_any_uint!(Uint40, UintType40);
impl_any_uint!(Uint48, UintType48);
impl_any_uint!(Uint56, UintType56);
impl_any_uint!(Uint64, UintType64);

impl From<AsciiRange> for NullMixedType {
    fn from(value: AsciiRange) -> Self {
        Self::Ascii(value)
    }
}

impl From<AnyNullBitmask> for NullMixedType {
    fn from(value: AnyNullBitmask) -> Self {
        Self::Uint(value)
    }
}

impl From<F32Type> for NullMixedType {
    fn from(value: F32Type) -> Self {
        Self::F32(value)
    }
}

impl From<F64Type> for NullMixedType {
    fn from(value: F64Type) -> Self {
        Self::F64(value)
    }
}

impl From<ColumnReader<AsciiRange, u64, NoByteOrd<false>>> for ReaderMixedType {
    fn from(value: ColumnReader<AsciiRange, u64, NoByteOrd<false>>) -> Self {
        Self::Ascii(value)
    }
}

impl From<AnyReaderBitmask> for ReaderMixedType {
    fn from(value: AnyReaderBitmask) -> Self {
        Self::Uint(value)
    }
}

impl From<ColumnReader<F32Type, f32, Endian>> for ReaderMixedType {
    fn from(value: ColumnReader<F32Type, f32, Endian>) -> Self {
        Self::F32(value)
    }
}

impl From<ColumnReader<F64Type, f64, Endian>> for ReaderMixedType {
    fn from(value: ColumnReader<F64Type, f64, Endian>) -> Self {
        Self::F64(value)
    }
}

impl<'a> From<ColumnWriter<'a, AsciiRange, u64, NoByteOrd<false>>> for WriterMixedType<'a> {
    fn from(value: ColumnWriter<'a, AsciiRange, u64, NoByteOrd<false>>) -> Self {
        Self::Ascii(value)
    }
}

impl<'a> From<AnyWriterBitmask<'a>> for WriterMixedType<'a> {
    fn from(value: AnyWriterBitmask<'a>) -> Self {
        Self::Uint(value)
    }
}

impl<'a> From<ColumnWriter<'a, F32Type, f32, Endian>> for WriterMixedType<'a> {
    fn from(value: ColumnWriter<'a, F32Type, f32, Endian>) -> Self {
        Self::F32(value)
    }
}

impl<'a> From<ColumnWriter<'a, F64Type, f64, Endian>> for WriterMixedType<'a> {
    fn from(value: ColumnWriter<'a, F64Type, f64, Endian>) -> Self {
        Self::F64(value)
    }
}

impl MeasDatatypeDef for NoMeasDatatype {
    type MeasDatatype = NullMeasDatatype;

    fn lookup_datatype(
        _: &mut StdKeywords,
        _: MeasIndex,
    ) -> LookupTentative<Self::MeasDatatype, LookupKeysError> {
        Tentative::new1(NullMeasDatatype)
    }

    fn lookup_datatype_ro(
        _: &StdKeywords,
        _: MeasIndex,
    ) -> Tentative<Self::MeasDatatype, ParseKeyError<NumTypeError>, RawParsedError> {
        Tentative::new1(NullMeasDatatype)
    }
}

impl MeasDatatypeDef for HasMeasDatatype {
    type MeasDatatype = Option<NumType>;

    fn lookup_datatype(
        kws: &mut StdKeywords,
        i: MeasIndex,
    ) -> LookupTentative<Self::MeasDatatype, LookupKeysError> {
        NumType::lookup_opt(kws, i.into()).map(|x| x.0)
    }

    fn lookup_datatype_ro(
        kws: &StdKeywords,
        i: MeasIndex,
    ) -> Tentative<Self::MeasDatatype, ParseKeyError<NumTypeError>, RawParsedError> {
        NumType::get_meas_opt(kws, i.into())
            .map(|x| x.0)
            .map_or_else(|e| Tentative::new(None, vec![e], vec![]), Tentative::new1)
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

impl ColumnFamily for ColumnNullFamily {
    type ColumnWrapper<C, T, S> = C;
}

impl ColumnFamily for ColumnReaderFamily {
    type ColumnWrapper<C, T, S> = ColumnReader<C, T, S>;
}

impl<'a> ColumnFamily for ColumnWriterFamily<'a> {
    type ColumnWrapper<C, T, S> = ColumnWriter<'a, C, T, S>;
}

macro_rules! match_any_uint {
    ($value:expr, $root:ident, $inner:ident, $action:block) => {
        match_many_to_one!(
            $value,
            $root,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            $inner,
            $action
        )
    };
}

macro_rules! match_any_mixed {
    ($value:expr, $inner:ident, $action:block) => {
        match_many_to_one!($value, MixedType, [Ascii, Uint, F32, F64], $inner, $action)
    };
}

impl<T, const LEN: usize> From<&UintType<T, LEN>> for Range
where
    Bitmask<T, LEN>: Copy,
    Range: From<Bitmask<T, LEN>>,
{
    fn from(value: &UintType<T, LEN>) -> Self {
        Range::from(value.range)
    }
}

impl From<&NullMixedType> for Range {
    fn from(value: &NullMixedType) -> Self {
        match_any_mixed!(value, x, { x.into() })
    }
}

impl From<&AnyNullBitmask> for Range {
    fn from(value: &AnyNullBitmask) -> Self {
        match_any_uint!(value, AnyNullBitmask, x, { x.into() })
    }
}

impl<T: Clone, const LEN: usize> From<&FloatType<T, LEN>> for Range {
    fn from(value: &FloatType<T, LEN>) -> Self {
        value.range.clone().into()
    }
}

macro_rules! mixed_to_inner {
    ($inner:ident, $var:ident) => {
        impl TryFrom<NullMixedType> for $inner {
            type Error = MixedToInnerError;
            fn try_from(value: NullMixedType) -> Result<Self, Self::Error> {
                let dest_type = value.as_alpha_num_type();
                if let MixedType::$var(x) = value {
                    Ok(x)
                } else {
                    Err(MixedToInnerError {
                        dest_type,
                        src: value,
                    })
                }
            }
        }
    };
}

mixed_to_inner!(AsciiRange, Ascii);
mixed_to_inner!(AnyNullBitmask, Uint);
mixed_to_inner!(F32Type, F32);
mixed_to_inner!(F64Type, F64);

impl<T, const LEN: usize> ToNativeReader for UintType<T, LEN> where Self: HasNativeType<Native = T> {}

impl<T, const LEN: usize> ToNativeReader for FloatType<T, LEN> where Self: HasNativeType<Native = T> {}

impl ToNativeReader for AsciiRange {}

impl<T, const LEN: usize> NativeReadable<Endian> for UintType<T, LEN>
where
    Self: HasNativeType<Native = T>,
    T: Ord + Copy + IntFromBytes<LEN>,
{
    fn h_read_native<R: Read>(
        &self,
        h: &mut BufReader<R>,
        byte_layout: Endian,
        _: &mut Vec<u8>,
    ) -> IOResult<T, ReadDataframeError> {
        let x = T::h_read_endian(h, byte_layout)?;
        Ok(x)
    }
}

impl<T, const LEN: usize> NativeReadable<SizedByteOrd<LEN>> for UintType<T, LEN>
where
    Self: HasNativeType<Native = T>,
    T: Ord + Copy + IntFromBytes<LEN>,
{
    fn h_read_native<R: Read>(
        &self,
        h: &mut BufReader<R>,
        byte_layout: SizedByteOrd<LEN>,
        _: &mut Vec<u8>,
    ) -> IOResult<T, ReadDataframeError> {
        let x = T::h_read_ordered(h, byte_layout)?;
        Ok(x)
    }
}

impl<T, const LEN: usize> NativeReadable<Endian> for FloatType<T, LEN>
where
    FloatType<T, LEN>: HasNativeType<Native = T>,
    T: Copy + FloatFromBytes<LEN>,
{
    fn h_read_native<R: Read>(
        &self,
        h: &mut BufReader<R>,
        byte_layout: Endian,
        _: &mut Vec<u8>,
    ) -> IOResult<T, ReadDataframeError> {
        let x = T::h_read_endian(h, byte_layout)?;
        Ok(x)
    }
}

impl<T, const LEN: usize> NativeReadable<SizedByteOrd<LEN>> for FloatType<T, LEN>
where
    FloatType<T, LEN>: HasNativeType<Native = T>,
    T: Copy + FloatFromBytes<LEN>,
{
    fn h_read_native<R: Read>(
        &self,
        h: &mut BufReader<R>,
        byte_layout: SizedByteOrd<LEN>,
        _: &mut Vec<u8>,
    ) -> IOResult<T, ReadDataframeError> {
        let x = T::h_read_ordered(h, byte_layout)?;
        Ok(x)
    }
}

impl<const ORD: bool> NativeReadable<NoByteOrd<ORD>> for AsciiRange {
    fn h_read_native<R: Read>(
        &self,
        h: &mut BufReader<R>,
        _: NoByteOrd<ORD>,
        buf: &mut Vec<u8>,
    ) -> IOResult<Self::Native, ReadDataframeError> {
        buf.clear();
        h.take(u8::from(self.chars()).into()).read_to_end(buf)?;
        ascii_to_uint(buf).map_err(|e| ImpureError::Pure(e.into()))
    }
}

impl<C, S> IntoReader<S> for C
where
    AnyFCSColumn: From<FCSColumn<C::Native>>,
    C: NativeReadable<S> + ToNativeReader,
{
    type Target = ColumnReader<C, C::Native, S>;

    fn into_reader(self, nrows: usize) -> Self::Target {
        self.into_native_reader(nrows)
    }
}

impl IntoReader<Endian> for AnyNullBitmask {
    type Target = AnyReaderBitmask;

    fn into_reader(self, nrows: usize) -> Self::Target {
        match_any_uint!(self, Self, c, { c.into_native_reader(nrows).into() })
    }
}

impl IntoReader<Endian> for NullMixedType {
    type Target = ReaderMixedType;

    fn into_reader(self, nrows: usize) -> Self::Target {
        match_any_mixed!(self, c, { c.into_reader(nrows).into() })
    }
}

impl<C, T, S> Readable<S> for ColumnReader<C, T, S>
where
    T: Copy + Default,
    C: NativeReadable<S> + HasNativeType<Native = T> + ToNativeReader,
    AnyFCSColumn: From<FCSColumn<T>>,
{
    fn into_dataframe_column(self) -> AnyFCSColumn {
        FCSColumn::from(self.data).into()
    }

    fn h_read<R: Read>(
        &mut self,
        h: &mut BufReader<R>,
        row: usize,
        byte_layout: S,
        buf: &mut Vec<u8>,
    ) -> IOResult<(), ReadDataframeError> {
        self.data[row] = self.column_type.h_read_native(h, byte_layout, buf)?;
        Ok(())
    }
}

impl Readable<Endian> for ReaderMixedType {
    fn into_dataframe_column(self) -> AnyFCSColumn {
        match_any_mixed!(self, c, { c.into_dataframe_column() })
    }

    fn h_read<R: Read>(
        &mut self,
        h: &mut BufReader<R>,
        row: usize,
        byte_layout: Endian,
        buf: &mut Vec<u8>,
    ) -> IOResult<(), ReadDataframeError> {
        match self {
            MixedType::Ascii(c) => c.h_read(h, row, NoByteOrd, buf),
            MixedType::Uint(c) => c.h_read(h, row, byte_layout, buf),
            MixedType::F32(c) => c.h_read(h, row, byte_layout, buf),
            MixedType::F64(c) => c.h_read(h, row, byte_layout, buf),
        }
    }
}

impl Readable<Endian> for AnyReaderBitmask {
    fn into_dataframe_column(self) -> AnyFCSColumn {
        match_any_uint!(self, AnyBitmask, c, { c.into_dataframe_column() })
    }

    fn h_read<R: Read>(
        &mut self,
        h: &mut BufReader<R>,
        row: usize,
        byte_layout: Endian,
        buf: &mut Vec<u8>,
    ) -> IOResult<(), ReadDataframeError> {
        match_any_uint!(self, AnyBitmask, c, { c.h_read(h, row, byte_layout, buf) })
    }
}

impl<T, const LEN: usize> NativeWritable<Endian> for UintType<T, LEN>
where
    Self: HasNativeType<Native = T>,
    T: Ord + Copy + IntFromBytes<LEN>,
{
    fn h_write<W: Write>(
        &self,
        h: &mut BufWriter<W>,
        x: CastResult<T>,
        byte_layout: Endian,
    ) -> io::Result<()> {
        self.range.apply(x.new).h_write_endian(h, byte_layout)
    }
}

impl<T, const LEN: usize> NativeWritable<SizedByteOrd<LEN>> for UintType<T, LEN>
where
    Self: HasNativeType<Native = T>,
    T: Ord + Copy + IntFromBytes<LEN>,
{
    fn h_write<W: Write>(
        &self,
        h: &mut BufWriter<W>,
        x: CastResult<T>,
        byte_layout: SizedByteOrd<LEN>,
    ) -> io::Result<()> {
        self.range.apply(x.new).h_write_ordered(h, byte_layout)
    }
}

impl<T, const LEN: usize> NativeWritable<Endian> for FloatType<T, LEN>
where
    FloatType<T, LEN>: HasNativeType<Native = T>,
    T: Copy + FloatFromBytes<LEN>,
{
    fn h_write<W: Write>(
        &self,
        h: &mut BufWriter<W>,
        x: CastResult<T>,
        byte_layout: Endian,
    ) -> io::Result<()> {
        x.new.h_write_endian(h, byte_layout)
    }
}

impl<T, const LEN: usize> NativeWritable<SizedByteOrd<LEN>> for FloatType<T, LEN>
where
    FloatType<T, LEN>: HasNativeType<Native = T>,
    T: Copy + FloatFromBytes<LEN>,
{
    fn h_write<W: Write>(
        &self,
        h: &mut BufWriter<W>,
        x: CastResult<T>,
        byte_layout: SizedByteOrd<LEN>,
    ) -> io::Result<()> {
        x.new.h_write_ordered(h, byte_layout)
    }
}

impl<const ORD: bool> NativeWritable<NoByteOrd<ORD>> for AsciiRange {
    fn h_write<W: Write>(
        &self,
        h: &mut BufWriter<W>,
        x: CastResult<Self::Native>,
        _: NoByteOrd<ORD>,
    ) -> io::Result<()> {
        let s = x.new.to_string();
        let w: usize = u8::from(self.chars()).into();
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

impl<'a, C, S> IntoWriter<'a, S> for C
where
    C: ToNativeWriter,
    ColumnWriter<'a, C, C::Native, S>: Writable<'a, S>,
    C::Native: Default + Copy + AllFCSCast,
    AnySource<'a, C::Native>: From<FCSColIter<'a, u8, C::Native>>
        + From<FCSColIter<'a, u16, C::Native>>
        + From<FCSColIter<'a, u32, C::Native>>
        + From<FCSColIter<'a, u64, C::Native>>
        + From<FCSColIter<'a, f32, C::Native>>
        + From<FCSColIter<'a, f64, C::Native>>,
    AnyLossError: From<LossError<C::Error>>,
{
    type Target = ColumnWriter<'a, C, C::Native, S>;

    fn into_writer(self, col: &'a AnyFCSColumn) -> Self::Target {
        self.into_native_writer(col)
    }

    fn check_writer(&self, col: &'a AnyFCSColumn) -> Result<(), AnyLossError> {
        self.check_native_writer(col).map_err(|e| e.into())
    }
}

impl<'a> IntoWriter<'a, Endian> for AnyNullBitmask {
    type Target = AnyWriterBitmask<'a>;

    fn into_writer(self, col: &'a AnyFCSColumn) -> Self::Target {
        match_any_uint!(self, Self, c, { c.into_native_writer(col).into() })
    }

    fn check_writer(&self, col: &'a AnyFCSColumn) -> Result<(), AnyLossError> {
        match_any_uint!(self, Self, c, {
            c.check_native_writer(col).map_err(|e| e.into())
        })
    }
}

impl<'a> IntoWriter<'a, Endian> for NullMixedType {
    type Target = WriterMixedType<'a>;

    fn into_writer(self, col: &'a AnyFCSColumn) -> Self::Target {
        match_any_mixed!(self, c, { c.into_writer(col).into() })
    }

    fn check_writer(&self, col: &'a AnyFCSColumn) -> Result<(), AnyLossError> {
        match self {
            MixedType::Ascii(c) => IntoWriter::<NoByteOrd3_1>::check_writer(c, col),
            MixedType::Uint(c) => c.check_writer(col),
            MixedType::F32(c) => c.check_native_writer(col).map_err(|e| e.into()),
            MixedType::F64(c) => c.check_native_writer(col).map_err(|e| e.into()),
        }
    }
}

impl<'a, C, T, S> Writable<'a, S> for ColumnWriter<'a, C, T, S>
where
    C: NativeWritable<S> + HasNativeType<Native = T> + ToNativeWriter,
{
    fn h_write<W: Write>(&mut self, h: &mut BufWriter<W>, byte_layout: S) -> io::Result<()> {
        let x = self.data.next().unwrap();
        self.column_type.h_write(h, x, byte_layout)
    }
}

impl<'a> Writable<'a, Endian> for WriterMixedType<'a> {
    fn h_write<W: Write>(&mut self, h: &mut BufWriter<W>, byte_layout: Endian) -> io::Result<()> {
        match self {
            Self::Ascii(c) => c.h_write(h, NoByteOrd),
            Self::Uint(c) => c.h_write(h, byte_layout),
            Self::F32(c) => c.h_write(h, byte_layout),
            Self::F64(c) => c.h_write(h, byte_layout),
        }
    }
}

impl<'a> Writable<'a, Endian> for AnyWriterBitmask<'a> {
    fn h_write<W: Write>(&mut self, h: &mut BufWriter<W>, byte_layout: Endian) -> io::Result<()> {
        match_any_uint!(self, Self, c, { c.h_write(h, byte_layout) })
    }
}

impl<T, const LEN: usize> ToNativeWriter for UintType<T, LEN>
where
    Self: HasNativeType<Native = T>,
    u64: From<T>,
    T: Ord + Copy,
{
    type Error = BitmaskLossError;

    fn check_other_loss(&self, x: T) -> Option<Self::Error> {
        let m = self.range.bitmask();
        if x > m {
            Some(BitmaskLossError(u64::from(m)))
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

    fn check_other_loss(&self, _: T) -> Option<Self::Error> {
        None
    }
}

impl ToNativeWriter for AsciiRange {
    type Error = AsciiLossError;

    fn check_other_loss(&self, x: Self::Native) -> Option<Self::Error>
    where
        u64: From<Self::Native>,
    {
        if Chars::from_u64(x) > self.chars() {
            Some(AsciiLossError(self.chars()))
        } else {
            None
        }
    }
}

/// A wrapper for any of the 6 source types that can be written.
///
/// Each inner type is an iterator from a different source type which emit
/// the given target type.
enum AnySource<'a, TargetType> {
    FromU08(FCSColIter<'a, u8, TargetType>),
    FromU16(FCSColIter<'a, u16, TargetType>),
    FromU32(FCSColIter<'a, u32, TargetType>),
    FromU64(FCSColIter<'a, u64, TargetType>),
    FromF32(FCSColIter<'a, f32, TargetType>),
    FromF64(FCSColIter<'a, f64, TargetType>),
}

impl<'a, T> AnySource<'a, T> {
    fn new(c: &'a AnyFCSColumn) -> Self
    where
        T: AllFCSCast,
        Self: From<FCSColIter<'a, u8, T>>
            + From<FCSColIter<'a, u16, T>>
            + From<FCSColIter<'a, u32, T>>
            + From<FCSColIter<'a, u64, T>>
            + From<FCSColIter<'a, f32, T>>
            + From<FCSColIter<'a, f64, T>>,
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

fn is_ascii_delim(x: u8) -> bool {
    // tab, newline, carriage return, space, or comma
    x == 9 || x == 10 || x == 13 || x == 32 || x == 44
}

impl<C: Serialize, L: Serialize, T, D> Serialize for FixedLayout<C, L, T, D> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("FixedLayout", 2)?;
        state.serialize_field("columns", Vec::from(self.columns.as_ref()).as_slice())?;
        state.serialize_field("byte_layout", &self.byte_layout)?;
        state.end()
    }
}

impl<T, D, const ORD: bool> Serialize for DelimAsciiLayout<T, D, ORD> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("DelimitedLayout", 1)?;
        state.serialize_field("ranges", Vec::from(self.ranges.as_ref()).as_slice())?;
        state.end()
    }
}

impl<D> EndianLayout<AnyNullBitmask, D> {
    pub(crate) fn endian_uint_try_new(
        cs: NonEmpty<ColumnLayoutValues<D::MeasDatatype>>,
        e: Endian,
        notrunc: bool,
    ) -> DeferredResult<Self, ColumnError<BitmaskError>, ColumnError<NewUintTypeError>>
    where
        D: MeasDatatypeDef,
    {
        FixedLayout::try_new(cs, e, |c| {
            AnyBitmask::from_width_and_range(c.width, c.range, notrunc).def_errors_into()
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

impl<D> EndianLayout<NullMixedType, D> {
    pub(crate) fn try_into_ordered<T>(
        self,
    ) -> MultiResult<AnyOrderedLayout<T>, MixedToOrderedLayoutError> {
        let c0 = self.columns.head;
        let cs = self.columns.tail;
        let endian = self.byte_layout;
        match c0 {
            MixedType::Ascii(x) => cs
                .into_iter()
                .enumerate()
                .map(|(i, c)| {
                    c.try_into().map_err(|e| MixedColumnConvertError {
                        error: MixedToOrderedConvertError::from(e),
                        index: (i + 1).into(),
                    })
                })
                .gather()
                .map(|xs| AnyAsciiLayout::Fixed(FixedLayout::new1(x, xs, NoByteOrd)).into()),
            MixedType::Uint(x) => x
                .try_into_one_size(cs, endian, 1)
                .map(|i| i.into())
                .mult_map_errors(|(index, error)| MixedColumnConvertError {
                    index,
                    error: error.into(),
                }),
            MixedType::F32(x) => cs
                .into_iter()
                .enumerate()
                .map(|(i, c)| {
                    c.try_into().map_err(|e| MixedColumnConvertError {
                        error: MixedToOrderedConvertError::from(e),
                        index: (i + 1).into(),
                    })
                })
                .gather()
                .map(|xs| FixedLayout::new1(x, xs, endian.into()).into()),
            MixedType::F64(x) => cs
                .into_iter()
                .enumerate()
                .map(|(i, c)| {
                    c.try_into().map_err(|e| MixedColumnConvertError {
                        error: MixedToOrderedConvertError::from(e),
                        index: (i + 1).into(),
                    })
                })
                .gather()
                .map(|xs| FixedLayout::new1(x, xs, endian.into()).into()),
        }
    }

    pub(crate) fn try_into_non_mixed(
        self,
    ) -> MultiResult<NonMixedEndianLayout<NoMeasDatatype>, MixedToNonMixedLayoutError> {
        let c0 = self.columns.head;
        let it = self.columns.tail.into_iter().enumerate();
        let byte_layout = self.byte_layout;
        match c0 {
            MixedType::Ascii(x) => it
                .map(|(i, c)| c.try_into().map_err(|e| (i, e)))
                .gather()
                .map(|xs| AnyAsciiLayout::Fixed(FixedLayout::new1(x, xs, NoByteOrd)).into()),
            MixedType::Uint(x) => it
                .map(|(i, c)| c.try_into().map_err(|e| (i, e)))
                .gather()
                .map(|xs| FixedLayout::new1(x, xs, byte_layout).into()),
            MixedType::F32(x) => it
                .map(|(i, c)| c.try_into().map_err(|e| (i, e)))
                .gather()
                .map(|xs| FixedLayout::new1(x, xs, byte_layout).into()),
            MixedType::F64(x) => it
                .map(|(i, c)| c.try_into().map_err(|e| (i, e)))
                .gather()
                .map(|xs| FixedLayout::new1(x, xs, byte_layout).into()),
        }
        .mult_map_errors(|(i, error)| MixedColumnConvertError {
            index: (i + 1).into(),
            error,
        })
    }
}

// TODO doesn't num_traits have this?
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
        }
    };
}

impl_num_props!(1, u8);
impl_num_props!(2, u16);
impl_num_props!(4, u32);
impl_num_props!(8, u64);
impl_num_props!(4, f32);
impl_num_props!(8, f64);

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
            .map(|(i, c)| Self::try_from(c).map_err(|e| ((i + starting_index).into(), e)))
            .gather()
    }

    fn from_u64(range: u64) -> Self
    where
        T: TryFrom<u64> + PrimInt,
    {
        // TODO use arg for xform
        Self {
            range: Bitmask::from_u64(range).0,
            xform: UintXform::Lin(PositiveFloat::one()),
        }
    }
}

impl<T, const LEN: usize> FloatType<T, LEN> {
    pub fn new(range: FloatDecimal<T>) -> Self {
        Self {
            range,
            _t: PhantomData,
        }
    }

    /// Make new float range from $PnB and $PnR values.
    ///
    /// Will return an error if $PnB is the incorrect size.
    pub(crate) fn from_width_and_range(
        width: Width,
        range: Range,
        notrunc: bool,
    ) -> DeferredResult<Self, FloatToDecimalError, FloatWidthError>
    where
        FloatDecimal<T>: TryFrom<BigDecimal, Error = FloatToDecimalError>,
        T: HasFloatBounds,
    {
        Bytes::try_from(width)
            .into_deferred()
            .def_and_maybe(|bytes| {
                if usize::from(u8::from(bytes)) == LEN {
                    Ok(Self::from_range(range, notrunc).errors_into())
                } else {
                    Err(DeferredFailure::new1(FloatWidthError::WrongWidth(
                        WrongFloatWidth {
                            expected: LEN,
                            width: bytes,
                        },
                    )))
                }
            })
    }
}

// TODO also check scale here?
impl NullMixedType {
    /// Make a new mixed range from $PnB and $PnR, and $PnDATATYPE values
    pub(crate) fn from_width_and_range(
        width: Width,
        range: Range,
        datatype: Option<NumType>,
        notrunc: bool,
    ) -> DeferredResult<Self, NewMixedTypeWarning, NewMixedTypeError> {
        if let Some(dt) = datatype {
            match dt {
                NumType::Integer => AnyBitmask::from_width_and_range(width, range, notrunc)
                    .def_map_value(Self::Uint)
                    .def_inner_into(),
                NumType::Single => F32Type::from_width_and_range(width, range, notrunc)
                    .def_map_value(Self::F32)
                    .def_inner_into(),
                NumType::Double => F64Type::from_width_and_range(width, range, notrunc)
                    .def_map_value(Self::F64)
                    .def_inner_into(),
            }
        } else {
            AsciiRange::from_width_and_range(width, range, notrunc)
                .def_map_value(Self::Ascii)
                .def_inner_into()
        }
    }

    fn as_alpha_num_type(&self) -> AlphaNumType {
        match self {
            Self::Ascii(_) => AlphaNumType::Ascii,
            Self::Uint(_) => AlphaNumType::Integer,
            Self::F32(_) => AlphaNumType::Single,
            Self::F64(_) => AlphaNumType::Double,
        }
    }
}

impl AnyNullBitmask {
    /// Make a new bitmask from $PnB and PnR values.
    ///
    /// Will return an error if $PnB (in bits) cannot be converted into a width
    /// in bytes.
    fn from_width_and_range(
        width: Width,
        range: Range,
        notrunc: bool,
    ) -> DeferredResult<Self, BitmaskError, NewUintTypeError> {
        width
            .try_into()
            .into_deferred()
            .def_and_tentatively(|bytes| Self::new1(bytes, range, notrunc).errors_into())
    }

    /// Make a new bitmask with a given width (in bytes) using a float/int.
    fn new1(width: Bytes, range: Range, notrunc: bool) -> BiTentative<Self, BitmaskError> {
        match width {
            Bytes::B1 => UintType08::from_range(range, notrunc).map(|b| b.into()),
            Bytes::B2 => UintType16::from_range(range, notrunc).map(|b| b.into()),
            Bytes::B3 => UintType24::from_range(range, notrunc).map(|b| b.into()),
            Bytes::B4 => UintType32::from_range(range, notrunc).map(|b| b.into()),
            Bytes::B5 => UintType40::from_range(range, notrunc).map(|b| b.into()),
            Bytes::B6 => UintType48::from_range(range, notrunc).map(|b| b.into()),
            Bytes::B7 => UintType56::from_range(range, notrunc).map(|b| b.into()),
            Bytes::B8 => UintType64::from_range(range, notrunc).map(|b| b.into()),
        }
    }

    /// Make a new bitmask from a u64.
    ///
    /// The width is determined by the magnitude of the range; the smallest
    /// possible will be used.
    pub fn from_u64(range: u64) -> Self {
        // ASSUME these will never truncate because we check the width first
        match Bytes::from_u64(range) {
            Bytes::B1 => Self::Uint08(UintType::from_u64(range)),
            Bytes::B2 => Self::Uint16(UintType::from_u64(range)),
            Bytes::B3 => Self::Uint24(UintType::from_u64(range)),
            Bytes::B4 => Self::Uint32(UintType::from_u64(range)),
            Bytes::B5 => Self::Uint40(UintType::from_u64(range)),
            Bytes::B6 => Self::Uint48(UintType::from_u64(range)),
            Bytes::B7 => Self::Uint56(UintType::from_u64(range)),
            Bytes::B8 => Self::Uint64(UintType::from_u64(range)),
        }
    }

    pub(crate) fn try_into_one_size<X, E, T>(
        self,
        tail: Vec<X>,
        endian: Endian,
        starting_index: usize,
    ) -> MultiResult<AnyOrderedUintLayout<T>, (MeasIndex, E)>
    where
        UintType08: TryFrom<X, Error = E>,
        UintType16: TryFrom<X, Error = E>,
        UintType24: TryFrom<X, Error = E>,
        UintType32: TryFrom<X, Error = E>,
        UintType40: TryFrom<X, Error = E>,
        UintType48: TryFrom<X, Error = E>,
        UintType56: TryFrom<X, Error = E>,
        UintType64: TryFrom<X, Error = E>,
    {
        match_any_uint!(self, Self, x, {
            UintType::try_from_many(tail, starting_index)
                .map(|xs| FixedLayout::new1(x, xs, endian.into()).into())
        })
    }
}

fn ascii_to_uint(buf: &[u8]) -> Result<u64, AsciiToUintError> {
    if buf.is_ascii() {
        let s = unsafe { str::from_utf8_unchecked(buf) };
        s.parse().map_err(AsciiToUintError::from)
    } else {
        Err(NotAsciiError(buf.to_vec()).into())
    }
}

impl From<ColumnLayoutValues3_2> for ColumnLayoutValues2_0 {
    fn from(value: ColumnLayoutValues3_2) -> Self {
        Self {
            width: value.width,
            range: value.range,
            datatype: NullMeasDatatype,
        }
    }
}

impl<T, D, const ORD: bool> LayoutOps<'_, T> for DelimAsciiLayout<T, D, ORD>
where
    T: TotDefinition,
    NoByteOrd<ORD>: HasByteOrd,
{
    fn ncols(&self) -> usize {
        self.ranges.len()
    }

    fn nbytes(&self, df: &FCSDataFrame) -> u64 {
        df.ascii_nbytes()
    }

    fn widths(&self) -> Vec<BitsOrChars> {
        vec![]
    }

    fn ranges(&self) -> NonEmpty<Range> {
        self.ranges.as_ref().map(|x| Range::from(*x))
    }

    fn datatype(&self) -> AlphaNumType {
        AlphaNumType::Ascii
    }

    fn byteord_keyword(&self) -> (String, String) {
        // NOTE BYTEORD is meaningless for delimited ASCII so use a dummy
        <NoByteOrd<ORD> as HasByteOrd>::ByteOrd::from(NoByteOrd).pair()
    }

    fn req_meas_keywords(&self) -> NonEmpty<[(String, String); 2]> {
        self.ranges.as_ref().enumerate().map(|(i, r)| {
            let x = Width::Variable.pair(i.into());
            let y = Range((*r).into()).pair(i.into());
            [x, y]
        })
    }

    fn remove_nocheck(&mut self, index: MeasIndex) -> Result<(), ClearOptional> {
        self.ranges.remove_nocheck(index.into())
    }

    fn h_read_df_inner<R: Read>(
        &self,
        h: &mut BufReader<R>,
        _: &mut Vec<u8>,
        tot: T::Tot,
        seg: AnyDataSegment,
        _: &ReaderConfig,
    ) -> IODeferredResult<FCSDataFrame, ReadDataframeWarning, ReadDataframeError> {
        let rs = &self.ranges;
        let nbytes = seg.inner.len() as usize;
        T::with_tot(
            h,
            tot,
            |_h, t| h_read_delim_with_rows(rs, _h, t, nbytes).map_err(|e| e.inner_into()),
            |_h| h_read_delim_without_rows(rs, _h, nbytes).map_err(|e| e.inner_into()),
        )
        .into_deferred()
    }

    fn check_writer(&self, df: &FCSDataFrame) -> MultiResult<(), ColumnError<AnyLossError>> {
        df.iter_columns()
            .enumerate()
            .map(|(i, c)| {
                c.check_writer::<_, _, u64>(|_| None)
                    .map_err(|error| ColumnError {
                        error: AnyLossError::Int(error),
                        index: i.into(),
                    })
            })
            .gather()
            .void()
    }

    fn h_write_df_inner<W: Write>(
        &self,
        h: &mut BufWriter<W>,
        df: &FCSDataFrame,
    ) -> io::Result<()> {
        let ncols = df.ncols();
        let nrows = df.nrows();
        // ASSUME dataframe has correct number of columns
        let mut column_srcs: Vec<_> = df.iter_columns().map(AnySource::<'_, u64>::new).collect();
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

impl<T, D, const ORD: bool> InterLayoutOps<D> for DelimAsciiLayout<T, D, ORD> {
    fn opt_meas_headers(&self) -> Vec<MeasHeader> {
        vec![]
    }

    fn opt_meas_keywords(&self) -> NonEmpty<Vec<(String, Option<String>)>> {
        self.ranges.as_ref().map(|_| vec![])
    }

    fn insert_nocheck(
        &mut self,
        index: MeasIndex,
        range: Range,
        notrunc: bool,
    ) -> BiTentative<(), AnyRangeError> {
        range
            .into_uint(notrunc)
            .inner_into()
            .map(|r| self.ranges.insert(index.into(), r))
    }

    fn push(&mut self, range: Range, notrunc: bool) -> BiTentative<(), AnyRangeError> {
        range
            .into_uint(notrunc)
            .inner_into()
            .map(|r| self.ranges.push(r))
    }
}

impl<T, D, const ORD: bool> DelimAsciiLayout<T, D, ORD> {
    fn new(ranges: NonEmpty<u64>) -> Self {
        Self {
            ranges,
            _tot_def: PhantomData,
            _meas_data_def: PhantomData,
        }
    }

    fn check_writer(&self, df: &FCSDataFrame) -> MultiResult<(), ColumnError<AnyLossError>> {
        df.iter_columns()
            .enumerate()
            .map(|(i, c)| {
                c.check_writer::<_, _, u64>(|_| None)
                    .map_err(|error| ColumnError {
                        error: AnyLossError::Int(error),
                        index: i.into(),
                    })
            })
            .gather()
            .void()
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

impl<'a, C, S, T, D> LayoutOps<'a, T> for FixedLayout<C, S, T, D>
where
    D: MeasDatatypeDef,
    T: TotDefinition,
    C: Clone + IsFixed + HasDatatype + IntoReader<S> + IntoWriter<'a, S> + FromRange,
    S: Copy + HasByteOrd,
    for<'c> Range: From<&'c C>,
    <C as IntoReader<S>>::Target: Readable<S>,
    <C as IntoWriter<'a, S>>::Target: Writable<'a, S>,
    AnyRangeError: From<<C as FromRange>::Error>,
{
    fn widths(&self) -> Vec<BitsOrChars> {
        self.columns.as_ref().map(|x| x.fixed_width()).into()
    }

    fn ranges(&self) -> NonEmpty<Range> {
        self.columns.as_ref().map(|x| x.into())
    }

    fn ncols(&self) -> usize {
        self.columns.len()
    }

    fn nbytes(&self, df: &FCSDataFrame) -> u64 {
        (self.event_width() * df.nrows()) as u64
    }

    fn datatype(&self) -> AlphaNumType {
        C::datatype_from_columns(&self.columns)
    }

    fn byteord_keyword(&self) -> (String, String) {
        S::ByteOrd::from(self.byte_layout).pair()
    }

    fn req_meas_keywords(&self) -> NonEmpty<[(String, String); 2]> {
        self.columns
            .as_ref()
            .enumerate()
            .map(|(i, c)| c.req_meas_keywords(i.into()))
    }

    fn remove_nocheck(&mut self, index: MeasIndex) -> Result<(), ClearOptional> {
        self.columns.remove_nocheck(index.into())
    }

    fn h_read_df_inner<R: Read>(
        &self,
        h: &mut BufReader<R>,
        buf: &mut Vec<u8>,
        tot: T::Tot,
        seg: AnyDataSegment,
        conf: &ReaderConfig,
    ) -> IODeferredResult<FCSDataFrame, ReadDataframeWarning, ReadDataframeError>
    where
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
                self.h_read_unchecked_df(h, nrows, buf)
                    .map_err(|e| e.inner_into())
                    .into_deferred()
            })
    }

    fn check_writer(&self, df: &'a FCSDataFrame) -> MultiResult<(), ColumnError<AnyLossError>> {
        // ASSUME df has same number of columns as layout
        self.columns
            .iter()
            .zip(df.iter_columns())
            .enumerate()
            .map(|(i, (col_type, col_data))| {
                col_type
                    .check_writer(col_data)
                    .map_err(|error| ColumnError {
                        error,
                        index: i.into(),
                    })
            })
            .gather()
            .void()
    }

    fn h_write_df_inner<W: Write>(
        &self,
        h: &mut BufWriter<W>,
        df: &'a FCSDataFrame,
    ) -> io::Result<()> {
        let nrows = df.nrows();
        // ASSUME df has same number of columns as layout
        let mut cs: Vec<_> = self
            .columns
            .iter()
            .zip(df.iter_columns())
            .map(|(col_type, col_data)| col_type.clone().into_writer(col_data))
            .collect();
        for _ in 0..nrows {
            for c in cs.iter_mut() {
                c.h_write(h, self.byte_layout)?;
            }
        }
        Ok(())
    }
}

impl<'a, C, S, T, D> InterLayoutOps<D> for FixedLayout<C, S, T, D>
where
    T: TotDefinition,
    C: Clone + IsFixed + HasDatatype + IntoReader<S> + IntoWriter<'a, S> + FromRange,
    S: Copy + HasByteOrd,
    for<'c> Range: From<&'c C>,
    <C as IntoReader<S>>::Target: Readable<S>,
    <C as IntoWriter<'a, S>>::Target: Writable<'a, S>,
    AnyRangeError: From<<C as FromRange>::Error>,
{
    fn opt_meas_headers(&self) -> Vec<MeasHeader> {
        vec![]
    }

    fn opt_meas_keywords(&self) -> NonEmpty<Vec<(String, Option<String>)>> {
        self.columns.as_ref().map(|_| vec![])
    }

    fn insert_nocheck(
        &mut self,
        index: MeasIndex,
        range: Range,
        notrunc: bool,
    ) -> BiTentative<(), AnyRangeError> {
        C::from_range(range, notrunc)
            .inner_into()
            .map(|col| self.insert_column(index, col))
    }

    fn push(&mut self, range: Range, notrunc: bool) -> BiTentative<(), AnyRangeError> {
        C::from_range(range, notrunc)
            .inner_into()
            .map(|col| self.push_column(col))
    }
}

impl<C, S, T, D> OrderedLayoutOps for FixedLayout<C, S, T, D>
where
    S: Copy,
    ByteOrd2_0: From<S>,
{
    fn byte_order(&self) -> Option<ByteOrd2_0> {
        Some(self.byte_layout.into())
    }
}

impl<C, T, D> EndianLayoutOps for FixedLayout<C, Endian, T, D> {
    fn endianness(&self) -> Option<Endian> {
        Some(self.byte_layout)
    }
}

impl<C, S, T, D> FixedLayout<C, S, T, D> {
    fn new(columns: NonEmpty<C>, byte_layout: S) -> Self {
        Self {
            columns,
            byte_layout,
            _tot_def: PhantomData,
            _meas_data_def: PhantomData,
        }
    }

    fn new1(head: C, tail: Vec<C>, byte_layout: S) -> Self {
        Self::new((head, tail).into(), byte_layout)
    }

    fn try_new<F, W, E, CW, CE>(
        cs: NonEmpty<ColumnLayoutValues<D::MeasDatatype>>,
        byte_layout: S,
        new_col_f: F,
    ) -> DeferredResult<Self, W, E>
    where
        D: MeasDatatypeDef,
        W: From<ColumnError<CW>>,
        E: From<ColumnError<CE>>,
        F: Fn(ColumnLayoutValues<D::MeasDatatype>) -> DeferredResult<C, CW, CE>,
    {
        cs.enumerate()
            .map_results(|(i, c)| {
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

    fn h_read_unchecked_df<R: Read>(
        &self,
        h: &mut BufReader<R>,
        nrows: usize,
        buf: &mut Vec<u8>,
    ) -> IOResult<FCSDataFrame, ReadDataframeError>
    where
        S: Copy,
        C: IsFixed + Clone + IntoReader<S>,
        <C as IntoReader<S>>::Target: Readable<S>,
    {
        // TODO to clone
        let mut col_readers: Vec<_> = self
            .columns
            .iter()
            .map(|c| c.clone().into_reader(nrows))
            .collect();
        for row in 0..nrows {
            for c in col_readers.iter_mut() {
                c.h_read(h, row, self.byte_layout, buf)
                    .map_err(|e| e.inner_into())?;
            }
        }
        let data = col_readers
            .into_iter()
            .map(|c| c.into_dataframe_column())
            .collect();
        Ok(FCSDataFrame::try_new(data).unwrap())
    }

    fn insert_column(&mut self, index: MeasIndex, col: C) {
        self.columns.insert(index.into(), col)
    }

    fn push_column(&mut self, col: C) {
        self.columns.push(col)
    }

    fn columns_into<X>(self) -> FixedLayout<X, S, T, D>
    where
        X: From<C>,
    {
        FixedLayout::new(self.columns.map(|c| c.into()), self.byte_layout)
    }

    fn byte_layout_into<X>(self) -> FixedLayout<C, X, T, D>
    where
        X: From<S>,
    {
        FixedLayout::new(self.columns, self.byte_layout.into())
    }

    fn byte_layout_try_into<X>(self) -> Result<FixedLayout<C, X, T, D>, X::Error>
    where
        X: TryFrom<S>,
    {
        self.byte_layout
            .try_into()
            .map(|byte_layout| FixedLayout::new(self.columns, byte_layout))
    }

    fn phantom_into<T1, D1>(self) -> FixedLayout<C, S, T1, D1> {
        FixedLayout::new(self.columns, self.byte_layout)
    }

    fn event_width(&self) -> usize
    where
        C: IsFixed,
    {
        self.columns.iter().map(|c| usize::from(c.nbytes())).sum()
    }

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
}

impl<C> EndianLayout<C, HasMeasDatatype> {
    fn insert_mixed(
        mut self,
        index: MeasIndex,
        range: Range,
        notrunc: bool,
    ) -> BiTentative<DataLayout3_2, AnyRangeError>
    where
        C: TryFrom<NullMixedType, Error = MixedToInnerError>,
        NullMixedType: From<C>,
        NonMixedEndianLayout<HasMeasDatatype>: From<EndianLayout<C, HasMeasDatatype>>,
    {
        NullMixedType::from_range(range, notrunc).map(|col| match col.try_into() {
            Ok(c) => {
                self.insert_column(index, c);
                DataLayout3_2::NonMixed(self.into())
            }
            Err(e) => {
                let mut z = self.columns_into();
                z.insert_column(index, e.src);
                z.into()
            }
        })
    }

    fn push_mixed(
        mut self,
        range: Range,
        notrunc: bool,
    ) -> BiTentative<DataLayout3_2, AnyRangeError>
    where
        C: TryFrom<NullMixedType, Error = MixedToInnerError>,
        NullMixedType: From<C>,
        NonMixedEndianLayout<HasMeasDatatype>: From<EndianLayout<C, HasMeasDatatype>>,
    {
        NullMixedType::from_range(range, notrunc).map(|col| match col.try_into() {
            Ok(c) => {
                self.push_column(c);
                DataLayout3_2::NonMixed(self.into())
            }
            Err(e) => {
                let mut z = self.columns_into();
                z.push_column(e.src);
                z.into()
            }
        })
    }
}

macro_rules! def_native_wrapper {
    ($name:path, $native:ty, $size:expr, $native_size:expr, $bytes:ident) => {
        impl HasNativeType for $name {
            type Native = $native;
        }

        impl HasNativeWidth for $name {
            const BYTES: Bytes = Bytes::$bytes;
            const LEN: usize = $native_size;
            type Order = SizedByteOrd<$size>;
        }
    };
}

def_native_wrapper!(UintType08, u8, 1, 1, B1);
def_native_wrapper!(UintType16, u16, 2, 2, B2);
def_native_wrapper!(UintType24, u32, 3, 4, B3);
def_native_wrapper!(UintType32, u32, 4, 4, B4);
def_native_wrapper!(UintType40, u64, 5, 8, B5);
def_native_wrapper!(UintType48, u64, 6, 8, B6);
def_native_wrapper!(UintType56, u64, 7, 8, B7);
def_native_wrapper!(UintType64, u64, 8, 8, B8);
def_native_wrapper!(F32Type, f32, 4, 4, B4);
def_native_wrapper!(F64Type, f64, 8, 8, B8);

impl HasNativeType for AsciiRange {
    type Native = u64;
}

impl HasDatatype for AsciiRange {
    fn datatype_from_columns(_: &NonEmpty<Self>) -> AlphaNumType {
        AlphaNumType::Ascii
    }
}

impl<T, const LEN: usize> HasDatatype for UintType<T, LEN> {
    fn datatype_from_columns(_: &NonEmpty<Self>) -> AlphaNumType {
        AlphaNumType::Integer
    }
}

impl HasDatatype for F32Type {
    fn datatype_from_columns(_: &NonEmpty<Self>) -> AlphaNumType {
        AlphaNumType::Single
    }
}

impl HasDatatype for F64Type {
    fn datatype_from_columns(_: &NonEmpty<Self>) -> AlphaNumType {
        AlphaNumType::Double
    }
}

impl HasDatatype for AnyNullBitmask {
    fn datatype_from_columns(_: &NonEmpty<Self>) -> AlphaNumType {
        AlphaNumType::Integer
    }
}

impl HasDatatype for NullMixedType {
    fn datatype_from_columns(cs: &NonEmpty<Self>) -> AlphaNumType {
        // If any numeric types are none, then that means at least one column is
        // ASCII, which means that $DATATYPE needs to be "A" since $PnDATATYPE
        // cannot be "A". Otherwise, find majority type.
        cs.as_ref()
            .try_map(|c| NumType::try_from(c.as_alpha_num_type()))
            .ok()
            .map_or(AlphaNumType::Ascii, |mut ds| {
                ds.sort();
                let x = (*ds.mode().0).into();
                x
            })
    }
}

impl<T, const LEN: usize> FromRange for UintType<T, LEN>
where
    T: TryFrom<Range, Error = IntRangeError<T>> + PrimInt,
    u64: From<T>,
{
    type Error = BitmaskError;

    // TODO this is wrong
    fn from_range(range: Range, notrunc: bool) -> BiTentative<Self, Self::Error> {
        range.into_uint(notrunc).inner_into().and_tentatively(|x| {
            Bitmask::from_native_tnt(x, notrunc)
                .inner_into()
                .map(|r| UintType {
                    range: r,
                    // TODO make default method
                    xform: UintXform::Lin(PositiveFloat::one()),
                })
        })
    }
}

impl<T, const LEN: usize> FromRange for FloatType<T, LEN>
where
    T: HasFloatBounds,
{
    type Error = FloatToDecimalError;

    fn from_range(range: Range, notrunc: bool) -> BiTentative<Self, Self::Error> {
        range.into_float(notrunc).map(Self::new)
    }
}

impl FromRange for AsciiRange {
    type Error = IntRangeError<()>;

    /// Make new AsciiRange from a float or integer.
    ///
    /// The number of chars will be automatically selected as the minimum
    /// required to express the range.
    fn from_range(range: Range, notrunc: bool) -> BiTentative<Self, Self::Error> {
        range.into_uint::<u64>(notrunc).map(AsciiRange::from)
    }
}

impl FromRange for AnyNullBitmask {
    type Error = IntRangeError<()>;

    /// make a new bitmask from a float or integer.
    ///
    /// The size will be determined by the input and will be kept as small as
    /// possible.
    fn from_range(range: Range, notrunc: bool) -> BiTentative<Self, Self::Error> {
        range.into_uint(notrunc).map(Self::from_u64)
    }
}

impl FromRange for NullMixedType {
    type Error = AnyRangeError;

    /// Create a mixed type based on the range.
    ///
    /// If int is supplied, return one of the uint types depending on size. If
    /// float is supplied, return f64 if range extends beyond the bounds of f32,
    /// otherwise use f32 (note that precision is not taken into consideration).
    ///
    /// ASCII will never be returned. This method will never fail.
    fn from_range(range: Range, notrunc: bool) -> BiTentative<Self, Self::Error> {
        if range.0.is_integer() {
            AnyBitmask::from_range(range, notrunc)
                .map(Self::Uint)
                .inner_into()
        } else {
            let (x, e) = FloatDecimal::<f32>::try_from(range.0)
                .map_or_else(
                    |e| FloatDecimal::<f64>::try_from(e.src).map(|r| Self::F64(FloatType::new(r))),
                    |r| Ok(Self::F32(FloatType::new(r))),
                )
                .map_or_else(
                    |e| {
                        // TODO kinda not dry
                        let m = if e.over {
                            f64::max_decimal()
                        } else {
                            f64::min_decimal()
                        };
                        (Self::F64(FloatType::new(m)), Some(e))
                    },
                    |x| (x, None),
                );
            BiTentative::new_either1(x, e, notrunc).inner_into()
        }
    }
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
        let x = u64::from(self.range.value());
        // TODO fix u64 max
        Range(if x == u64::MAX { x } else { x + 1 }.into())
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

    fn range(&self) -> Range {
        // TODO clone?
        self.range.clone().into()
    }
}

impl IsFixed for AsciiRange {
    fn nbytes(&self) -> u8 {
        self.chars().into()
    }

    fn fixed_width(&self) -> BitsOrChars {
        self.chars().into()
    }

    fn range(&self) -> Range {
        Range(self.value().into())
    }
}

impl IsFixed for AnyNullBitmask {
    fn nbytes(&self) -> u8 {
        match_any_uint!(self, Self, x, { x.nbytes() })
    }

    fn fixed_width(&self) -> BitsOrChars {
        match_any_uint!(self, Self, x, { x.fixed_width() })
    }

    fn range(&self) -> Range {
        match_any_uint!(self, Self, x, { x.range() })
    }
}

impl IsFixed for NullMixedType {
    fn nbytes(&self) -> u8 {
        match_any_mixed!(self, x, { x.nbytes() })
    }

    fn fixed_width(&self) -> BitsOrChars {
        match_any_mixed!(self, x, { x.fixed_width() })
    }

    fn range(&self) -> Range {
        match_any_mixed!(self, x, { x.range() })
    }
}

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

impl<T> AnyOrderedUintLayout<T> {
    fn phantom_into<X>(self) -> AnyOrderedUintLayout<X> {
        match_any_uint!(self, Self, l, { l.phantom_into().into() })
    }

    fn into_endian<D>(self) -> Result<EndianLayout<AnyNullBitmask, D>, OrderedToEndianError> {
        match_any_uint!(self, Self, l, {
            l.phantom_into()
                .byte_layout_try_into()
                .map(|x| x.columns_into())
        })
    }

    fn try_new(
        cs: NonEmpty<ColumnLayoutValues<NullMeasDatatype>>,
        bo: ByteOrd2_0,
        conf: &StdTextReadConfig,
    ) -> DeferredResult<Self, ColumnError<BitmaskError>, NewFixedIntLayoutError> {
        let notrunc = conf.disallow_range_truncation;
        let real_bo = conf.integer_byteord_override.unwrap_or(bo);
        let n = real_bo.nbytes();
        // First, scan through the widths to make sure they are all fixed and
        // are all the same number of bytes as ByteOrd. Skip this step if we
        // are ignoring $PnB for width and simply using the length of $BYTEORD.
        let width_res = if conf.integer_widths_from_byteord {
            Ok(())
        } else {
            cs.as_ref()
                .map(|c| c.width)
                .map_results(Bytes::try_from)
                .mult_map_errors(SingleFixedWidthError::Bytes)
                .and_then(|widths| {
                    let us = widths.unique();
                    if us.tail.is_empty() && us.head == n {
                        Ok(())
                    } else {
                        Err(NonEmpty::new(
                            WidthMismatchError {
                                byteord: real_bo,
                                found: us,
                            }
                            .into(),
                        ))
                    }
                })
                .void()
        };
        // Second, make the layout, and force all columns to the correct type
        // based on ByteOrd. It is necessary to check the columns first because
        // the bitmask won't necessarily fail even if it is larger than the
        // target type.
        width_res.mult_to_deferred().def_and_maybe(|_| {
            match_many_to_one!(real_bo, ByteOrd2_0, [O1, O2, O3, O4, O5, O6, O7, O8], o, {
                FixedLayout::try_new(cs, o, |c| {
                    // NOTE at this point $PnB doesn't matter, so assume we
                    // either ignored $PnB by way of $BYTEORD or checked to make
                    // sure they match.
                    Ok(UintType::from_range(c.range, notrunc).errors_into())
                })
                .def_map_value(|x| x.into())
            })
        })
    }
}

impl<T, D> OrderedLayoutOps for AnyAsciiLayout<T, D, true> {
    fn byte_order(&self) -> Option<ByteOrd2_0> {
        None
    }
}

impl<T, D> EndianLayoutOps for AnyAsciiLayout<T, D, false> {
    fn endianness(&self) -> Option<Endian> {
        None
    }
}

impl<T, D, const ORD: bool> AnyAsciiLayout<T, D, ORD> {
    fn phantom_into<T1, D1, const ORD_1: bool>(self) -> AnyAsciiLayout<T1, D1, ORD_1> {
        match self {
            Self::Delimited(x) => DelimAsciiLayout::new(x.ranges).into(),
            Self::Fixed(x) => FixedLayout::new(x.columns, NoByteOrd).into(),
        }
    }

    pub(crate) fn try_new(
        cs: NonEmpty<ColumnLayoutValues<D::MeasDatatype>>,
        notrunc: bool,
    ) -> DeferredResult<
        Self,
        ColumnError<IntRangeError<()>>,
        ColumnError<ascii_range::NewAsciiRangeError>,
    >
    where
        D: MeasDatatypeDef,
    {
        let go = |error: IntRangeError<()>, i: usize| ColumnError {
            error,
            index: i.into(),
        };
        if cs.iter().all(|c| c.width == Width::Variable) {
            let ts = cs.enumerate().map(|(i, c)| {
                c.range
                    .into_uint(notrunc)
                    .map_errors(|e| go(e, i))
                    .map_warnings(|e| go(e, i))
            });
            let ret = Tentative::mconcat_ne(ts)
                .map(|ranges| DelimAsciiLayout::new(ranges).into())
                .map_errors(|e| e.inner_into());
            Ok(ret)
        } else {
            FixedLayout::try_new(cs, NoByteOrd, |c| {
                AsciiRange::from_width_and_range(c.width, c.range, notrunc)
            })
            .def_map_value(Self::Fixed)
        }
    }

    fn new_fixed(columns: NonEmpty<AsciiRange>) -> Self {
        Self::Fixed(FixedLayout::new(columns, NoByteOrd))
    }

    fn new_delim(ranges: NonEmpty<u64>) -> Self {
        Self::Delimited(DelimAsciiLayout::new(ranges))
    }
}

impl VersionedDataLayout for DataLayout2_0 {
    type ByteLayout = ByteOrd2_0;
    type MeasDTDef = NoMeasDatatype;
    type TotDef = MaybeTot;

    fn lookup(
        kws: &mut StdKeywords,
        conf: &StdTextReadConfig,
        par: Par,
    ) -> LookupLayoutResult<Option<Self>> {
        AnyOrderedLayout::lookup(kws, conf, par).def_map_value(|x| x.map(|y| y.into()))
    }

    fn lookup_ro(kws: &StdKeywords, conf: &StdTextReadConfig) -> FromRawResult<Option<Self>> {
        AnyOrderedLayout::lookup_ro(kws, conf).def_map_value(|x| x.map(|y| y.into()))
    }

    fn try_new(
        datatype: AlphaNumType,
        byteord: Self::ByteLayout,
        columns: NonEmpty<ColumnLayoutValues<<Self::MeasDTDef as MeasDatatypeDef>::MeasDatatype>>,
        conf: &StdTextReadConfig,
    ) -> DeferredResult<Self, ColumnError<NewMixedTypeWarning>, NewDataLayoutError> {
        AnyOrderedLayout::try_new(datatype, byteord, columns, conf)
            .def_map_value(|x| x.into())
            .def_map_warnings(|e| e.inner_into())
    }
}

impl VersionedDataLayout for DataLayout3_0 {
    type ByteLayout = ByteOrd2_0;
    type MeasDTDef = NoMeasDatatype;
    type TotDef = KnownTot;

    fn lookup(
        kws: &mut StdKeywords,
        conf: &StdTextReadConfig,
        par: Par,
    ) -> LookupLayoutResult<Option<Self>> {
        AnyOrderedLayout::lookup(kws, conf, par).def_map_value(|x| x.map(|y| y.into()))
    }

    fn lookup_ro(kws: &StdKeywords, conf: &StdTextReadConfig) -> FromRawResult<Option<Self>> {
        AnyOrderedLayout::lookup_ro(kws, conf).def_map_value(|x| x.map(|y| y.into()))
    }

    fn try_new(
        datatype: AlphaNumType,
        byteord: Self::ByteLayout,
        columns: NonEmpty<ColumnLayoutValues<NullMeasDatatype>>,
        conf: &StdTextReadConfig,
    ) -> DeferredResult<Self, ColumnError<NewMixedTypeWarning>, NewDataLayoutError> {
        AnyOrderedLayout::try_new(datatype, byteord, columns, conf)
            .def_map_value(|x| x.into())
            .def_map_warnings(|e| e.inner_into())
    }
}

impl VersionedDataLayout for DataLayout3_1 {
    type ByteLayout = Endian;
    type MeasDTDef = NoMeasDatatype;
    type TotDef = KnownTot;

    fn lookup(
        kws: &mut StdKeywords,
        conf: &StdTextReadConfig,
        par: Par,
    ) -> LookupLayoutResult<Option<Self>> {
        NonMixedEndianLayout::lookup(kws, conf, par).def_map_value(|x| x.map(|y| y.into()))
    }

    fn lookup_ro(kws: &StdKeywords, conf: &StdTextReadConfig) -> FromRawResult<Option<Self>> {
        NonMixedEndianLayout::lookup_ro(kws, conf).def_map_value(|x| x.map(|y| y.into()))
    }

    fn try_new(
        datatype: AlphaNumType,
        endian: Self::ByteLayout,
        columns: NonEmpty<ColumnLayoutValues<NullMeasDatatype>>,
        conf: &StdTextReadConfig,
    ) -> DeferredResult<Self, ColumnError<NewMixedTypeWarning>, NewDataLayoutError> {
        NonMixedEndianLayout::try_new(datatype, endian, columns, conf)
            .def_map_value(|x| x.into())
            .def_map_warnings(|e| e.inner_into())
    }
}

impl VersionedDataLayout for DataLayout3_2 {
    type ByteLayout = ByteOrd3_1;
    type MeasDTDef = HasMeasDatatype;
    type TotDef = KnownTot;

    fn lookup(
        kws: &mut StdKeywords,
        conf: &StdTextReadConfig,
        par: Par,
    ) -> LookupLayoutResult<Option<Self>> {
        let d = AlphaNumType::lookup_req_check_ascii(kws);
        let e = ByteOrd3_1::lookup_req(kws);
        let cs = HasMeasDatatype::lookup_all(kws, par);
        d.def_zip3(e, cs)
            .def_inner_into()
            .def_and_maybe(|(datatype, endian, columns)| {
                def_transpose(
                    NonEmpty::from_vec(columns).map(|xs| Self::try_new(datatype, endian, xs, conf)),
                )
                .def_inner_into()
            })
    }

    fn lookup_ro(kws: &StdKeywords, conf: &StdTextReadConfig) -> FromRawResult<Option<Self>> {
        let d = AlphaNumType::get_metaroot_req(kws)
            .map_err(RawParsedError::from)
            .into_deferred();
        let e = ByteOrd3_1::get_metaroot_req(kws)
            .map_err(RawParsedError::from)
            .into_deferred();
        let cs = HasMeasDatatype::lookup_ro_all(kws).def_inner_into();
        d.def_zip3(e, cs)
            .def_and_maybe(|(datatype, endian, columns)| {
                def_transpose(
                    NonEmpty::from_vec(columns).map(|xs| Self::try_new(datatype, endian, xs, conf)),
                )
                .def_inner_into()
            })
    }

    fn try_new(
        datatype: AlphaNumType,
        endian: Self::ByteLayout,
        cs: NonEmpty<ColumnLayoutValues<Option<NumType>>>,
        conf: &StdTextReadConfig,
    ) -> DeferredResult<Self, ColumnError<NewMixedTypeWarning>, NewDataLayoutError> {
        let notrunc = conf.disallow_range_truncation;
        let unique_dt: Vec<_> = cs
            .iter()
            .map(|c| c.datatype.map(|x| x.into()).unwrap_or(datatype))
            .unique()
            .collect();
        match unique_dt[..] {
            [dt] => {
                let ds = cs.map(|c| ColumnLayoutValues {
                    width: c.width,
                    range: c.range,
                    datatype: NullMeasDatatype,
                });
                NonMixedEndianLayout::try_new(dt, endian.0, ds, conf)
                    .def_map_value(|x| Self::NonMixed(x.phantom_into::<HasMeasDatatype>()))
                    .def_map_warnings(|e| e.inner_into())
            }
            _ => FixedLayout::try_new(cs, endian.0, |c| {
                MixedType::from_width_and_range(c.width, c.range, c.datatype, notrunc)
            })
            .def_map_value(Self::Mixed),
        }
    }
}

impl InterLayoutOps<HasMeasDatatype> for DataLayout3_2 {
    fn opt_meas_headers(&self) -> Vec<MeasHeader> {
        vec![NumType::std_blank()]
    }

    // TODO add a way to remove keys which match $DATATYPE so we don't have
    // lots of redundant $PnDATATYPE keys
    fn opt_meas_keywords(&self) -> NonEmpty<Vec<(String, Option<String>)>> {
        match self {
            Self::NonMixed(x) => x
                .ranges()
                .enumerate()
                .map(|(i, _)| vec![NumType::pair_opt(&None.into(), i.into())]),
            Self::Mixed(x) => x.columns.as_ref().enumerate().map(|(i, c)| {
                let y = NumType::try_from(c.as_alpha_num_type()).ok();
                vec![NumType::pair_opt(&y.into(), i.into())]
            }),
        }
    }

    fn insert_nocheck(
        &mut self,
        index: MeasIndex,
        range: Range,
        notrunc: bool,
    ) -> BiTentative<(), AnyRangeError> {
        match mem::replace(self, Self::mixed_dummy()) {
            // If layout is mixed, interpret range as a mixed type
            Self::Mixed(mut x) => x
                .insert_nocheck(index, range, notrunc)
                .map(|_| Self::Mixed(x)),
            // If layout is non-mixed, interpret range as an ASCII range and
            // keep the layout as ASCII. Otherwise, interpret as a mixed range
            // and convert the layout to a mixed layout if the interpreted
            // result is different from the rest of the types in the layout.
            Self::NonMixed(x) => match x {
                NonMixedEndianLayout::Ascii(mut y) => y
                    .insert_nocheck(index, range, notrunc)
                    .map(|_| Self::NonMixed(y.into())),
                NonMixedEndianLayout::Integer(y) => y.insert_mixed(index, range, notrunc),
                NonMixedEndianLayout::F32(y) => y.insert_mixed(index, range, notrunc),
                NonMixedEndianLayout::F64(y) => y.insert_mixed(index, range, notrunc),
            },
        }
        .map(|newself| {
            *self = newself;
        })
    }

    fn push(&mut self, range: Range, notrunc: bool) -> BiTentative<(), AnyRangeError> {
        match mem::replace(self, Self::mixed_dummy()) {
            Self::Mixed(mut x) => x.push(range, notrunc).map(|_| Self::Mixed(x)),
            Self::NonMixed(x) => match x {
                NonMixedEndianLayout::Ascii(mut y) => {
                    y.push(range, notrunc).map(|_| Self::NonMixed(y.into()))
                }
                NonMixedEndianLayout::Integer(y) => y.push_mixed(range, notrunc),
                NonMixedEndianLayout::F32(y) => y.push_mixed(range, notrunc),
                NonMixedEndianLayout::F64(y) => y.push_mixed(range, notrunc),
            },
        }
        .map(|newself| {
            *self = newself;
        })
    }
}

impl DataLayout3_1 {
    pub(crate) fn into_ordered<T>(self) -> LayoutConvertResult<AnyOrderedLayout<T>> {
        self.0.into_ordered()
    }
}

impl DataLayout3_2 {
    pub(crate) fn into_ordered<T>(self) -> LayoutConvertResult<AnyOrderedLayout<T>> {
        match self {
            Self::NonMixed(x) => x.into_ordered(),
            Self::Mixed(x) => x.try_into_ordered().mult_errors_into(),
        }
    }

    pub fn new_mixed(ranges: NonEmpty<NullMixedType>, endian: Endian) -> Self {
        // Check if the mixed types are all the same, in which case we can use a
        // simpler layout. This clone thing is not ideal but it will only be
        // cloning big-decimals for floats and will use Copy for everything else
        // (not a huge deal).
        if let Ok(rs) = ranges.as_ref().try_map(|x| AsciiRange::try_from(x.clone())) {
            NonMixedEndianLayout::new_ascii_fixed(rs).into()
        } else if let Ok(rs) = ranges
            .as_ref()
            .try_map(|x| AnyNullBitmask::try_from(x.clone()))
        {
            NonMixedEndianLayout::new_uint(rs, endian).into()
        } else if let Ok(rs) = ranges.as_ref().try_map(|x| F32Type::try_from(x.clone())) {
            NonMixedEndianLayout::new_f32(rs, endian).into()
        } else if let Ok(rs) = ranges.as_ref().try_map(|x| F64Type::try_from(x.clone())) {
            NonMixedEndianLayout::new_f64(rs, endian).into()
        } else {
            FixedLayout::new(ranges, endian).into()
        }
    }

    pub fn datatypes(&self) -> NonEmpty<AlphaNumType> {
        match self {
            // somewhat hacky way of getting a nonempty in a type-safe way
            Self::NonMixed(x) => LayoutOps::ranges(x)
                .as_ref()
                .map(|_| LayoutOps::datatype(x)),
            Self::Mixed(x) => x.columns.as_ref().map(|y| y.as_alpha_num_type()),
        }
    }

    /// A dummy layout, used to make ['std::mem::replace'] work; not meaninful.
    fn mixed_dummy() -> Self {
        NonMixedEndianLayout::from(AnyAsciiLayout::from(DelimAsciiLayout::new(NonEmpty::new(
            0,
        ))))
        .into()
    }
}

impl<T> AnyOrderedLayout<T> {
    fn lookup(
        kws: &mut StdKeywords,
        conf: &StdTextReadConfig,
        par: Par,
    ) -> LookupLayoutResult<Option<Self>> {
        let cs = NoMeasDatatype::lookup_all(kws, par);
        let d = AlphaNumType::lookup_req(kws);
        let b = ByteOrd2_0::lookup_req(kws);
        d.def_zip3(b, cs)
            .def_inner_into()
            .def_and_maybe(|(datatype, byteord, columns)| {
                def_transpose(
                    NonEmpty::from_vec(columns)
                        .map(|xs| Self::try_new(datatype, byteord, xs, conf)),
                )
                .def_inner_into()
            })
    }

    fn lookup_ro(kws: &StdKeywords, conf: &StdTextReadConfig) -> FromRawResult<Option<Self>> {
        let cs = NoMeasDatatype::lookup_ro_all(kws);
        let d = AlphaNumType::get_metaroot_req(kws).into_deferred();
        let b = ByteOrd2_0::get_metaroot_req(kws).into_deferred();
        d.def_zip3(b, cs)
            .def_inner_into()
            .def_and_maybe(|(datatype, byteord, columns)| {
                def_transpose(
                    NonEmpty::from_vec(columns)
                        .map(|xs| Self::try_new(datatype, byteord, xs, conf)),
                )
                .def_inner_into()
            })
    }

    pub fn new_ascii_fixed(ranges: NonEmpty<AsciiRange>) -> Self {
        AnyAsciiLayout::new_fixed(ranges).into()
    }

    pub fn new_ascii_delim(ranges: NonEmpty<u64>) -> Self {
        AnyAsciiLayout::new_delim(ranges).into()
    }

    pub fn new_uint<U, const LEN: usize>(
        columns: NonEmpty<UintType<U, LEN>>,
        byte_layout: SizedByteOrd<LEN>,
    ) -> Self
    where
        AnyOrderedUintLayout<T>:
            From<FixedLayout<UintType<U, LEN>, SizedByteOrd<LEN>, T, NoMeasDatatype>>,
    {
        Self::Integer(FixedLayout::new(columns, byte_layout).into())
    }

    pub fn new_f32(ranges: NonEmpty<F32Type>, byte_layout: SizedByteOrd<4>) -> Self {
        FixedLayout::new(ranges, byte_layout).into()
    }

    pub fn new_f64(ranges: NonEmpty<F64Type>, byte_layout: SizedByteOrd<8>) -> Self {
        FixedLayout::new(ranges, byte_layout).into()
    }

    fn try_new(
        datatype: AlphaNumType,
        byteord: ByteOrd2_0,
        columns: NonEmpty<ColumnLayoutValues2_0>,
        conf: &StdTextReadConfig,
    ) -> DeferredResult<Self, ColumnError<NewMixedTypeWarning>, NewDataLayoutError> {
        let notrunc = conf.disallow_range_truncation;
        match datatype {
            AlphaNumType::Ascii => AnyAsciiLayout::try_new(columns, notrunc)
                .def_map_value(Self::Ascii)
                .def_errors_into()
                .def_map_warnings(|w| w.inner_into()),
            AlphaNumType::Integer => AnyOrderedUintLayout::try_new(columns, byteord, conf)
                .def_map_value(Self::Integer)
                .def_map_warnings(|e| e.inner_into())
                .def_inner_into(),
            AlphaNumType::Single => byteord.try_into().into_deferred().def_and_maybe(|b| {
                FixedLayout::try_new(columns, b, |c| {
                    F32Type::from_width_and_range(c.width, c.range, notrunc).def_warnings_into()
                })
                .def_map_value(Self::F32)
            }),
            AlphaNumType::Double => byteord.try_into().into_deferred().def_and_maybe(|b| {
                FixedLayout::try_new(columns, b, |c| {
                    F64Type::from_width_and_range(c.width, c.range, notrunc).def_warnings_into()
                })
                .def_map_value(Self::F64)
            }),
        }
    }

    pub(crate) fn phantom_into<X>(self) -> AnyOrderedLayout<X> {
        match_many_to_one!(self, Self, [Ascii, Integer, F32, F64], x, {
            x.phantom_into().into()
        })
    }

    pub fn into_unmixed<D>(self) -> LayoutConvertResult<NonMixedEndianLayout<D>> {
        match self {
            Self::Ascii(x) => Ok(x.phantom_into().into()),
            Self::Integer(x) => x.into_endian().map(NonMixedEndianLayout::Integer),
            Self::F32(x) => x.phantom_into().byte_layout_try_into().map(|y| y.into()),
            Self::F64(x) => x.phantom_into().byte_layout_try_into().map(|y| y.into()),
        }
        .into_mult()
    }

    pub(crate) fn into_3_1(self) -> LayoutConvertResult<DataLayout3_1> {
        self.into_unmixed().map(|x| x.into())
    }

    pub(crate) fn into_3_2(self) -> LayoutConvertResult<DataLayout3_2> {
        self.into_unmixed().map(DataLayout3_2::NonMixed)
    }
}

impl NonMixedEndianLayout<NoMeasDatatype> {
    fn lookup(
        kws: &mut StdKeywords,
        conf: &StdTextReadConfig,
        par: Par,
    ) -> LookupLayoutResult<Option<Self>> {
        let cs = NoMeasDatatype::lookup_all(kws, par);
        let d = AlphaNumType::lookup_req_check_ascii(kws);
        let n = ByteOrd3_1::lookup_req(kws);
        d.def_zip3(n, cs)
            .def_inner_into()
            .def_and_maybe(|(datatype, byteord, columns)| {
                def_transpose(
                    NonEmpty::from_vec(columns)
                        .map(|xs| Self::try_new(datatype, byteord.0, xs, conf)),
                )
                .def_inner_into()
            })
    }

    fn lookup_ro(kws: &StdKeywords, conf: &StdTextReadConfig) -> FromRawResult<Option<Self>> {
        let cs = NoMeasDatatype::lookup_ro_all(kws);
        let d = AlphaNumType::get_metaroot_req(kws).into_deferred();
        let n = ByteOrd3_1::get_metaroot_req(kws).into_deferred();
        d.def_zip3(n, cs)
            .def_inner_into()
            .def_and_maybe(|(datatype, byteord, columns)| {
                def_transpose(
                    NonEmpty::from_vec(columns)
                        .map(|xs| Self::try_new(datatype, byteord.0, xs, conf)),
                )
                .def_inner_into()
            })
    }

    fn try_new(
        datatype: AlphaNumType,
        endian: Endian,
        columns: NonEmpty<ColumnLayoutValues<NullMeasDatatype>>,
        conf: &StdTextReadConfig,
    ) -> DeferredResult<Self, ColumnError<NewMixedTypeWarning>, NewDataLayoutError> {
        let notrunc = conf.disallow_range_truncation;
        match datatype {
            AlphaNumType::Ascii => AnyAsciiLayout::try_new(columns, notrunc)
                .def_map_value(Self::Ascii)
                .def_errors_into()
                .def_map_warnings(|w| w.inner_into()),
            AlphaNumType::Integer => FixedLayout::endian_uint_try_new(columns, endian, notrunc)
                .def_map_value(Self::Integer)
                .def_map_warnings(|e| e.inner_into())
                .def_inner_into(),
            AlphaNumType::Single => FixedLayout::try_new(columns, endian, |c| {
                F32Type::from_width_and_range(c.width, c.range, notrunc).def_warnings_into()
            })
            .def_map_value(Self::F32),
            AlphaNumType::Double => FixedLayout::try_new(columns, endian, |c| {
                F64Type::from_width_and_range(c.width, c.range, notrunc).def_warnings_into()
            })
            .def_map_value(Self::F64),
        }
    }
}

impl<D> NonMixedEndianLayout<D> {
    pub fn new_ascii_fixed(ranges: NonEmpty<AsciiRange>) -> Self {
        AnyAsciiLayout::new_fixed(ranges).into()
    }

    pub fn new_ascii_delim(ranges: NonEmpty<u64>) -> Self {
        AnyAsciiLayout::new_delim(ranges).into()
    }

    pub fn new_uint(columns: NonEmpty<AnyNullBitmask>, endian: Endian) -> Self {
        FixedLayout::new(columns, endian).into()
    }

    pub fn new_f32(ranges: NonEmpty<F32Type>, endian: Endian) -> Self {
        FixedLayout::new(ranges, endian).into()
    }

    pub fn new_f64(ranges: NonEmpty<F64Type>, endian: Endian) -> Self {
        FixedLayout::new(ranges, endian).into()
    }

    pub(crate) fn into_ordered<T>(self) -> LayoutConvertResult<AnyOrderedLayout<T>> {
        match self {
            Self::Ascii(x) => Ok(x.phantom_into().into()),
            Self::Integer(x) => x.uint_try_into_ordered().map(|i| i.into()),
            Self::F32(x) => Ok(x.phantom_into().byte_layout_into().into()),
            Self::F64(x) => Ok(x.phantom_into().byte_layout_into().into()),
        }
    }

    pub(crate) fn phantom_into<D1>(self) -> NonMixedEndianLayout<D1> {
        match_many_to_one!(self, Self, [Ascii, Integer, F32, F64], x, {
            x.phantom_into().into()
        })
    }
}

#[derive(From, Display)]
pub enum AsciiToUintError {
    NotAscii(NotAsciiError),
    Int(ParseIntError),
}

pub struct NotAsciiError(Vec<u8>);

#[derive(From, Display)]
pub enum NewDataLayoutError {
    Ascii(ColumnError<ascii_range::NewAsciiRangeError>),
    FixedInt(NewFixedIntLayoutError),
    Float(ColumnError<FloatWidthError>),
    VariableInt(ColumnError<NewUintTypeError>),
    Mixed(ColumnError<NewMixedTypeError>),
    ByteOrd(ByteOrdToSizedError),
}

#[derive(From, Display)]
pub enum NewFixedIntLayoutError {
    Width(SingleFixedWidthError),
    Column(ColumnError<IntOrderedColumnError>),
}

#[derive(From, Display)]
pub enum IntOrderedColumnError {
    Order(ByteOrdToSizedError),
    // TODO sloppy nesting
    Endian(ByteOrdToSizedEndianError),
    Size(BitmaskError),
}

#[derive(From, Display)]
pub enum SingleFixedWidthError {
    Bytes(WidthToBytesError),
    Width(WidthMismatchError),
}

pub struct WidthMismatchError {
    byteord: ByteOrd2_0,
    found: NonEmpty<Bytes>,
}

#[derive(From, Display)]
pub enum NewMixedTypeError {
    Ascii(ascii_range::NewAsciiRangeError),
    Uint(NewUintTypeError),
    Float(FloatWidthError),
}

#[derive(From, Display)]
pub enum NewMixedTypeWarning {
    Ascii(IntRangeError<()>),
    Uint(BitmaskError),
    Float(FloatToDecimalError),
}

#[derive(From, Display)]
pub enum NewUintTypeError {
    Bitmask(BitmaskError),
    Bytes(WidthToBytesError),
}

#[derive(From, Display)]
pub enum FloatWidthError {
    Bytes(WidthToBytesError),
    WrongWidth(WrongFloatWidth),
    Range(FloatToDecimalError),
}

pub struct WrongFloatWidth {
    pub width: Bytes,
    pub expected: usize,
}

pub type DataReaderResult<T> = DeferredResult<T, NewDataReaderWarning, NewDataReaderError>;

#[derive(From, Display)]
pub enum NewDataReaderError {
    TotMismatch(TotEventMismatch),
    ParseTot(ReqKeyError<ParseIntError>),
    ParseSeg(ReqSegmentWithDefaultError<DataSegmentId>),
    Width(UnevenEventWidth),
    Mismatch(SegmentMismatchWarning<DataSegmentId>),
}

#[derive(From, Display)]
pub enum NewDataReaderWarning {
    TotMismatch(TotEventMismatch),
    ParseTot(ParseKeyError<ParseIntError>),
    Layout(ColumnError<IntRangeError<()>>),
    Width(UnevenEventWidth),
    Segment(ReqSegmentWithDefaultWarning<DataSegmentId>),
}

pub struct TotEventMismatch {
    tot: Tot,
    total_events: usize,
}

pub struct UnevenEventWidth {
    event_width: usize,
    nbytes: usize,
    remainder: usize,
}

#[derive(From, Display)]
pub enum AnyLossError {
    Int(LossError<BitmaskLossError>),
    Float(LossError<Infallible>),
    Ascii(LossError<AsciiLossError>),
}

pub struct AsciiLossError(Chars);

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

impl<E> ColumnError<E> {
    fn inner_into<X>(self) -> ColumnError<X>
    where
        X: From<E>,
    {
        ColumnError {
            index: self.index,
            error: self.error.into(),
        }
    }
}

type LookupLayoutResult<T> = DeferredResult<T, LookupLayoutWarning, LookupLayoutError>;

#[derive(From, Display)]
pub enum LookupLayoutError {
    New(NewDataLayoutError),
    Raw(LookupKeysError),
}

#[derive(From, Display)]
pub enum LookupLayoutWarning {
    New(ColumnError<NewMixedTypeWarning>),
    Raw(LookupKeysWarning),
}

type FromRawResult<T> = DeferredResult<T, RawToLayoutWarning, RawToLayoutError>;

#[derive(From, Display)]
pub enum RawToLayoutError {
    New(NewDataLayoutError),
    Raw(RawParsedError),
}

#[derive(From, Display)]
pub enum RawToLayoutWarning {
    New(ColumnError<NewMixedTypeWarning>),
    Raw(ParseKeyError<NumTypeError>),
}

#[derive(From, Display)]
pub enum RawParsedError {
    AlphaNumType(ReqKeyError<AlphaNumTypeError>),
    Endian(ReqKeyError<NewEndianError>),
    ByteOrd(ReqKeyError<ParseByteOrdError>),
    Int(ReqKeyError<ParseIntError>),
    Range(ReqKeyError<ParseBigDecimalError>),
}

#[derive(From, Display)]
pub enum ReadDataframeError {
    Ascii(ReadAsciiError),
    Uneven(UnevenEventWidth),
    TotMismatch(TotEventMismatch),
    Delim(ReadDelimWithRowsAsciiError),
    DelimNoRows(ReadDelimAsciiWithoutRowsError),
    AlphaNum(AsciiToUintError),
}

#[derive(From, Display)]
pub enum ReadAsciiError {
    Delim(ReadDelimAsciiError),
    Fixed(ReadFixedAsciiError),
}

#[derive(From, Display)]
pub enum ReadFixedAsciiError {
    Uneven(UnevenEventWidth),
    Tot(TotEventMismatch),
    ToUint(AsciiToUintError),
}

#[derive(From, Display)]
pub enum ReadDataframeWarning {
    Uneven(UnevenEventWidth),
    Tot(TotEventMismatch),
}

#[derive(From, Display)]
pub enum ReadDelimAsciiError {
    Rows(ReadDelimWithRowsAsciiError),
    NoRows(ReadDelimAsciiWithoutRowsError),
}

#[derive(From, Display)]
pub enum ReadDelimWithRowsAsciiError {
    RowsExceeded(RowsExceededError),
    Incomplete(DelimIncompleteError),
    Parse(AsciiToUintError),
}

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

impl fmt::Display for WidthMismatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        if self.found.tail.is_empty() {
            write!(
                f,
                "measurement width ({}) does not match byte order ({})",
                self.found.head, self.byteord,
            )
        } else {
            write!(
                f,
                "multiple measurement widths given ({}) for byte order [{}]",
                self.found.iter().join(", "),
                self.byteord,
            )
        }
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

pub type MixedToOrderedLayoutError = MixedColumnConvertError<MixedToOrderedConvertError>;
pub type MixedToNonMixedLayoutError = MixedColumnConvertError<MixedToInnerError>;

#[derive(From, Display)]
pub enum MixedToOrderedConvertError {
    Integer(MixedToOrderedUintError),
    Other(MixedToInnerError),
}

#[derive(From, Display)]
pub enum AnyRangeError {
    Ascii(IntRangeError<()>),
    Int(BitmaskError),
    Float(FloatToDecimalError),
}

pub struct MixedColumnConvertError<E> {
    index: MeasIndex,
    error: E,
}

impl<E: fmt::Display> fmt::Display for MixedColumnConvertError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "mixed conversion error in column {}: {}",
            self.index, self.error
        )
    }
}

#[derive(From, Display)]
pub enum MixedToOrderedUintError {
    IsWrongInteger(UintToUintError),
    Other(MixedToInnerError),
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

pub struct MixedToInnerError {
    dest_type: AlphaNumType,
    src: NullMixedType,
}

impl fmt::Display for MixedToInnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "could not convert mixed from {} to {}",
            self.src.as_alpha_num_type(),
            self.dest_type
        )
    }
}

// TODO this seems like it should live somewhere better
pub(crate) fn req_meas_headers() -> [MeasHeader; 2] {
    [Width::std_blank(), Range::std_blank()]
}
