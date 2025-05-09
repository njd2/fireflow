use crate::config::WriteConfig;
use crate::error::*;
use crate::macros::{match_many_to_one, newtype_from};
use crate::segment::*;
use crate::text::byteord::*;
use crate::text::core::*;
use crate::text::keywords::{AlphaNumType, Tot};
use crate::text::named_vec::MightHave;
use crate::text::optionalkw::*;
use crate::text::range::*;
use crate::validated::nonstandard::MeasIdx;
use crate::validated::shortname::*;

use itertools::Itertools;
use polars::prelude::*;
use std::fmt;
use std::io;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::iter;
use std::num::{IntErrorKind, ParseIntError};
use std::str::FromStr;

/// Represents the minimal data to fully describe one dataset in an FCS file.
///
/// This will include the standardized TEXT keywords as well as its
/// corresponding DATA segment parsed into a dataframe-like structure.
#[derive(Clone)]
pub struct CoreDataset<M, T, P, N, W> {
    /// Standardized TEXT segment in version specific format
    pub text: Box<CoreTEXT<M, T, P, N, W>>,

    /// DATA segment as a polars DataFrame
    ///
    /// The type of each column is such that each measurement is encoded with
    /// zero loss. This will/should never contain NULL values despite the
    /// underlying arrow framework allowing NULLs to exist.
    pub data: DataFrame,

    /// ANALYSIS segment
    ///
    /// This will be empty if ANALYSIS either doesn't exist or the computation
    /// fails. This has not standard structure, so the best we can capture is a
    /// byte sequence.
    pub analysis: Analysis,
}

pub(crate) type VersionedCoreDataset<M> = CoreDataset<
    M,
    <M as VersionedMetadata>::T,
    <M as VersionedMetadata>::P,
    <M as VersionedMetadata>::N,
    <<M as VersionedMetadata>::N as MightHave>::Wrapper<Shortname>,
>;

impl<M> VersionedCoreDataset<M>
where
    M: VersionedMetadata,
    M::N: Clone,
{
    /// Write this dataset (HEADER+TEXT+DATA+ANALYSIS) to a handle
    pub fn h_write<W>(&self, h: &mut BufWriter<W>, conf: &WriteConfig) -> ImpureResult<()>
    where
        W: Write,
    {
        let text = &self.text;
        let analysis_len = self.analysis.0.len();
        let df_ncols = self.data.width();

        // We can now confidently count the number of events (rows)
        let nrows = self.data.height();

        // Get the layout, or bail if we can't
        let layout = text.as_column_layout().map_err(|es| Failure {
            reason: "could not create data layout".to_string(),
            deferred: PureErrorBuf::from_many(es, PureErrorLevel::Error),
        })?;

        // Count number of measurements from layout. If the dataframe doesn't match
        // then something terrible happened and we need to escape through the
        // wormhole.
        let par = layout.ncols();
        if df_ncols != par {
            Err(Failure::new(format!(
                "datafame columns ({df_ncols}) unequal to number of measurements ({par})"
            )))?;
        }

        // Make common HEADER+TEXT writing function, for which the only unknown
        // now is the length of DATA.
        let write_text = |hh: &mut BufWriter<W>, data_len| -> ImpureResult<()> {
            if let Some(ts) = text.text_segment(Tot(nrows), data_len, analysis_len) {
                for t in ts {
                    hh.write_all(t.as_bytes())?;
                    hh.write_all(&[conf.delim.inner()])?;
                }
            } else {
                Err(Failure::new(
                    "primary TEXT does not fit into first 99,999,999 bytes".to_string(),
                ))?;
            }
            Ok(PureSuccess::from(()))
        };

        let res = if nrows == 0 {
            // Write HEADER+TEXT with no DATA if dataframe is empty. This assumes
            // the dataframe has the proper number of columns, but each column is
            // empty.
            write_text(h, 0)?;
            Ok(PureSuccess::from(()))
        } else {
            match layout {
                // For alphanumeric, only need to coerce the dataframe to the proper
                // types in each column and write these out bit-for-bit. User will
                // be warned if truncation happens.
                DataLayout::AlphaNum {
                    nrows: _,
                    columns: col_types,
                } => {
                    // ASSUME the dataframe will be coerced such that this
                    // relationship will hold true
                    let event_width: usize = col_types.iter().map(|c| c.width()).sum();
                    let data_len = event_width * nrows;
                    write_text(h, data_len)?;
                    h_write_numeric_dataframe(h, col_types, &self.data, conf)
                }

                // For delimited ASCII, need to first convert dataframe to u64 and
                // then figure out how much space this will take up based on a)
                // number of values in dataframe, and b) number of digits in each
                // value. Then convert values to strings and write byte
                // representation of strings. Fun...
                DataLayout::AsciiDelimited { nrows: _, ncols: _ } => {
                    if let Some(succ) = into_writable_matrix64(&self.data, conf) {
                        succ.try_map(|columns| {
                            let ndelim = df_ncols * nrows - 1;
                            // TODO cast?
                            let value_nbytes: u32 = columns
                                .iter()
                                .flat_map(|rows| {
                                    rows.iter().map(|x| x.checked_ilog10().unwrap_or(1))
                                })
                                .sum();
                            // compute data length (delimiters + number of digits)
                            let data_len = value_nbytes as usize + ndelim;
                            // write HEADER+TEXT
                            write_text(h, data_len)?;
                            // write DATA
                            h_write_delimited_matrix(h, nrows, columns)
                        })
                    } else {
                        // TODO lame...
                        Err(io::Error::other(
                            "could not get data from dataframe".to_string(),
                        ))?
                    }
                }
            }
        };

        h.write_all(&self.analysis.0)?;
        res
    }

    /// Convert this dataset into a different FCS version
    pub fn try_convert<ToM>(self) -> PureResult<VersionedCoreDataset<ToM>>
    where
        M::N: Clone,
        ToM: VersionedMetadata,
        ToM::P: VersionedMeasurement,
        ToM::T: VersionedTime,
        ToM::N: MightHave,
        ToM::N: Clone,
        ToM: TryFrom<M, Error = MetaConvertErrors>,
        ToM::P: TryFrom<M::P, Error = MeasConvertError>,
        ToM::T: From<M::T>,
        <ToM::N as MightHave>::Wrapper<Shortname>: TryFrom<<M::N as MightHave>::Wrapper<Shortname>>,
    {
        self.text.try_convert().map(|res| {
            res.map(|newtext| CoreDataset {
                text: Box::new(newtext),
                data: self.data,
                analysis: self.analysis,
            })
        })
    }

    // fn set_shortnames(&mut self, names: Vec<Shortname>) -> Result<NameMapping, String> {
    //     self.text
    //         .set_shortnames(names)
    //         .inspect(|_| self.text.set_df_column_names(&mut self.data).unwrap())
    // }

    // TODO also make a version of this that takes an index since not all
    // columns are named or we might not know the name
    fn remove_measurement(&mut self, n: &Shortname) -> Result<Option<MeasIdx>, String> {
        let i = self.text.remove_measurement_by_name(n)?;
        self.data.drop_in_place(n.as_ref()).unwrap();
        Ok(i.map(|x| x.0))
    }

    fn push_measurement<T>(
        &mut self,
        n: <M::N as MightHave>::Wrapper<Shortname>,
        m: Measurement<M::P>,
        col: Vec<T::Native>,
    ) -> Result<Shortname, String>
    where
        T: PolarsNumericType,
        ChunkedArray<T>: IntoSeries,
    {
        let k = self.text.push_measurement(n, m)?;
        let ser = ChunkedArray::<T>::from_vec(k.as_ref().into(), col).into_series();
        self.data.with_column(ser).map_err(|e| e.to_string())?;
        Ok(k)
    }

    fn insert_measurement<T>(
        &mut self,
        i: MeasIdx,
        n: <M::N as MightHave>::Wrapper<Shortname>,
        m: Measurement<M::P>,
        col: Vec<T::Native>,
    ) -> Result<Shortname, String>
    where
        T: PolarsNumericType,
        ChunkedArray<T>: IntoSeries,
    {
        let k = self.text.insert_measurement(i, n, m)?;
        let ser = ChunkedArray::<T>::from_vec(k.as_ref().into(), col).into_series();
        self.data
            .insert_column(i.into(), ser)
            .map_err(|e| e.to_string())?;
        Ok(k)
    }
}

// impl CoreDataset3_1 {
//     spillover_methods!(text);
// }

// impl CoreDataset3_2 {
//     spillover_methods!(text);
// }

/// The ANALYSIS segment, which is just a string of bytes
#[derive(Clone)]
pub struct Analysis(pub Vec<u8>);

newtype_from!(Analysis, Vec<u8>);

/// Read the analysis segment
pub(crate) fn h_read_analysis<R: Read + Seek>(
    h: &mut BufReader<R>,
    seg: &Segment,
) -> io::Result<Analysis> {
    let mut buf = vec![];
    h.seek(SeekFrom::Start(u64::from(seg.begin())))?;
    h.take(u64::from(seg.nbytes())).read_to_end(&mut buf)?;
    Ok(buf.into())
}

/// FCS file for any supported FCS version
#[derive(Clone)]
pub enum AnyCoreDataset {
    FCS2_0(CoreDataset2_0),
    FCS3_0(CoreDataset3_0),
    FCS3_1(CoreDataset3_1),
    FCS3_2(CoreDataset3_2),
}

impl AnyCoreDataset {
    pub fn as_data(&self) -> &DataFrame {
        match_many_to_one!(self, AnyCoreDataset, [FCS2_0, FCS3_0, FCS3_1, FCS3_2], x, {
            &x.data
        })
    }

    pub fn as_data_mut(&mut self) -> &mut DataFrame {
        match_many_to_one!(self, AnyCoreDataset, [FCS2_0, FCS3_0, FCS3_1, FCS3_2], x, {
            &mut x.data
        })
    }
}

/// The layout of the DATA segment
///
/// There are only two main configurations; delimited ASCII which is variable
/// width and fixed width which may contain ASCII or numeric bytes. In the
/// latter case, store the layout of each column, which may be a different
/// type (as given by the $DATATYPE/$PnDATATYPE keywords).
///
/// Generic parameter allows number of rows to also be encoded, which is
/// necessary for reading data. This is optional for the delimited case since
/// version 2.0 has $TOT as optional and it is impossible to back-calculate the
/// number of rows using $PnB in this case.
///
/// This will cover all possible data layouts, although not all possible layouts
/// will be valid for each version, so this is enforced elsewhere.
#[derive(Clone)]
pub(crate) enum DataLayout<T> {
    AsciiDelimited { nrows: Option<T>, ncols: usize },
    AlphaNum { nrows: T, columns: Vec<ColumnType> },
}

/// Data layout which includes columns and number of rows (used for reading)
pub(crate) type ColumnLayout = DataLayout<()>;

/// Data layout which includes only columns (used for writing)
pub(crate) type RowColumnLayout = DataLayout<Tot>;

/// The type of a non-delimited column in the DATA segment
#[derive(PartialEq, Clone)]
pub(crate) enum ColumnType {
    Ascii { chars: Chars },
    Integer(AnyUintType),
    Float(SingleType),
    Double(DoubleType),
}

/// An f32 column
type SingleType = FloatType<4, f32>;

/// An f64 column
type DoubleType = FloatType<8, f64>;

/// A floating point column (to be further constained)
#[derive(PartialEq, Clone)]
pub(crate) struct FloatType<const LEN: usize, T> {
    pub order: SizedByteOrd<LEN>,
    pub range: T,
}

/// An integer column of some size (1-8 bytes)
#[derive(PartialEq, Clone)]
pub(crate) enum AnyUintType {
    Uint08(Uint08Type),
    Uint16(Uint16Type),
    Uint24(Uint24Type),
    Uint32(Uint32Type),
    Uint40(Uint40Type),
    Uint48(Uint48Type),
    Uint56(Uint56Type),
    Uint64(Uint64Type),
}

type Uint08Type = UintType<u8, 1>;
type Uint16Type = UintType<u16, 2>;
type Uint24Type = UintType<u32, 3>;
type Uint32Type = UintType<u32, 4>;
type Uint40Type = UintType<u64, 5>;
type Uint48Type = UintType<u64, 6>;
type Uint56Type = UintType<u64, 7>;
type Uint64Type = UintType<u64, 8>;

/// A generic integer column type with a byte-layout and bitmask.
#[derive(PartialEq, Clone)]
pub struct UintType<T, const LEN: usize> {
    pub bitmask: T,
    pub size: SizedByteOrd<LEN>,
}

/// Instructions and buffers to read the DATA segment
pub(crate) struct DataReader {
    column_reader: ColumnReader,
    begin: u64,
}

/// Instructions to read one column in the DATA segment.
///
/// Each "column" contains a vector to hold the numbers read from DATA. In all
/// but the case of delimited ASCII, this is pre-allocated with the number of
/// rows to make reading faster. Each column has other information necessary to
/// read the column (bitmask, width, etc).
enum ColumnReader {
    DelimitedAscii(DelimAsciiReader),
    AlphaNum(AlphaNumReader),
    // TODO maybe resurrect these if it the mixed reader is too slow; it's prob
    // good enough
    // FixedWidthAscii(FixedAsciiReader),
    // Single(FloatReader<4>),
    // Double(FloatReader<8>),
    // Uint(UintReader),
}

struct AlphaNumReader {
    nrows: Tot,
    columns: Vec<AlphaNumColumnReader>,
}

enum AlphaNumColumnReader {
    Ascii(AsciiColumnReader),
    Uint(AnyUintColumnReader),
    Single(FloatColumnReader<f32, 4>),
    Double(FloatColumnReader<f64, 8>),
}

struct AsciiColumnReader {
    column: Vec<u64>,
    chars: Chars,
}

struct UintColumnReader<B, const LEN: usize> {
    layout: UintType<B, LEN>,
    column: Vec<B>,
}

enum AnyUintColumnReader {
    Uint08(UintColumnReader<u8, 1>),
    Uint16(UintColumnReader<u16, 2>),
    Uint24(UintColumnReader<u32, 3>),
    Uint32(UintColumnReader<u32, 4>),
    Uint40(UintColumnReader<u64, 5>),
    Uint48(UintColumnReader<u64, 6>),
    Uint56(UintColumnReader<u64, 7>),
    Uint64(UintColumnReader<u64, 8>),
}

struct FloatColumnReader<T, const LEN: usize> {
    column: Vec<T>,
    order: SizedByteOrd<LEN>,
}

struct DelimAsciiReader {
    ncols: usize,
    nrows: Option<Tot>,
    nbytes: usize,
}

// struct UintReader {
//     nrows: usize,
//     columns: Vec<AnyUintColumnReader>,
// }

// struct FixedAsciiReader {
//     widths: Vec<u8>,
//     nrows: Tot,
// }

// struct FloatReader<const LEN: usize> {
//     nrows: usize,
//     ncols: usize,
//     byteord: SizedByteOrd<LEN>,
// }

impl DelimAsciiReader {
    fn h_read<R: Read>(self, h: &mut BufReader<R>) -> io::Result<DataFrame> {
        let mut buf = Vec::new();
        let mut row = 0;
        let mut col = 0;
        let mut last_was_delim = false;
        // Delimiters are tab, newline, carriage return, space, or comma. Any
        // consecutive delimiter counts as one, and delimiters can be mixed.
        let is_delim = |byte| byte == 9 || byte == 10 || byte == 13 || byte == 32 || byte == 44;
        let mut data = if let Some(nrows) = self.nrows {
            // FCS 2.0 files have an optional $TOT field, which complicates this a
            // bit. If we know the number of rows, initialize a bunch of zero-ed
            // vectors and fill them sequentially.
            let mut data: Vec<_> = iter::repeat_with(|| vec![0; nrows.0])
                .take(self.ncols)
                .collect();
            for b in h.bytes().take(self.nbytes) {
                let byte = b?;
                // exit if we encounter more rows than expected.
                if row == nrows.0 {
                    let msg = format!("Exceeded expected number of rows: {nrows}");
                    return Err(io::Error::new(io::ErrorKind::InvalidData, msg));
                }
                if is_delim(byte) {
                    if !last_was_delim {
                        last_was_delim = true;
                        // TODO this will spaz out if we end up reading more
                        // rows than expected
                        data[col][row] = ascii_to_uint_io(buf.clone())?;
                        buf.clear();
                        if col == self.ncols - 1 {
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
            if !(col == 0 && row == nrows.0) {
                let msg = format!(
                    "Parsing ended in column {col} and row {row}, \
                               where expected number of rows is {nrows}"
                );
                return Err(io::Error::new(io::ErrorKind::InvalidData, msg));
            }
            data
        } else {
            // If we don't know the number of rows, the only choice is to push onto
            // the column vectors one at a time. This leads to the possibility that
            // the vectors may not be the same length in the end, in which case,
            // scream loudly and bail.
            let mut data: Vec<_> = iter::repeat_with(Vec::new).take(self.ncols).collect();
            for b in h.bytes().take(self.nbytes) {
                let byte = b?;
                // Delimiters are tab, newline, carriage return, space, or
                // comma. Any consecutive delimiter counts as one, and
                // delimiters can be mixed.
                if is_delim(byte) {
                    if !last_was_delim {
                        last_was_delim = true;
                        data[col].push(ascii_to_uint_io(buf.clone())?);
                        buf.clear();
                        if col == self.ncols - 1 {
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
                let msg = "Not all columns are equal length";
                return Err(io::Error::new(io::ErrorKind::InvalidData, msg));
            }
            data
        };
        // The spec isn't clear if the last value should be a delim or
        // not, so flush the buffer if it has anything in it since we
        // only try to parse if we hit a delim above.
        if !buf.is_empty() {
            data[col][row] = ascii_to_uint_io(buf.clone())?;
        }
        let ss: Vec<_> = data
            .into_iter()
            .enumerate()
            .map(|(i, s)| {
                ChunkedArray::<UInt64Type>::from_vec(format!("M{i}").into(), s)
                    .into_series()
                    .into()
            })
            .collect();
        DataFrame::new(ss).map_err(|e| io::Error::other(e.to_string()))
    }
}

impl AlphaNumReader {
    fn h_read<R: Read>(mut self, h: &mut BufReader<R>) -> io::Result<DataFrame> {
        let mut strbuf = String::new();
        for r in 0..self.nrows.0 {
            for c in self.columns.iter_mut() {
                match c {
                    AlphaNumColumnReader::Single(t) => Float32Type::h_read_to_column(h, t, r)?,
                    AlphaNumColumnReader::Double(t) => Float64Type::h_read_to_column(h, t, r)?,
                    AlphaNumColumnReader::Uint(u) => u.h_read_to_column(h, r)?,
                    AlphaNumColumnReader::Ascii(d) => {
                        strbuf.clear();
                        h.take(u64::from(u8::from(d.chars)))
                            .read_to_string(&mut strbuf)?;
                        d.column[r] = parse_u64_io(&strbuf)?;
                    }
                }
            }
        }
        // TODO get real column names here
        let ss: Vec<_> = self
            .columns
            .into_iter()
            .enumerate()
            .map(|(i, c)| c.into_pl_series(format!("X{i}").into()).into())
            .collect();
        DataFrame::new(ss).map_err(|e| io::Error::other(e.to_string()))
    }
}

/// Read the DATA segment and return a polars dataframe
pub(crate) fn h_read_data_segment<R: Read + Seek>(
    h: &mut BufReader<R>,
    parser: DataReader,
) -> io::Result<DataFrame> {
    h.seek(SeekFrom::Start(parser.begin))?;
    match parser.column_reader {
        ColumnReader::DelimitedAscii(p) => p.h_read(h),
        ColumnReader::AlphaNum(p) => p.h_read(h),
        // ColumnReader::FixedWidthAscii(p) => read_data_ascii_fixed(h, &p),
        // ColumnReader::Single(p) => Float32Type::read_matrix(h, p),
        // ColumnReader::Double(p) => Float64Type::read_matrix(h, p),
        // ColumnReader::Uint(p) => read_data_int(h, p),
    }
}

/// Instructions and data to write one column of the DATA segment
///
/// Each column contains a buffer with the data to be written (in the correct
/// type) and other type-specific information.
enum ColumnWriter {
    NumU8(NumColumnWriter<u8, 1>),
    NumU16(NumColumnWriter<u16, 2>),
    NumU24(NumColumnWriter<u32, 3>),
    NumU32(NumColumnWriter<u32, 4>),
    NumU40(NumColumnWriter<u64, 5>),
    NumU48(NumColumnWriter<u64, 6>),
    NumU56(NumColumnWriter<u64, 7>),
    NumU64(NumColumnWriter<u64, 8>),
    NumF32(NumColumnWriter<f32, 4>),
    NumF64(NumColumnWriter<f64, 8>),
    AsciiU8(AsciiColumnWriter<u8>),
    AsciiU16(AsciiColumnWriter<u16>),
    AsciiU32(AsciiColumnWriter<u32>),
    AsciiU64(AsciiColumnWriter<u64>),
}

use ColumnWriter::*;

struct NumColumnWriter<T, const LEN: usize> {
    data: Vec<T>,
    size: SizedByteOrd<LEN>,
}

struct AsciiColumnWriter<T> {
    data: Vec<T>,
    chars: Chars,
}

pub type CoreDataset2_0 = CoreDataset<
    InnerMetadata2_0,
    InnerTime2_0,
    InnerMeasurement2_0,
    OptionalKwFamily,
    OptionalKw<Shortname>,
>;
pub type CoreDataset3_0 = CoreDataset<
    InnerMetadata3_0,
    InnerTime3_0,
    InnerMeasurement3_0,
    OptionalKwFamily,
    OptionalKw<Shortname>,
>;
pub type CoreDataset3_1 = CoreDataset<
    InnerMetadata3_1,
    InnerTime3_1,
    InnerMeasurement3_1,
    IdentityFamily,
    Identity<Shortname>,
>;
pub type CoreDataset3_2 = CoreDataset<
    InnerMetadata3_2,
    InnerTime3_2,
    InnerMeasurement3_2,
    IdentityFamily,
    Identity<Shortname>,
>;

/// A valid Rust/Polars type which may be written to the DATA segment
enum ValidType {
    U08,
    U16,
    U32,
    U64,
    F32,
    F64,
}

macro_rules! series_cast {
    ($series:expr, $from:ident, $to:ty) => {
        $series
            .$from()
            .unwrap()
            .into_no_null_iter()
            .map(|x| x as $to)
            .collect()
    };
}

fn warn_bitmask<T: Ord + Copy>(xs: Vec<T>, deferred: &mut PureErrorBuf, bitmask: T) -> Vec<T> {
    let mut has_seen = false;
    xs.into_iter()
        .map(|x| {
            if x > bitmask && !has_seen {
                deferred.push_warning("bitmask exceed, value truncated".to_string());
                has_seen = true
            }
            x.min(bitmask)
        })
        .collect()
}

macro_rules! convert_to_uint1 {
    ($series:expr, $deferred:expr, $wrap:ident, $from:ident, $to:ty, $ut:expr) => {
        $wrap(NumColumnWriter {
            data: warn_bitmask(
                series_cast!($series, $from, $to),
                &mut $deferred,
                $ut.bitmask,
            ),
            size: $ut.size,
        })
    };
}

macro_rules! convert_to_uint {
    ($size:expr, $series:expr, $from:ident, $deferred:expr) => {
        match $size {
            AnyUintType::Uint08(ut) => {
                convert_to_uint1!($series, $deferred, NumU8, $from, u8, ut)
            }
            AnyUintType::Uint16(ut) => {
                convert_to_uint1!($series, $deferred, NumU16, $from, u16, ut)
            }
            AnyUintType::Uint24(ut) => {
                convert_to_uint1!($series, $deferred, NumU24, $from, u32, ut)
            }
            AnyUintType::Uint32(ut) => {
                convert_to_uint1!($series, $deferred, NumU32, $from, u32, ut)
            }
            AnyUintType::Uint40(ut) => {
                convert_to_uint1!($series, $deferred, NumU40, $from, u64, ut)
            }
            AnyUintType::Uint48(ut) => {
                convert_to_uint1!($series, $deferred, NumU48, $from, u64, ut)
            }
            AnyUintType::Uint56(ut) => {
                convert_to_uint1!($series, $deferred, NumU56, $from, u64, ut)
            }
            AnyUintType::Uint64(ut) => {
                convert_to_uint1!($series, $deferred, NumU64, $from, u64, ut)
            }
        }
    };
}

macro_rules! convert_to_float {
    ($size:expr, $series:expr, $wrap:ident, $from:ident, $to:ty) => {
        $wrap(NumColumnWriter {
            data: series_cast!($series, $from, $to),
            size: $size,
        })
    };
}

macro_rules! convert_to_f32 {
    ($size:expr, $series:expr, $from:ident) => {
        convert_to_float!($size, $series, NumF32, $from, f32)
    };
}

macro_rules! convert_to_f64 {
    ($size:expr, $series:expr, $from:ident) => {
        convert_to_float!($size, $series, NumF64, $from, f64)
    };
}

impl ColumnType {
    // TODO this in the number of literal bytes taken up by the column, use a
    // newtype wrapper for this
    pub(crate) fn width(&self) -> usize {
        match self {
            ColumnType::Ascii { chars } => usize::from(u8::from(*chars)),
            ColumnType::Integer(ut) => usize::from(ut.nbytes()),
            ColumnType::Float(_) => 4,
            ColumnType::Double(_) => 8,
        }
    }

    // fn datatype(&self) -> AlphaNumType {
    //     match self {
    //         ColumnType::Ascii { chars: _ } => AlphaNumType::Ascii,
    //         ColumnType::Integer(_) => AlphaNumType::Integer,
    //         ColumnType::Float(_) => AlphaNumType::Single,
    //         ColumnType::Double(_) => AlphaNumType::Double,
    //     }
    // }
}

impl AnyUintType {
    fn native_nbytes(&self) -> u8 {
        match self {
            AnyUintType::Uint08(_) => 1,
            AnyUintType::Uint16(_) => 2,
            AnyUintType::Uint24(_) => 4,
            AnyUintType::Uint32(_) => 4,
            AnyUintType::Uint40(_) => 8,
            AnyUintType::Uint48(_) => 8,
            AnyUintType::Uint56(_) => 8,
            AnyUintType::Uint64(_) => 8,
        }
    }

    fn nbytes(&self) -> u8 {
        match self {
            AnyUintType::Uint08(_) => 1,
            AnyUintType::Uint16(_) => 2,
            AnyUintType::Uint24(_) => 3,
            AnyUintType::Uint32(_) => 4,
            AnyUintType::Uint40(_) => 5,
            AnyUintType::Uint48(_) => 6,
            AnyUintType::Uint56(_) => 7,
            AnyUintType::Uint64(_) => 8,
        }
    }
}

impl RowColumnLayout {
    pub(crate) fn into_data_reader(self, data_seg: &Segment) -> DataReader {
        let column_parser = match self {
            DataLayout::AlphaNum { nrows, columns } => {
                ColumnReader::AlphaNum(make_mixed_reader(columns, nrows))
            }
            DataLayout::AsciiDelimited { nrows, ncols } => {
                let nbytes = data_seg.nbytes() as usize;
                ColumnReader::DelimitedAscii(DelimAsciiReader {
                    ncols,
                    nrows,
                    nbytes,
                })
            }
        };
        DataReader {
            column_reader: column_parser,
            begin: u64::from(data_seg.begin()),
        }
    }
}

fn make_mixed_reader(cs: Vec<ColumnType>, total_events: Tot) -> AlphaNumReader {
    let columns = cs
        .into_iter()
        .map(|p| match p {
            ColumnType::Ascii { chars } => AlphaNumColumnReader::Ascii(AsciiColumnReader {
                chars,
                column: vec![],
            }),
            ColumnType::Float(t) => {
                AlphaNumColumnReader::Single(Float32Type::column_reader(t.order, total_events))
            }
            ColumnType::Double(t) => {
                AlphaNumColumnReader::Double(Float64Type::column_reader(t.order, total_events))
            }
            ColumnType::Integer(col) => {
                AlphaNumColumnReader::Uint(AnyUintColumnReader::from_column(col, total_events))
            }
        })
        .collect();
    AlphaNumReader {
        columns,
        nrows: total_events,
    }
}

fn make_uint_type(b: BitsOrChars, r: &Range, o: &ByteOrd) -> Result<AnyUintType, Vec<String>> {
    if let Some(bytes) = b.bytes() {
        // ASSUME this can only be 1-8
        match u8::from(bytes) {
            1 => UInt8Type::column_type(r, o).map(AnyUintType::Uint08),
            2 => UInt16Type::column_type(r, o).map(AnyUintType::Uint16),
            3 => <UInt32Type as IntFromBytes<4, 3>>::column_type(r, o).map(AnyUintType::Uint24),
            4 => <UInt32Type as IntFromBytes<4, 4>>::column_type(r, o).map(AnyUintType::Uint32),
            5 => <UInt64Type as IntFromBytes<8, 5>>::column_type(r, o).map(AnyUintType::Uint40),
            6 => <UInt64Type as IntFromBytes<8, 6>>::column_type(r, o).map(AnyUintType::Uint48),
            7 => <UInt64Type as IntFromBytes<8, 7>>::column_type(r, o).map(AnyUintType::Uint56),
            8 => <UInt64Type as IntFromBytes<8, 8>>::column_type(r, o).map(AnyUintType::Uint64),
            _ => Err(vec!["make_uint_type: this should not happen".to_string()]),
        }
    } else {
        Err(vec!["$PnB is not an octet".to_string()])
    }
}

// hack to get bounds on error to work in IntMath trait
trait IntErr: Sized {
    fn err_kind(&self) -> &IntErrorKind;
}

impl IntErr for ParseIntError {
    fn err_kind(&self) -> &IntErrorKind {
        self.kind()
    }
}

trait IntMath: Sized
where
    Self: fmt::Display,
    Self: FromStr,
    <Self as FromStr>::Err: IntErr,
    <Self as FromStr>::Err: fmt::Display,
{
    fn next_power_2(x: Self) -> Self;

    fn int_from_str(s: &str) -> Result<Self, IntErrorKind> {
        s.parse()
            .map_err(|e| <Self as FromStr>::Err::err_kind(&e).clone())
    }

    fn maxval() -> Self;

    fn h_write_ascii_int<W: Write>(h: &mut BufWriter<W>, chars: Chars, x: Self) -> io::Result<()> {
        let s = x.to_string();
        // ASSUME bytes has been ensured to be able to hold the largest digit
        // expressible with this type, which means this will never be negative
        let w = u8::from(chars);
        let offset = usize::from(w) - s.len();
        let mut buf: Vec<u8> = vec![0, w];
        for (i, c) in s.bytes().enumerate() {
            buf[offset + i] = c;
        }
        h.write_all(&buf)
    }
}

trait NumProps<const DTLEN: usize>: Sized + Copy {
    fn zero() -> Self;

    fn from_big(buf: [u8; DTLEN]) -> Self;

    fn from_little(buf: [u8; DTLEN]) -> Self;

    fn to_big(self) -> [u8; DTLEN];

    fn to_little(self) -> [u8; DTLEN];
}

trait OrderedFromBytes<const DTLEN: usize, const OLEN: usize>: NumProps<DTLEN> {
    fn h_read_from_ordered<R: Read>(h: &mut BufReader<R>, order: &[u8; OLEN]) -> io::Result<Self> {
        let mut tmp = [0; OLEN];
        let mut buf = [0; DTLEN];
        h.read_exact(&mut tmp)?;
        for (i, j) in order.iter().enumerate() {
            buf[usize::from(*j)] = tmp[i];
        }
        Ok(Self::from_little(buf))
    }

    fn h_write_from_ordered<W: Write>(
        h: &mut BufWriter<W>,
        order: &[u8; OLEN],
        x: Self,
    ) -> io::Result<()> {
        let tmp = Self::to_little(x);
        let mut buf = [0; OLEN];
        for (i, j) in order.iter().enumerate() {
            buf[usize::from(*j)] = tmp[i];
        }
        h.write_all(&tmp)
    }
}

trait IntFromBytes<const DTLEN: usize, const INTLEN: usize>
where
    Self::Native: NumProps<DTLEN>,
    Self::Native: OrderedFromBytes<DTLEN, INTLEN>,
    Self::Native: TryFrom<u64>,
    Self::Native: IntMath,
    Self::Native: Ord,
    Self::Native: FromStr,
    <Self::Native as FromStr>::Err: fmt::Display,
    <Self::Native as FromStr>::Err: IntErr,
    Self::Native: FromStr,
    Self: PolarsNumericType,
    ChunkedArray<Self>: IntoSeries,
{
    fn range_to_bitmask(range: &Range) -> Result<Self::Native, String> {
        // TODO add way to control this behavior, we may not always want to
        // truncate an overflowing number, and at the very least may wish to
        // warn the user that truncation happened
        Self::Native::int_from_str(range.as_ref())
            .map(Self::Native::next_power_2)
            .or_else(|e| match e {
                IntErrorKind::PosOverflow => Ok(Self::Native::maxval()),
                _ => Err(format!("could not convert to u{INTLEN}")),
            })
    }

    fn column_type(
        range: &Range,
        byteord: &ByteOrd,
    ) -> Result<UintType<Self::Native, INTLEN>, Vec<String>> {
        // TODO be more specific, which means we need the measurement index
        let m = Self::range_to_bitmask(range);
        let s = byteord.as_sized();
        match (m, s) {
            (Ok(bitmask), Ok(size)) => Ok(UintType { bitmask, size }),
            (a, b) => Err([a.err(), b.err()].into_iter().flatten().collect()),
        }
    }

    fn h_read_int_masked<R: Read>(
        h: &mut BufReader<R>,
        byteord: &SizedByteOrd<INTLEN>,
        bitmask: Self::Native,
    ) -> io::Result<Self::Native> {
        Self::h_read_int(h, byteord).map(|x| x.min(bitmask))
    }

    fn h_read_int<R: Read>(
        h: &mut BufReader<R>,
        byteord: &SizedByteOrd<INTLEN>,
    ) -> io::Result<Self::Native> {
        // This lovely code will read data that is not a power-of-two
        // bytes long. Start by reading n bytes into a vector, which can
        // take a varying size. Then copy this into the power of 2 buffer
        // and reset all the unused cells to 0. This copy has to go to one
        // or the other end of the buffer depending on endianness.
        //
        // ASSUME for u8 and u16 that these will get heavily optimized away
        // since 'order' is totally meaningless for u8 and the only two possible
        // 'orders' for u16 are big and little.
        match byteord {
            SizedByteOrd::Endian(e) => {
                let mut tmp = [0; INTLEN];
                let mut buf = [0; DTLEN];
                h.read_exact(&mut tmp)?;
                Ok(if *e == Endian::Big {
                    let b = DTLEN - INTLEN;
                    buf[b..].copy_from_slice(&tmp[b..]);
                    Self::Native::from_big(buf)
                } else {
                    buf[..INTLEN].copy_from_slice(&tmp[..INTLEN]);
                    Self::Native::from_little(buf)
                })
            }
            SizedByteOrd::Order(order) => Self::Native::h_read_from_ordered(h, order),
        }
    }

    fn h_read_to_column<R: Read>(
        h: &mut BufReader<R>,
        d: &mut UintColumnReader<Self::Native, INTLEN>,
        row: usize,
    ) -> io::Result<()> {
        d.column[row] = Self::h_read_int_masked(h, &d.layout.size, d.layout.bitmask)?;
        Ok(())
    }

    fn h_write_int<W: Write>(
        h: &mut BufWriter<W>,
        byteord: &SizedByteOrd<INTLEN>,
        x: Self::Native,
    ) -> io::Result<()> {
        match byteord {
            SizedByteOrd::Endian(e) => {
                let mut buf = [0; INTLEN];
                let (start, end, tmp) = if *e == Endian::Big {
                    ((DTLEN - INTLEN), DTLEN, Self::Native::to_big(x))
                } else {
                    (0, INTLEN, Self::Native::to_little(x))
                };
                buf[..].copy_from_slice(&tmp[start..end]);
                h.write_all(&buf)
            }
            SizedByteOrd::Order(order) => Self::Native::h_write_from_ordered(h, order, x),
        }
    }
}

trait FloatFromBytes<const LEN: usize>
where
    Self::Native: NumProps<LEN>,
    Self::Native: OrderedFromBytes<LEN, LEN>,
    Self::Native: FromStr,
    <Self::Native as FromStr>::Err: fmt::Display,
    Self: Clone,
    Self: PolarsNumericType,
    ChunkedArray<Self>: IntoSeries,
{
    fn h_read_to_column<R: Read>(
        h: &mut BufReader<R>,
        column: &mut FloatColumnReader<Self::Native, LEN>,
        row: usize,
    ) -> io::Result<()> {
        column.column[row] = Self::h_read_float(h, &column.order)?;
        Ok(())
    }

    // /// Read byte sequence into a matrix of floats
    // fn read_matrix<R: Read>(h: &mut BufReader<R>, p: FloatReader<LEN>) -> io::Result<DataFrame> {
    //     let mut columns: Vec<_> = iter::repeat_with(|| vec![Self::Native::zero(); p.nrows])
    //         .take(p.ncols)
    //         .collect();
    //     for row in 0..p.nrows {
    //         for column in columns.iter_mut() {
    //             column[row] = Self::read_float(h, &p.byteord)?;
    //         }
    //     }
    //     let ss: Vec<_> = columns
    //         .into_iter()
    //         .enumerate()
    //         .map(|(i, s)| {
    //             ChunkedArray::<Self>::from_vec(format!("M{i}").into(), s)
    //                 .into_series()
    //                 .into()
    //         })
    //         .collect();
    //     DataFrame::new(ss).map_err(|e| io::Error::other(e.to_string()))
    //     // Ok(Dataframe::from(
    //     //     columns.into_iter().map(Vec::<Self>::into).collect(),
    //     // ))
    // }

    /// Make configuration to read one column of floats in a dataset.
    fn column_reader(
        order: SizedByteOrd<LEN>,
        total_events: Tot,
    ) -> FloatColumnReader<Self::Native, LEN> {
        FloatColumnReader {
            column: vec![Self::Native::zero(); total_events.0],
            order,
        }
    }

    fn column_type(o: &ByteOrd, r: &Range) -> Result<FloatType<LEN, Self::Native>, Vec<String>> {
        match (o.as_sized(), r.as_ref().parse::<Self::Native>()) {
            (Ok(order), Ok(range)) => Ok(FloatType { order, range }),
            (a, b) => Err([a.err(), b.err().map(|s| s.to_string())]
                .into_iter()
                .flatten()
                .collect()),
        }
    }

    // fn make_matrix_parser(
    //     byteord: &ByteOrd,
    //     par: usize,
    //     total_events: usize,
    // ) -> PureMaybe<FloatReader<LEN>> {
    //     let res = byteord.as_sized().map(|b| FloatReader {
    //         nrows: total_events,
    //         ncols: par,
    //         byteord: b,
    //     });
    //     PureMaybe::from_result_1(res, PureErrorLevel::Error)
    // }

    fn h_read_float<R: Read>(
        h: &mut BufReader<R>,
        byteord: &SizedByteOrd<LEN>,
    ) -> io::Result<Self::Native> {
        match byteord {
            SizedByteOrd::Endian(e) => {
                let mut buf = [0; LEN];
                h.read_exact(&mut buf)?;
                Ok(if *e == Endian::Big {
                    Self::Native::from_big(buf)
                } else {
                    Self::Native::from_little(buf)
                })
            }
            SizedByteOrd::Order(order) => Self::Native::h_read_from_ordered(h, order),
        }
    }

    fn h_write_float<W: Write>(
        h: &mut BufWriter<W>,
        byteord: &SizedByteOrd<LEN>,
        x: Self::Native,
    ) -> io::Result<()> {
        match byteord {
            SizedByteOrd::Endian(e) => {
                let buf: [u8; LEN] = if *e == Endian::Big {
                    Self::Native::to_big(x)
                } else {
                    Self::Native::to_little(x)
                };
                h.write_all(&buf)
            }
            SizedByteOrd::Order(order) => Self::Native::h_write_from_ordered(h, order, x),
        }
    }
}

/// Convert a series into a writable vector
///
/// Data that is to be written in a different type will be converted. Depending
/// on the start and end types, data loss may occur, in which case the user will
/// be warned.
///
/// For some cases like float->ASCII (bad idea), it is not clear how much space
/// will be needed to represent every possible float in the file, so user will
/// be warned always.
///
/// If the start type will fit into the end type, all is well and nothing bad
/// will happen to user's precious data.
fn series_coerce(
    c: &Column,
    w: ColumnType,
    conf: &WriteConfig,
) -> Option<PureSuccess<ColumnWriter>> {
    let dt = ValidType::from(c.dtype())?;
    let mut deferred = PureErrorBuf::default();

    let ascii_uint_warn = |d: &mut PureErrorBuf, bits, bytes| {
        let msg = format!(
            "writing ASCII as {bits}-bit uint in {bytes} \
                             may result in truncation"
        );
        d.push_msg_leveled(msg, conf.disallow_lossy_conversions);
    };
    let num_warn = |d: &mut PureErrorBuf, from, to| {
        let msg = format!("converting {from} to {to} may truncate data");
        d.push_msg_leveled(msg, conf.disallow_lossy_conversions);
    };

    // TODO this will make a copy of the data within a new vector, which is
    // simply going to be shoved onto disk a few nanoseconds later. Would make
    // more sense to return a lazy iterator which would skip this intermediate.
    let res = match w {
        // For Uint* -> ASCII, warn user if there are not enough bytes to
        // hold the max range of the type being formatted. ASCII shouldn't
        // store floats at all, so warn user if input data is float or
        // double.
        ColumnType::Ascii { chars } => match dt {
            ValidType::U08 => {
                if u8::from(chars) < 3 {
                    ascii_uint_warn(&mut deferred, 8, 3);
                }
                AsciiU8(AsciiColumnWriter {
                    data: c.u8().unwrap().into_no_null_iter().collect(),
                    chars,
                })
            }
            ValidType::U16 => {
                if u8::from(chars) < 5 {
                    ascii_uint_warn(&mut deferred, 16, 5);
                }
                AsciiU16(AsciiColumnWriter {
                    data: c.u16().unwrap().into_no_null_iter().collect(),
                    chars,
                })
            }
            ValidType::U32 => {
                if u8::from(chars) < 10 {
                    ascii_uint_warn(&mut deferred, 32, 10);
                }
                AsciiU32(AsciiColumnWriter {
                    data: c.u32().unwrap().into_no_null_iter().collect(),
                    chars,
                })
            }
            ValidType::U64 => {
                if u8::from(chars) < 20 {
                    ascii_uint_warn(&mut deferred, 64, 20);
                }
                AsciiU64(AsciiColumnWriter {
                    data: c.u64().unwrap().into_no_null_iter().collect(),
                    chars,
                })
            }
            ValidType::F32 => {
                num_warn(&mut deferred, "float", "uint64");
                AsciiU64(AsciiColumnWriter {
                    data: series_cast!(c, f32, u64),
                    chars,
                })
            }
            ValidType::F64 => {
                num_warn(&mut deferred, "double", "uint64");
                AsciiU64(AsciiColumnWriter {
                    data: series_cast!(c, f32, u64),
                    chars,
                })
            }
        },

        // Uint* -> Uint* is quite easy, just compare sizes and warn if the
        // target type is too small. Float/double -> Uint always could
        // potentially truncate a fractional value. Also check to see if
        // bitmask is exceeded, and if so truncate and warn user.
        ColumnType::Integer(ut) => {
            match dt {
                ValidType::F32 => num_warn(&mut deferred, "float", "uint"),
                ValidType::F64 => num_warn(&mut deferred, "float", "uint"),
                _ => {
                    let from_size = ut.nbytes();
                    let to_size = ut.native_nbytes();
                    if to_size < from_size {
                        let msg = format!(
                            "converted uint from {from_size} to \
                             {to_size} bytes may truncate data"
                        );
                        deferred.push_warning(msg);
                    }
                }
            }
            match dt {
                ValidType::U08 => convert_to_uint!(ut, c, u8, deferred),
                ValidType::U16 => convert_to_uint!(ut, c, u16, deferred),
                ValidType::U32 => convert_to_uint!(ut, c, u32, deferred),
                ValidType::U64 => convert_to_uint!(ut, c, u64, deferred),
                ValidType::F32 => convert_to_uint!(ut, c, f32, deferred),
                ValidType::F64 => convert_to_uint!(ut, c, f64, deferred),
            }
        }

        // Floats can hold small uints and themselves, anything else might
        // truncate.
        ColumnType::Float(t) => {
            match dt {
                ValidType::U32 => num_warn(&mut deferred, "float", "uint32"),
                ValidType::U64 => num_warn(&mut deferred, "float", "uint64"),
                ValidType::F64 => num_warn(&mut deferred, "float", "double"),
                _ => (),
            }
            match dt {
                ValidType::U08 => convert_to_f32!(t.order, c, u8),
                ValidType::U16 => convert_to_f32!(t.order, c, u16),
                ValidType::U32 => convert_to_f32!(t.order, c, u32),
                ValidType::U64 => convert_to_f32!(t.order, c, u64),
                ValidType::F32 => convert_to_f32!(t.order, c, f32),
                ValidType::F64 => convert_to_f32!(t.order, c, f64),
            }
        }

        // Doubles can hold all but uint64
        ColumnType::Double(t) => {
            if let ValidType::U64 = dt {
                num_warn(&mut deferred, "double", "uint64")
            }
            match dt {
                ValidType::U08 => convert_to_f64!(t.order, c, u8),
                ValidType::U16 => convert_to_f64!(t.order, c, u16),
                ValidType::U32 => convert_to_f64!(t.order, c, u32),
                ValidType::U64 => convert_to_f64!(t.order, c, u64),
                ValidType::F32 => convert_to_f64!(t.order, c, f32),
                ValidType::F64 => convert_to_f64!(t.order, c, f64),
            }
        }
    };
    Some(PureSuccess {
        data: res,
        deferred,
    })
}

/// Convert Series into a u64 vector.
///
/// Used when writing delimited ASCII. This is faster and more convenient
/// than the general coercion function.
fn series_coerce64(s: &Column, conf: &WriteConfig) -> Option<PureSuccess<Vec<u64>>> {
    let dt = ValidType::from(s.dtype())?;
    let mut deferred = PureErrorBuf::default();

    let num_warn = |d: &mut PureErrorBuf, from, to| {
        let msg = format!("converting {from} to {to} may truncate data");
        d.push_msg_leveled(msg, conf.disallow_lossy_conversions);
    };

    let res = match dt {
        ValidType::U08 => series_cast!(s, u8, u64),
        ValidType::U16 => series_cast!(s, u16, u64),
        ValidType::U32 => series_cast!(s, u32, u64),
        ValidType::U64 => series_cast!(s, u64, u64),
        ValidType::F32 => {
            num_warn(&mut deferred, "float", "uint64");
            series_cast!(s, f32, u64)
        }
        ValidType::F64 => {
            num_warn(&mut deferred, "double", "uint64");
            series_cast!(s, f64, u64)
        }
    };
    Some(PureSuccess {
        data: res,
        deferred,
    })
}

impl ValidType {
    fn from(dt: &DataType) -> Option<Self> {
        match dt {
            DataType::UInt8 => Some(ValidType::U08),
            DataType::UInt16 => Some(ValidType::U16),
            DataType::UInt32 => Some(ValidType::U32),
            DataType::UInt64 => Some(ValidType::U64),
            DataType::Float32 => Some(ValidType::F32),
            DataType::Float64 => Some(ValidType::F64),
            _ => None,
        }
    }
}

macro_rules! impl_num_props {
    ($size:expr, $zero:expr, $t:ty, $p:ident) => {
        impl NumProps<$size> for $t {
            fn zero() -> Self {
                $zero
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

impl_num_props!(1, 0, u8, U08);
impl_num_props!(2, 0, u16, U16);
impl_num_props!(4, 0, u32, U32);
impl_num_props!(8, 0, u64, U64);
impl_num_props!(4, 0.0, f32, F32);
impl_num_props!(8, 0.0, f64, F64);

macro_rules! impl_int_math {
    ($t:ty) => {
        impl IntMath for $t {
            // TODO this name is deceptive because it actually returns one less
            // the next power of 2
            fn next_power_2(x: Self) -> Self {
                Self::checked_next_power_of_two(x)
                    .map(|x| x - 1)
                    .unwrap_or(Self::MAX)
            }

            fn maxval() -> Self {
                Self::MAX
            }
        }
    };
}

impl_int_math!(u8);
impl_int_math!(u16);
impl_int_math!(u32);
impl_int_math!(u64);

impl OrderedFromBytes<1, 1> for u8 {}
impl OrderedFromBytes<2, 2> for u16 {}
impl OrderedFromBytes<4, 3> for u32 {}
impl OrderedFromBytes<4, 4> for u32 {}
impl OrderedFromBytes<8, 5> for u64 {}
impl OrderedFromBytes<8, 6> for u64 {}
impl OrderedFromBytes<8, 7> for u64 {}
impl OrderedFromBytes<8, 8> for u64 {}
impl OrderedFromBytes<4, 4> for f32 {}
impl OrderedFromBytes<8, 8> for f64 {}

impl FloatFromBytes<4> for Float32Type {}
impl FloatFromBytes<8> for Float64Type {}

impl IntFromBytes<1, 1> for UInt8Type {}
impl IntFromBytes<2, 2> for UInt16Type {}
impl IntFromBytes<4, 3> for UInt32Type {}
impl IntFromBytes<4, 4> for UInt32Type {}
impl IntFromBytes<8, 5> for UInt64Type {}
impl IntFromBytes<8, 6> for UInt64Type {}
impl IntFromBytes<8, 7> for UInt64Type {}
impl IntFromBytes<8, 8> for UInt64Type {}

impl AlphaNumColumnReader {
    fn into_pl_series(self, name: PlSmallStr) -> Series {
        match self {
            AlphaNumColumnReader::Ascii(x) => UInt64Chunked::from_vec(name, x.column).into_series(),
            AlphaNumColumnReader::Single(x) => {
                Float32Chunked::from_vec(name, x.column).into_series()
            }
            AlphaNumColumnReader::Double(x) => {
                Float64Chunked::from_vec(name, x.column).into_series()
            }
            AlphaNumColumnReader::Uint(x) => x.into_pl_series(name),
        }
    }
}

impl AnyUintColumnReader {
    fn into_pl_series(self, name: PlSmallStr) -> Series {
        match self {
            AnyUintColumnReader::Uint08(x) => UInt8Chunked::from_vec(name, x.column).into_series(),
            AnyUintColumnReader::Uint16(x) => UInt16Chunked::from_vec(name, x.column).into_series(),
            AnyUintColumnReader::Uint24(x) => UInt32Chunked::from_vec(name, x.column).into_series(),
            AnyUintColumnReader::Uint32(x) => UInt32Chunked::from_vec(name, x.column).into_series(),
            AnyUintColumnReader::Uint40(x) => UInt64Chunked::from_vec(name, x.column).into_series(),
            AnyUintColumnReader::Uint48(x) => UInt64Chunked::from_vec(name, x.column).into_series(),
            AnyUintColumnReader::Uint56(x) => UInt64Chunked::from_vec(name, x.column).into_series(),
            AnyUintColumnReader::Uint64(x) => UInt64Chunked::from_vec(name, x.column).into_series(),
        }
    }
}

macro_rules! uint_reader_from_column {
    ($x:ident, $t:expr, $($a:ident),+) => {
        match $x {
            $(
                AnyUintType::$a(layout) => AnyUintColumnReader::$a(UintColumnReader {
                    layout,
                    column: vec![0; $t],
                }),
            )+
        }
    };
}

impl AnyUintColumnReader {
    fn from_column(ut: AnyUintType, total_events: Tot) -> Self {
        let t = total_events.0;
        uint_reader_from_column!(
            ut, t, Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64
        )
    }

    fn h_read_to_column<R: Read>(&mut self, h: &mut BufReader<R>, r: usize) -> io::Result<()> {
        match self {
            AnyUintColumnReader::Uint08(d) => UInt8Type::h_read_to_column(h, d, r)?,
            AnyUintColumnReader::Uint16(d) => UInt16Type::h_read_to_column(h, d, r)?,
            AnyUintColumnReader::Uint24(d) => UInt32Type::h_read_to_column(h, d, r)?,
            AnyUintColumnReader::Uint32(d) => UInt32Type::h_read_to_column(h, d, r)?,
            AnyUintColumnReader::Uint40(d) => UInt64Type::h_read_to_column(h, d, r)?,
            AnyUintColumnReader::Uint48(d) => UInt64Type::h_read_to_column(h, d, r)?,
            AnyUintColumnReader::Uint56(d) => UInt64Type::h_read_to_column(h, d, r)?,
            AnyUintColumnReader::Uint64(d) => UInt64Type::h_read_to_column(h, d, r)?,
        }
        Ok(())
    }
}

fn into_writable_columns(
    df: &DataFrame,
    cs: Vec<ColumnType>,
    conf: &WriteConfig,
) -> Option<PureSuccess<Vec<ColumnWriter>>> {
    let cols = df.get_columns();
    let (writable_columns, msgs): (Vec<_>, Vec<_>) = cs
        .into_iter()
        // TODO do this without cloning?
        .zip(cols)
        .flat_map(|(w, c)| series_coerce(c, w, conf).map(|succ| (succ.data, succ.deferred)))
        .unzip();
    if df.width() != writable_columns.len() {
        return None;
    }
    Some(PureSuccess {
        data: writable_columns,
        deferred: PureErrorBuf::mconcat(msgs),
    })
}

fn into_writable_matrix64(
    df: &DataFrame,
    conf: &WriteConfig,
) -> Option<PureSuccess<Vec<Vec<u64>>>> {
    let (columns, msgs): (Vec<_>, Vec<_>) = df
        .get_columns()
        .iter()
        .flat_map(|c| {
            if let Some(res) = series_coerce64(c, conf) {
                Some((res.data, res.deferred))
            } else {
                None
            }
        })
        .unzip();
    if df.width() != columns.len() {
        return None;
    }
    Some(PureSuccess {
        data: columns,
        deferred: PureErrorBuf::mconcat(msgs),
    })
}

// fn read_data_ascii_fixed<R: Read>(
//     h: &mut BufReader<R>,
//     parser: &FixedAsciiReader,
// ) -> io::Result<DataFrame> {
//     let ncols = parser.widths.len();
//     let mut data: Vec<_> = iter::repeat_with(|| vec![0; parser.nrows.0])
//         .take(ncols)
//         .collect();
//     let mut buf = String::new();
//     for r in 0..parser.nrows.0 {
//         for (c, width) in parser.widths.iter().enumerate() {
//             buf.clear();
//             h.take(u64::from(*width)).read_to_string(&mut buf)?;
//             data[c][r] = parse_u64_io(&buf)?;
//         }
//     }
//     // TODO not DRY
//     let ss: Vec<_> = data
//         .into_iter()
//         .enumerate()
//         .map(|(i, s)| {
//             ChunkedArray::<UInt64Type>::from_vec(format!("M{i}").into(), s)
//                 .into_series()
//                 .into()
//         })
//         .collect();
//     DataFrame::new(ss).map_err(|e| io::Error::other(e.to_string()))
// }

// fn read_data_int<R: Read>(h: &mut BufReader<R>, parser: UintReader) -> io::Result<DataFrame> {
//     let mut p = parser;
//     for r in 0..p.nrows {
//         for c in p.columns.iter_mut() {
//             c.read_to_column(h, r)?;
//         }
//     }
//     let ss: Vec<_> = p
//         .columns
//         .into_iter()
//         .enumerate()
//         .map(|(i, c)| c.into_pl_series(format!("X{i}").into()).into())
//         .collect();
//     DataFrame::new(ss).map_err(|e| io::Error::other(e.to_string()))
// }

fn h_write_numeric_dataframe<W: Write>(
    h: &mut BufWriter<W>,
    cs: Vec<ColumnType>,
    df: &DataFrame,
    conf: &WriteConfig,
) -> ImpureResult<()> {
    let df_nrows = df.height();
    let res = into_writable_columns(df, cs, conf);
    if let Some(succ) = res {
        succ.try_map(|writable_columns| {
            for r in 0..df_nrows {
                for c in writable_columns.iter() {
                    match c {
                        NumU8(w) => UInt8Type::h_write_int(h, &w.size, w.data[r]),
                        NumU16(w) => UInt16Type::h_write_int(h, &w.size, w.data[r]),
                        NumU24(w) => UInt32Type::h_write_int(h, &w.size, w.data[r]),
                        NumU32(w) => UInt32Type::h_write_int(h, &w.size, w.data[r]),
                        NumU40(w) => UInt64Type::h_write_int(h, &w.size, w.data[r]),
                        NumU48(w) => UInt64Type::h_write_int(h, &w.size, w.data[r]),
                        NumU56(w) => UInt64Type::h_write_int(h, &w.size, w.data[r]),
                        NumU64(w) => UInt64Type::h_write_int(h, &w.size, w.data[r]),
                        NumF32(w) => Float32Type::h_write_float(h, &w.size, w.data[r]),
                        NumF64(w) => Float64Type::h_write_float(h, &w.size, w.data[r]),
                        AsciiU8(w) => u8::h_write_ascii_int(h, w.chars, w.data[r]),
                        AsciiU16(w) => u16::h_write_ascii_int(h, w.chars, w.data[r]),
                        AsciiU32(w) => u32::h_write_ascii_int(h, w.chars, w.data[r]),
                        AsciiU64(w) => u64::h_write_ascii_int(h, w.chars, w.data[r]),
                    }?
                }
            }
            Ok(PureSuccess::from(()))
        })
    } else {
        // TODO lame error message
        Err(io::Error::other(
            "could not get data from dataframe".to_string(),
        ))?
    }
}

fn h_write_delimited_matrix<W: Write>(
    h: &mut BufWriter<W>,
    nrows: usize,
    columns: Vec<Vec<u64>>,
) -> ImpureResult<()> {
    let ncols = columns.len();
    for ri in 0..nrows {
        for (ci, c) in columns.iter().enumerate() {
            let x = c[ri];
            // if zero, just write "0", if anything else convert
            // to a string and write that
            if x == 0 {
                h.write_all(&[48])?; // 48 = "0" in ASCII
            } else {
                let s = x.to_string();
                let t = s.trim_start_matches("0");
                let buf = t.as_bytes();
                h.write_all(buf)?;
            }
            // write delimiter after all but last value
            if !(ci == ncols - 1 && ri == nrows - 1) {
                h.write_all(&[32])?; // 32 = space in ASCII
            }
        }
    }
    Ok(PureSuccess::from(()))
}

impl<T> DataLayout<T> {
    fn ncols(&self) -> usize {
        match self {
            DataLayout::AsciiDelimited { nrows: _, ncols } => *ncols,
            DataLayout::AlphaNum { nrows: _, columns } => columns.len(),
        }
    }
}

// TODO also check scale here?
impl ColumnType {
    pub(crate) fn try_new(
        b: Width,
        dt: AlphaNumType,
        byteord: &ByteOrd,
        rng: &Range,
    ) -> Result<Option<Self>, Vec<String>> {
        match b {
            Width::Fixed(f) => match dt {
                AlphaNumType::Ascii => {
                    if let Some(chars) = f.chars() {
                        Ok(Self::Ascii { chars })
                    } else {
                        Err(vec![
                            "$DATATYPE=A but $PnB greater than 20 chars".to_string()
                        ])
                    }
                }
                AlphaNumType::Integer => make_uint_type(f, rng, byteord).map(Self::Integer),
                AlphaNumType::Single => {
                    if let Some(bytes) = f.bytes() {
                        if u8::from(bytes) == 4 {
                            Float32Type::column_type(byteord, rng).map(Self::Float)
                        } else {
                            Err(vec![format!("$DATATYPE=F but $PnB={}", f.inner())])
                        }
                    } else {
                        Err(vec![format!("$PnB is not an octet, got {}", f.inner())])
                    }
                }
                AlphaNumType::Double => {
                    if let Some(bytes) = f.bytes() {
                        if u8::from(bytes) == 8 {
                            Float64Type::column_type(byteord, rng).map(Self::Double)
                        } else {
                            Err(vec![format!("$DATATYPE=D but $PnB={}", f.inner())])
                        }
                    } else {
                        Err(vec![format!("$PnB is not an octet, got {}", f.inner())])
                    }
                }
            }
            .map(Some),
            Width::Variable => match dt {
                // ASSUME the only way this can happen is if $DATATYPE=A since
                // Ascii is not allowed in $PnDATATYPE.
                AlphaNumType::Ascii => Ok(None),
                _ => Err(vec![format!("variable $PnB not allowed for {dt}")]),
            },
        }
    }
}

fn ascii_to_uint_io(buf: Vec<u8>) -> io::Result<u64> {
    String::from_utf8(buf)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
        .and_then(|s| parse_u64_io(&s))
}

fn parse_u64_io(s: &str) -> io::Result<u64> {
    s.parse::<u64>()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
