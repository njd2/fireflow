use crate::config::WriteConfig;
use crate::core::*;
use crate::error::*;
use crate::macros::match_many_to_one;
use crate::segment::*;
use crate::text::byteord::*;
use crate::text::keywords::*;
use crate::text::optionalkw::*;
use crate::text::range::*;
use crate::validated::dataframe::*;
use crate::validated::shortname::*;

use itertools::Itertools;
use std::fmt;
use std::io;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::iter;
use std::num::{IntErrorKind, ParseIntError};
use std::str::FromStr;

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

pub trait VersionedDataLayout: Sized {
    type S;
    type D;

    fn try_new(
        dt: AlphaNumType,
        size: Self::S,
        cs: Vec<ColumnLayoutData<Self::D>>,
    ) -> Result<Self, Vec<String>>;

    fn try_new_from_raw(kws: &StdKeywords) -> Result<Self, Vec<String>>;

    fn ncols(&self) -> usize;

    fn into_reader(self, kws: &StdKeywords, data_seg: Segment) -> PureMaybe<ColumnReader>;

    fn as_writer<'a>(
        &self,
        df: &'a FCSDataFrame,
        conf: &WriteConfig,
    ) -> PureResult<DataWriter<'a>> {
        // The dataframe should be encapsulated such that a) the column number
        // matches the number of measurements. If these are not true, the code
        // is wrong.
        let par = self.ncols();
        let ncols = df.ncols();
        if ncols != par {
            panic!("datafame columns ({ncols}) unequal to number of measurements ({par})");
        }

        let res = self.as_writer_inner(df, conf);
        let level = if conf.disallow_lossy_conversions {
            PureErrorLevel::Error
        } else {
            PureErrorLevel::Warning
        };
        PureMaybe::from_result_strs(res, level).into_result("could not make data layout".into())
    }

    fn as_writer_inner<'a>(
        &self,
        df: &'a FCSDataFrame,
        conf: &WriteConfig,
    ) -> Result<DataWriter<'a>, Vec<String>>;
}

pub enum DataLayout2_0 {
    Ascii(AsciiLayout),
    Integer(AnyUintLayout),
    Float(FloatLayout),
}

pub enum DataLayout3_0 {
    Ascii(AsciiLayout),
    Integer(AnyUintLayout),
    Float(FloatLayout),
}

pub enum DataLayout3_1 {
    Ascii(AsciiLayout),
    Integer(FixedLayout<AnyUintType>),
    Float(FloatLayout),
}

pub enum DataLayout3_2 {
    // TODO we could just use delimited and mixed for this
    Ascii(AsciiLayout),
    Integer(FixedLayout<AnyUintType>),
    Float(FloatLayout),
    Mixed(FixedLayout<MixedType>),
}

pub enum AsciiLayout {
    Delimited(DelimitedLayout),
    Fixed(FixedLayout<AsciiType>),
}

pub enum FloatLayout {
    F32(FixedLayout<F32Type>),
    F64(FixedLayout<F64Type>),
}

pub struct DelimitedLayout {
    pub ncols: usize,
}

pub struct FixedLayout<C> {
    pub columns: Vec<C>,
}

pub enum AnyUintLayout {
    Uint08(FixedLayout<Uint08Type>),
    Uint16(FixedLayout<Uint16Type>),
    Uint24(FixedLayout<Uint24Type>),
    Uint32(FixedLayout<Uint32Type>),
    Uint40(FixedLayout<Uint40Type>),
    Uint48(FixedLayout<Uint48Type>),
    Uint56(FixedLayout<Uint56Type>),
    Uint64(FixedLayout<Uint64Type>),
}

/// The type of a non-delimited column in the DATA segment
#[derive(PartialEq, Clone, Copy)]
pub enum MixedType {
    Ascii(AsciiType),
    Integer(AnyUintType),
    Float(F32Type),
    Double(F64Type),
}

#[derive(PartialEq, Clone, Copy)]
pub struct AsciiType {
    pub(crate) chars: Chars,
}

impl From<AsciiType> for MixedType {
    fn from(value: AsciiType) -> Self {
        MixedType::Ascii(value)
    }
}

/// An f32 column
type F32Type = FloatType<4, f32>;

/// An f64 column
type F64Type = FloatType<8, f64>;

impl From<F32Type> for MixedType {
    fn from(value: F32Type) -> Self {
        MixedType::Float(value)
    }
}

impl From<F64Type> for MixedType {
    fn from(value: F64Type) -> Self {
        MixedType::Double(value)
    }
}

/// A floating point column (to be further constained)
#[derive(PartialEq, Clone, Copy)]
pub struct FloatType<const LEN: usize, T> {
    pub order: SizedByteOrd<LEN>,
    // TODO why is this here?
    pub range: T,
}

/// An integer column of some size (1-8 bytes)
#[derive(PartialEq, Clone, Copy)]
pub enum AnyUintType {
    Uint08(Uint08Type),
    Uint16(Uint16Type),
    Uint24(Uint24Type),
    Uint32(Uint32Type),
    Uint40(Uint40Type),
    Uint48(Uint48Type),
    Uint56(Uint56Type),
    Uint64(Uint64Type),
}

impl From<AnyUintType> for MixedType {
    fn from(value: AnyUintType) -> Self {
        MixedType::Integer(value)
    }
}

macro_rules! uint_to_mixed {
    ($uint:ident, $wrap:ident) => {
        impl From<$uint> for MixedType {
            fn from(value: $uint) -> Self {
                MixedType::Integer(AnyUintType::$wrap(value))
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

type Uint08Type = UintType<u8, 1>;
type Uint16Type = UintType<u16, 2>;
type Uint24Type = UintType<u32, 3>;
type Uint32Type = UintType<u32, 4>;
type Uint40Type = UintType<u64, 5>;
type Uint48Type = UintType<u64, 6>;
type Uint56Type = UintType<u64, 7>;
type Uint64Type = UintType<u64, 8>;

/// A generic integer column type with a byte-layout and bitmask.
#[derive(PartialEq, Clone, Copy)]
pub struct UintType<T, const LEN: usize> {
    pub bitmask: T,
    pub byteord: SizedByteOrd<LEN>,
}

/// Instructions for writing measurements to a file.
///
/// This structure can be used with all FCS versions, as each column is treated
/// as it's own separate type. This is in contrast to some FCS versions where
/// we could think of the DATA as a matrix of one uniform type, which doesn't
/// work well with polars which is heavily optimized for column operations.
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

pub type DelimWriter<'a> = DataWriterInner<AnyDelimColumnWriter<'a>>;
pub type FixedWriter<'a> = DataWriterInner<AnyFixedColumnWriter<'a>>;

pub struct DataWriterInner<C> {
    columns: Vec<C>,
    nrows: usize,
    nbytes: usize,
}

/// Writer for ASCII delimited layout.
///
/// Each variant represents a type in the original column, which is necessary
/// due to the use of lazy iterators for each type the original column may hold.
///
/// The type within the variant will only convert to a u64 without any size
/// information.
pub enum AnyDelimColumnWriter<'a> {
    FromU08(DelimColumnWriter<'a, u8>),
    FromU16(DelimColumnWriter<'a, u16>),
    FromU32(DelimColumnWriter<'a, u32>),
    FromU64(DelimColumnWriter<'a, u64>),
    FromF32(DelimColumnWriter<'a, f32>),
    FromF64(DelimColumnWriter<'a, f64>),
}

/// Writer for any fixed numeric/ascii layout
///
/// Each variant represents a type in the original column, which is necessary
/// due to the use of lazy iterators for each type the original column may hold.
///
/// The type within each variant can hold any target representation for a
/// measurement, which includes type and size information.
pub enum AnyFixedColumnWriter<'a> {
    FromU08(AnyColumnWriter<'a, u8>),
    FromU16(AnyColumnWriter<'a, u16>),
    FromU32(AnyColumnWriter<'a, u32>),
    FromU64(AnyColumnWriter<'a, u64>),
    FromF32(AnyColumnWriter<'a, f32>),
    FromF64(AnyColumnWriter<'a, f64>),
}

/// Writer for one column.
///
/// This encodes the target type for encoding any FCS measurement, which may be
/// integers of varying sizes, floats, doubles, or a string of numbers. Each
/// type has its own size information. The generic type encodes the source type
/// within the polars column from which the written type will be converted,
/// possibly with data loss.
pub enum AnyColumnWriter<'a, X> {
    U08(IntColumnWriter<'a, X, u8, 1>),
    U16(IntColumnWriter<'a, X, u16, 2>),
    U24(IntColumnWriter<'a, X, u32, 3>),
    U32(IntColumnWriter<'a, X, u32, 4>),
    U40(IntColumnWriter<'a, X, u64, 5>),
    U48(IntColumnWriter<'a, X, u64, 6>),
    U56(IntColumnWriter<'a, X, u64, 7>),
    U64(IntColumnWriter<'a, X, u64, 8>),
    F32(FloatColumnWriter<'a, X, f32, 4>),
    F64(FloatColumnWriter<'a, X, f64, 8>),
    Ascii(AsciiColumnWriter<'a, X>),
}

pub type IntColumnWriter<'a, X, T, const LEN: usize> = ColumnWriter<'a, X, T, UintType<T, LEN>>;

pub type FloatColumnWriter<'a, X, T, const LEN: usize> = ColumnWriter<'a, X, T, SizedByteOrd<LEN>>;

pub type AsciiColumnWriter<'a, X> = ColumnWriter<'a, X, u64, Chars>;

pub type DelimColumnWriter<'a, X> = ColumnWriter<'a, X, u64, ()>;

pub struct ColumnWriter<'a, X, Y, S> {
    pub(crate) data: FCSColIter<'a, X, Y>,
    pub(crate) size: S,
}

impl DataWriter<'_> {
    pub(crate) fn h_write<W: Write>(&mut self, h: &mut BufWriter<W>) -> io::Result<()> {
        match self {
            DataWriter::Delim(d) => d.h_write(h),
            DataWriter::Fixed(f) => f.h_write(h),
        }
    }

    pub(crate) fn nbytes(&self) -> usize {
        match self {
            DataWriter::Delim(d) => d.nbytes,
            DataWriter::Fixed(f) => f.nbytes,
        }
    }
}

impl DelimWriter<'_> {
    fn h_write<W: Write>(&mut self, h: &mut BufWriter<W>) -> io::Result<()> {
        let ncols = self.columns.len();
        let nrows = self.nrows;
        for i in 0..nrows {
            for (j, c) in self.columns.iter_mut().enumerate() {
                c.h_write(h)?;
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

impl AnyDelimColumnWriter<'_> {
    fn h_write<W: Write>(&mut self, h: &mut BufWriter<W>) -> io::Result<()> {
        match_many_to_one!(
            self,
            AnyDelimColumnWriter,
            [FromU08, FromU16, FromU32, FromU64, FromF32, FromF64],
            c,
            { c.h_write_delim_ascii(h) }
        )
    }
}

impl AnyFixedColumnWriter<'_> {
    fn h_write<W: Write>(&mut self, h: &mut BufWriter<W>) -> io::Result<()> {
        match_many_to_one!(
            self,
            AnyFixedColumnWriter,
            [FromU08, FromU16, FromU32, FromU64, FromF32, FromF64],
            c,
            { c.h_write(h) }
        )
    }
}

impl<X> AnyColumnWriter<'_, X> {
    fn h_write<W: Write>(&mut self, h: &mut BufWriter<W>) -> io::Result<()>
    where
        X: Copy,
    {
        match self {
            AnyColumnWriter::U08(c) => c.h_write_int(h),
            AnyColumnWriter::U16(c) => c.h_write_int(h),
            AnyColumnWriter::U24(c) => c.h_write_int(h),
            AnyColumnWriter::U32(c) => c.h_write_int(h),
            AnyColumnWriter::U40(c) => c.h_write_int(h),
            AnyColumnWriter::U48(c) => c.h_write_int(h),
            AnyColumnWriter::U56(c) => c.h_write_int(h),
            AnyColumnWriter::U64(c) => c.h_write_int(h),
            AnyColumnWriter::F32(c) => c.h_write_float(h),
            AnyColumnWriter::F64(c) => c.h_write_float(h),
            AnyColumnWriter::Ascii(c) => c.h_write_ascii(h),
        }
    }
}

#[derive(Default)]
pub enum LossError {
    #[default]
    DataType,
    Chars,
    Bitmask,
}

impl fmt::Display for LossError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        let s = match self {
            // TODO this error is basically meaningless
            LossError::DataType => "loss occurred when converting to different type",
            LossError::Chars => "loss occurred when truncating ASCII string to required width",
            LossError::Bitmask => "loss occurred when applying bitmask to integer",
        };
        write!(f, "{}", s)
    }
}

impl<X, Y, const INTLEN: usize> IntColumnWriter<'_, X, Y, INTLEN> {
    fn h_write_int<W: Write, const DTLEN: usize>(&mut self, h: &mut BufWriter<W>) -> io::Result<()>
    where
        X: Copy,
        Y: IntFromBytes<DTLEN, INTLEN>,
        <Y as FromStr>::Err: fmt::Display,
        // <Y as FromStr>::Err: IntErr,
        Y: Ord,
    {
        let x = self.data.next().unwrap();
        x.new
            .min(self.size.bitmask)
            .h_write_int(h, &self.size.byteord)
    }
}

impl<X, Y, const DTLEN: usize> FloatColumnWriter<'_, X, Y, DTLEN> {
    fn h_write_float<W: Write>(&mut self, h: &mut BufWriter<W>) -> io::Result<()>
    where
        X: Copy,
        Y: FloatFromBytes<DTLEN>,
        <Y as FromStr>::Err: fmt::Display,
    {
        self.data.next().unwrap().new.h_write_float(h, &self.size)
    }
}

impl<X> AsciiColumnWriter<'_, X> {
    fn h_write_ascii<W: Write>(&mut self, h: &mut BufWriter<W>) -> io::Result<()>
    where
        X: Copy,
    {
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

impl<X> DelimColumnWriter<'_, X> {
    fn h_write_delim_ascii<W: Write>(&mut self, h: &mut BufWriter<W>) -> io::Result<()>
    where
        X: Copy,
    {
        let x = self.data.next().unwrap();
        let s = x.new.to_string();
        let buf = s.as_bytes();
        h.write_all(buf)
    }
}

/// Instructions and buffers to read the DATA segment
pub(crate) struct DataReader {
    pub(crate) column_reader: ColumnReader,
    pub(crate) begin: u64,
}

/// Instructions to read one column in the DATA segment.
///
/// Each "column" contains a vector to hold the numbers read from DATA. In all
/// but the case of delimited ASCII, this is pre-allocated with the number of
/// rows to make reading faster. Each column has other information necessary to
/// read the column (bitmask, width, etc).
pub enum ColumnReader {
    DelimitedAscii(DelimAsciiReader),
    AlphaNum(AlphaNumReader),
}

pub struct DelimAsciiReader {
    pub ncols: usize,
    pub nrows: Option<Tot>,
    pub nbytes: usize,
}

pub struct AlphaNumReader {
    pub nrows: Tot,
    pub columns: Vec<AlphaNumColumnReader>,
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
    pub size: SizedByteOrd<LEN>,
}

pub struct AsciiColumnReader {
    pub column: Vec<u64>,
    pub width: Chars,
}

pub struct UintColumnReader<B, const LEN: usize> {
    pub column: Vec<B>,
    pub uint_type: UintType<B, LEN>,
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
    // TODO don't take ownership
    pub(crate) fn h_read<R>(self, h: &mut BufReader<R>) -> io::Result<FCSDataFrame>
    where
        R: Read + Seek,
    {
        h.seek(SeekFrom::Start(self.begin))?;
        match self.column_reader {
            ColumnReader::DelimitedAscii(p) => p.h_read(h),
            ColumnReader::AlphaNum(p) => p.h_read(h),
        }
    }
}

impl FloatReader {
    fn h_read<R: Read>(&mut self, h: &mut BufReader<R>, r: usize) -> io::Result<()> {
        match self {
            FloatReader::F32(t) => t.h_read(h, r),
            FloatReader::F64(t) => t.h_read(h, r),
        }
    }

    fn into_fcs_column(self) -> AnyFCSColumn {
        match self {
            FloatReader::F32(x) => F32Column::from(x.column).into(),
            FloatReader::F64(x) => F64Column::from(x.column).into(),
        }
    }
}

impl DelimAsciiReader {
    fn h_read<R: Read>(self, h: &mut BufReader<R>) -> io::Result<FCSDataFrame> {
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

impl AlphaNumReader {
    fn h_read<R: Read>(mut self, h: &mut BufReader<R>) -> io::Result<FCSDataFrame> {
        let mut strbuf = String::new();
        for r in 0..self.nrows.0 {
            for c in self.columns.iter_mut() {
                match c {
                    AlphaNumColumnReader::Float(f) => f.h_read(h, r)?,
                    AlphaNumColumnReader::Uint(u) => u.h_read_to_column(h, r)?,
                    AlphaNumColumnReader::Ascii(d) => {
                        strbuf.clear();
                        h.take(u64::from(u8::from(d.width)))
                            .read_to_string(&mut strbuf)?;
                        d.column[r] = parse_u64_io(&strbuf)?;
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
}

fn make_uint_type_inner(bytes: Bytes, r: &Range, e: Endian) -> Result<AnyUintType, String> {
    // ASSUME this can only be 1-8
    match u8::from(bytes) {
        1 => u8::column_type_endian(r, e).map(AnyUintType::Uint08),
        2 => u16::column_type_endian(r, e).map(AnyUintType::Uint16),
        3 => <u32 as IntFromBytes<4, 3>>::column_type_endian(r, e).map(AnyUintType::Uint24),
        4 => <u32 as IntFromBytes<4, 4>>::column_type_endian(r, e).map(AnyUintType::Uint32),
        5 => <u64 as IntFromBytes<8, 5>>::column_type_endian(r, e).map(AnyUintType::Uint40),
        6 => <u64 as IntFromBytes<8, 6>>::column_type_endian(r, e).map(AnyUintType::Uint48),
        7 => <u64 as IntFromBytes<8, 7>>::column_type_endian(r, e).map(AnyUintType::Uint56),
        8 => <u64 as IntFromBytes<8, 8>>::column_type_endian(r, e).map(AnyUintType::Uint64),
        _ => unreachable!(),
    }
}

fn make_uint_type(b: BitsOrChars, r: &Range, e: Endian) -> Result<AnyUintType, String> {
    if let Some(bytes) = b.bytes() {
        make_uint_type_inner(bytes, r, e)
    } else {
        Err("$PnB is not an octet".to_string())
    }
}

impl FixedLayout<AnyUintType> {
    pub(crate) fn try_new<D>(cs: Vec<ColumnLayoutData<D>>, o: Endian) -> Result<Self, Vec<String>> {
        let ncols = cs.len();
        let (ws, rs): (Vec<_>, Vec<_>) = cs.into_iter().map(|c| (c.width, c.range)).unzip();
        let fixed: Vec<_> = ws.into_iter().flat_map(|w: Width| w.as_fixed()).collect();
        if fixed.len() < ncols {
            return Err(vec!["not all fixed width".into()]);
        }
        let bytes: Vec<_> = fixed.into_iter().flat_map(|f| f.bytes()).collect();
        if bytes.len() < ncols {
            return Err(vec!["bad stuff".into()]);
        }
        let (columns, fail): (Vec<_>, Vec<_>) = bytes
            .into_iter()
            .zip(rs)
            .map(|(b, r)| make_uint_type_inner(b, &r, o))
            .partition_result();
        if fail.is_empty() {
            Ok(FixedLayout { columns })
        } else {
            Err(fail)
        }
    }
}

// // hack to get bounds on error to work in IntMath trait
// trait IntErr: Sized {
//     fn err_kind(&self) -> &IntErrorKind;
// }

// impl IntErr for ParseIntError {
//     fn err_kind(&self) -> &IntErrorKind {
//         self.kind()
//     }
// }

trait IntMath: Sized
where
    Self: fmt::Display,
    Self: FromStr,
{
    fn next_power_2(x: Self) -> Self;

    // fn int_from_str(s: &str) -> Result<Self, IntErrorKind> {
    //     s.parse()
    //         .map_err(|e| <Self as FromStr>::Err::err_kind(&e).clone())
    // }
}

// TODO clean this up with https://github.com/rust-lang/rust/issues/76560 once
// it lands in a stable compiler, in theory there is no reason to put the length
// of the type as a parameter, but the current compiler is not smart enough
trait NumProps<const DTLEN: usize>: Sized + Copy + Default {
    fn from_big(buf: [u8; DTLEN]) -> Self;

    fn from_little(buf: [u8; DTLEN]) -> Self;

    fn to_big(self) -> [u8; DTLEN];

    fn to_little(self) -> [u8; DTLEN];

    fn maxval() -> Self;
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
        self,
        h: &mut BufWriter<W>,
        order: &[u8; OLEN],
    ) -> io::Result<()> {
        let tmp = Self::to_little(self);
        let mut buf = [0; OLEN];
        for (i, j) in order.iter().enumerate() {
            buf[usize::from(*j)] = tmp[i];
        }
        h.write_all(&tmp)
    }
}

trait IntFromBytes<const DTLEN: usize, const INTLEN: usize>
where
    Self: OrderedFromBytes<DTLEN, INTLEN>,
    Self: TryFrom<Range, Error = RangeToIntError<Self>>,
    Self: IntMath,
    <Self as FromStr>::Err: fmt::Display,
{
    fn range_to_bitmask(range: &Range) -> Result<Self, String> {
        // TODO add way to control this behavior, we may not always want to
        // truncate an overflowing number, and at the very least may wish to
        // warn the user that truncation happened
        (*range).try_into().or_else(|e| match e {
            RangeToIntError::IntOverrange(_) => Ok(Self::maxval()),
            RangeToIntError::FloatOverrange(_) => Ok(Self::maxval()),
            RangeToIntError::FloatUnderrange(_) => Ok(Self::default()),
            RangeToIntError::FloatPrecisionLoss(_, x) => Ok(x),
        })
    }

    fn column_type_endian(range: &Range, endian: Endian) -> Result<UintType<Self, INTLEN>, String> {
        // TODO be more specific, which means we need the measurement index
        Self::range_to_bitmask(range).map(|bitmask| UintType {
            bitmask,
            byteord: endian.into(),
        })
    }

    fn column_type(
        range: &Range,
        byteord: &ByteOrd,
        // index: MeasIdx,
    ) -> Result<UintType<Self, INTLEN>, Vec<String>> {
        // TODO be more specific, which means we need the measurement index
        let m = Self::range_to_bitmask(range);
        let s = byteord.as_sized();
        match (m, s) {
            (Ok(bitmask), Ok(size)) => Ok(UintType {
                bitmask,
                byteord: size,
            }),
            (a, b) => Err([a.err(), b.err()].into_iter().flatten().collect()),
        }
    }

    fn layout(
        rs: Vec<Range>,
        byteord: &ByteOrd,
    ) -> Result<FixedLayout<UintType<Self, INTLEN>>, Vec<String>> {
        let (columns, fail): (Vec<_>, Vec<_>) = rs
            .into_iter()
            .map(|r| Self::column_type(&r, byteord))
            .partition_result();
        if fail.is_empty() {
            Ok(FixedLayout { columns })
        } else {
            Err(fail.into_iter().flatten().collect())
        }
    }

    fn h_read_int<R: Read>(
        h: &mut BufReader<R>,
        byteord: &SizedByteOrd<INTLEN>,
    ) -> io::Result<Self> {
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
                    Self::from_big(buf)
                } else {
                    buf[..INTLEN].copy_from_slice(&tmp[..INTLEN]);
                    Self::from_little(buf)
                })
            }
            SizedByteOrd::Order(order) => Self::h_read_from_ordered(h, order),
        }
    }

    fn h_write_int<W: Write>(
        self,
        h: &mut BufWriter<W>,
        byteord: &SizedByteOrd<INTLEN>,
    ) -> io::Result<()> {
        match byteord {
            SizedByteOrd::Endian(e) => {
                let mut buf = [0; INTLEN];
                let (start, end, tmp) = if *e == Endian::Big {
                    ((DTLEN - INTLEN), DTLEN, Self::to_big(self))
                } else {
                    (0, INTLEN, Self::to_little(self))
                };
                buf[..].copy_from_slice(&tmp[start..end]);
                h.write_all(&buf)
            }
            SizedByteOrd::Order(order) => self.h_write_from_ordered(h, order),
        }
    }
}

trait FloatFromBytes<const LEN: usize>
where
    Self: NumProps<LEN>,
    Self: OrderedFromBytes<LEN, LEN>,
    Self: FromStr,
    Self: TryFrom<Range, Error = RangeToFloatError<Self>>,
    <Self as FromStr>::Err: fmt::Display,
    Self: Clone,
{
    fn range(r: &Range) -> Self {
        // TODO control how this works and/or warn user if we truncate
        (*r).try_into().unwrap_or_else(|e| match e {
            RangeToFloatError::IntPrecisionLoss(_, x) => x,
            RangeToFloatError::FloatOverrange(_) => Self::maxval(),
            RangeToFloatError::FloatUnderrange(_) => Self::default(),
        })
    }

    fn column_type_endian(o: Endian, r: &Range) -> FloatType<LEN, Self> {
        let range = Self::range(r);
        FloatType {
            order: o.into(),
            range,
        }
    }

    fn column_type(o: &ByteOrd, r: &Range) -> Result<FloatType<LEN, Self>, String> {
        let range = Self::range(r);
        o.as_sized()
            .map_err(|e| e.to_string())
            .map(|order| FloatType { order, range })
    }

    fn layout_endian<D>(
        cs: Vec<ColumnLayoutData<D>>,
        endian: Endian,
    ) -> Result<FixedLayout<FloatType<LEN, Self>>, Vec<String>> {
        let (pass, fail): (Vec<_>, Vec<_>) = cs
            .into_iter()
            .map(|c| {
                if c.width
                    .as_fixed()
                    .and_then(|f| f.bytes())
                    .is_some_and(|b| u8::from(b) == 4)
                {
                    Ok(Self::column_type_endian(endian, &c.range))
                } else {
                    // TODO which one?
                    Err(format!("$PnB is not {} bytes wide", 4))
                }
            })
            .partition_result();
        if fail.is_empty() {
            Ok(FixedLayout { columns: pass })
        } else {
            Err(fail)
        }
    }

    fn layout<D>(
        cs: Vec<ColumnLayoutData<D>>,
        byteord: &ByteOrd,
    ) -> Result<FixedLayout<FloatType<LEN, Self>>, Vec<String>> {
        let (pass, fail): (Vec<_>, Vec<_>) = cs
            .into_iter()
            .map(|c| {
                if c.width
                    .as_fixed()
                    .and_then(|f| f.bytes())
                    .is_some_and(|b| u8::from(b) == 4)
                {
                    Self::column_type(byteord, &c.range)
                } else {
                    // TODO which one?
                    Err(format!("$PnB is not {} bytes wide", 4))
                }
            })
            .partition_result();
        if fail.is_empty() {
            Ok(FixedLayout { columns: pass })
        } else {
            Err(fail.into_iter().collect())
        }
    }

    fn h_read_float<R: Read>(
        h: &mut BufReader<R>,
        byteord: &SizedByteOrd<LEN>,
    ) -> io::Result<Self> {
        match byteord {
            SizedByteOrd::Endian(e) => {
                let mut buf = [0; LEN];
                h.read_exact(&mut buf)?;
                Ok(if *e == Endian::Big {
                    Self::from_big(buf)
                } else {
                    Self::from_little(buf)
                })
            }
            SizedByteOrd::Order(order) => Self::h_read_from_ordered(h, order),
        }
    }

    fn h_write_float<W: Write>(
        self,
        h: &mut BufWriter<W>,
        byteord: &SizedByteOrd<LEN>,
    ) -> io::Result<()> {
        match byteord {
            SizedByteOrd::Endian(e) => {
                let buf: [u8; LEN] = if *e == Endian::Big {
                    Self::to_big(self)
                } else {
                    Self::to_little(self)
                };
                h.write_all(&buf)
            }
            SizedByteOrd::Order(order) => self.h_write_from_ordered(h, order),
        }
    }
}

macro_rules! impl_num_props {
    ($size:expr, $t:ty) => {
        impl NumProps<$size> for $t {
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
            // TODO this name is deceptive because it actually returns one less
            // the next power of 2
            fn next_power_2(x: Self) -> Self {
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

impl FloatFromBytes<4> for f32 {}
impl FloatFromBytes<8> for f64 {}

impl IntFromBytes<1, 1> for u8 {}
impl IntFromBytes<2, 2> for u16 {}
impl IntFromBytes<4, 3> for u32 {}
impl IntFromBytes<4, 4> for u32 {}
impl IntFromBytes<8, 5> for u64 {}
impl IntFromBytes<8, 6> for u64 {}
impl IntFromBytes<8, 7> for u64 {}
impl IntFromBytes<8, 8> for u64 {}

impl AlphaNumColumnReader {
    fn into_fcs_column(self) -> AnyFCSColumn {
        match self {
            AlphaNumColumnReader::Ascii(x) => U64Column::from(x.column).into(),
            AlphaNumColumnReader::Float(x) => x.into_fcs_column(),
            AlphaNumColumnReader::Uint(x) => x.into_fcs_column(),
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
}

macro_rules! uint_reader_from_column {
    ($x:ident, $t:expr, $($a:ident),+) => {
        match $x {
            $(
                AnyUintType::$a(uint_type) => AnyUintColumnReader::$a(UintColumnReader {
                    uint_type,
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
impl MixedType {
    pub(crate) fn try_new(
        b: Width,
        dt: AlphaNumType,
        // TODO this should only take an endian since it only applies to 3.2
        // and has no chance in hogwarts of being in anything earlier
        endian: Endian,
        rng: &Range,
    ) -> Result<Option<Self>, String> {
        match b {
            Width::Fixed(f) => match dt {
                AlphaNumType::Ascii => {
                    if let Some(chars) = f.chars() {
                        Ok(Self::Ascii(AsciiType { chars }))
                    } else {
                        Err("$DATATYPE=A but $PnB greater than 20 chars".to_string())
                    }
                }
                AlphaNumType::Integer => make_uint_type(f, rng, endian).map(Self::Integer),
                AlphaNumType::Single => {
                    if let Some(bytes) = f.bytes() {
                        if u8::from(bytes) == 4 {
                            Ok(Self::Float(f32::column_type_endian(endian, rng)))
                        } else {
                            Err(format!("$DATATYPE=F but $PnB={}", f.inner()))
                        }
                    } else {
                        Err(format!("$PnB is not an octet, got {}", f.inner()))
                    }
                }
                AlphaNumType::Double => {
                    if let Some(bytes) = f.bytes() {
                        if u8::from(bytes) == 8 {
                            Ok(Self::Double(f64::column_type_endian(endian, rng)))
                        } else {
                            Err(format!("$DATATYPE=D but $PnB={}", f.inner()))
                        }
                    } else {
                        Err(format!("$PnB is not an octet, got {}", f.inner()))
                    }
                }
            }
            .map(Some),
            Width::Variable => match dt {
                // ASSUME the only way this can happen is if $DATATYPE=A since
                // Ascii is not allowed in $PnDATATYPE.
                AlphaNumType::Ascii => Ok(None),
                _ => Err(format!("variable $PnB not allowed for {dt}")),
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

pub struct ColumnLayoutData<D> {
    pub width: Width,
    pub range: Range,
    pub datatype: D,
}

impl<C> FixedLayout<C>
where
    C: IsFixed,
{
    fn event_width(&self) -> usize {
        self.columns.iter().map(|c| c.width()).sum()
    }

    fn ncols(&self) -> usize {
        self.columns.len()
    }

    pub fn into_reader(self, seg: Segment, kw_tot: Option<Tot>) -> PureSuccess<AlphaNumReader> {
        let n = seg.nbytes() as usize;
        let w = self.event_width();
        let total_events = n / w;
        let remainder = n % w;
        let mut deferred = PureErrorBuf::default();
        if let Some(t) = kw_tot {
            if t.0 != total_events {
                let msg = format!(
                    "$TOT field is {t} but number of events \
                     that evenly fit into DATA is {total_events}",
                );
                // TODO toggle
                deferred.push_warning(msg);
            }
        }
        if remainder > 0 {
            let msg = format!(
                "Events are {w} bytes wide, but this does not evenly \
                     divide DATA segment which is {n} bytes long \
                     (remainder of {remainder})",
            );
            // TODO toggle
            deferred.push_warning(msg);
        }
        let columns = self
            .columns
            .into_iter()
            .map(|c| c.into_reader(total_events))
            .collect();
        let r = AlphaNumReader {
            columns,
            nrows: Tot(total_events),
        };
        PureSuccess { data: r, deferred }
    }

    fn as_writer<'a>(
        &self,
        df: &'a FCSDataFrame,
        conf: &WriteConfig,
    ) -> Result<FixedWriter<'a>, Vec<String>>
    where
        MixedType: From<C>,
        C: Copy,
    {
        let check = conf.check_conversion;
        let (pass, fail): (Vec<_>, Vec<_>) = self
            .columns
            .iter()
            .map(|c| (*c).into())
            .zip(df.iter_columns())
            .enumerate()
            .map(|(i, (t, c)): (usize, (MixedType, &AnyFCSColumn))| {
                t.into_writer(c, check).map_err(|e| (i, e))
            })
            .partition_result();
        let nbytes = self.event_width() * df.nrows();
        if fail.is_empty() {
            Ok(FixedWriter {
                columns: pass,
                nrows: df.nrows(),
                nbytes,
            })
        } else {
            Err(fail
                .into_iter()
                .map(|(i, e)| format!("Conversion error in column {i}: {e}"))
                .collect())
        }
    }
}

pub trait IsFixed {
    fn width(&self) -> usize;

    fn into_reader(self, nrows: usize) -> AlphaNumColumnReader;

    fn into_writer(self, c: &AnyFCSColumn, check: bool) -> Result<AnyFixedColumnWriter, LossError>;
}

impl<T, const LEN: usize> IsFixed for UintType<T, LEN>
where
    T: Clone,
    T: Copy,
    T: Default,
    T: Ord,
    AlphaNumColumnReader: From<UintColumnReader<T, LEN>>,
    T: NumCast<u8>,
    T: NumCast<u16>,
    T: NumCast<u32>,
    T: NumCast<u64>,
    T: NumCast<f32>,
    T: NumCast<f64>,
    for<'b> AnyFixedColumnWriter<'b>: From<IntColumnWriter<'b, u8, T, LEN>>,
    for<'b> AnyFixedColumnWriter<'b>: From<IntColumnWriter<'b, u16, T, LEN>>,
    for<'b> AnyFixedColumnWriter<'b>: From<IntColumnWriter<'b, u32, T, LEN>>,
    for<'b> AnyFixedColumnWriter<'b>: From<IntColumnWriter<'b, u64, T, LEN>>,
    for<'b> AnyFixedColumnWriter<'b>: From<IntColumnWriter<'b, f32, T, LEN>>,
    for<'b> AnyFixedColumnWriter<'b>: From<IntColumnWriter<'b, f64, T, LEN>>,
{
    fn width(&self) -> usize {
        LEN
    }

    fn into_reader(self, nrows: usize) -> AlphaNumColumnReader {
        UintColumnReader {
            column: vec![T::default(); nrows],
            uint_type: self,
        }
        .into()
    }

    fn into_writer(self, c: &AnyFCSColumn, check: bool) -> Result<AnyFixedColumnWriter, LossError> {
        let bitmask = self.bitmask;
        match_many_to_one!(c, AnyFCSColumn, [U08, U16, U32, U64, F32, F64], xs, {
            FCSDataType::into_writer(xs, self, check, |x: T| {
                if x > bitmask {
                    Some(LossError::Bitmask)
                } else {
                    None
                }
            })
            .map(|w| w.into())
        })
    }
}

impl IsFixed for AnyUintType {
    fn width(&self) -> usize {
        match_many_to_one!(
            self,
            AnyUintType,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            x,
            { (*x).width() }
        )
    }

    fn into_reader(self, nrows: usize) -> AlphaNumColumnReader {
        match_many_to_one!(
            self,
            AnyUintType,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            x,
            { x.into_reader(nrows) }
        )
    }

    fn into_writer(self, c: &AnyFCSColumn, check: bool) -> Result<AnyFixedColumnWriter, LossError> {
        match_many_to_one!(
            self,
            AnyUintType,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            x,
            { x.into_writer(c, check) }
        )
    }
}

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

macro_rules! uint_from_writer {
    ($fromtype:ident, $totype:ident, $len:expr, $fromwrap:ident, $wrap:ident) => {
        impl<'a> From<IntColumnWriter<'a, $fromtype, $totype, $len>> for AnyFixedColumnWriter<'a> {
            fn from(value: IntColumnWriter<'a, $fromtype, $totype, $len>) -> Self {
                AnyFixedColumnWriter::$fromwrap(AnyColumnWriter::$wrap(value))
            }
        }
    };
}

// TODO ...
uint_from_writer!(u8, u8, 1, FromU08, U08);
uint_from_writer!(u8, u16, 2, FromU08, U16);
uint_from_writer!(u8, u32, 3, FromU08, U24);
uint_from_writer!(u8, u32, 4, FromU08, U32);
uint_from_writer!(u8, u64, 5, FromU08, U40);
uint_from_writer!(u8, u64, 6, FromU08, U48);
uint_from_writer!(u8, u64, 7, FromU08, U56);
uint_from_writer!(u8, u64, 8, FromU08, U64);

uint_from_writer!(u16, u8, 1, FromU16, U08);
uint_from_writer!(u16, u16, 2, FromU16, U16);
uint_from_writer!(u16, u32, 3, FromU16, U24);
uint_from_writer!(u16, u32, 4, FromU16, U32);
uint_from_writer!(u16, u64, 5, FromU16, U40);
uint_from_writer!(u16, u64, 6, FromU16, U48);
uint_from_writer!(u16, u64, 7, FromU16, U56);
uint_from_writer!(u16, u64, 8, FromU16, U64);

uint_from_writer!(u32, u8, 1, FromU32, U08);
uint_from_writer!(u32, u16, 2, FromU32, U16);
uint_from_writer!(u32, u32, 3, FromU32, U24);
uint_from_writer!(u32, u32, 4, FromU32, U32);
uint_from_writer!(u32, u64, 5, FromU32, U40);
uint_from_writer!(u32, u64, 6, FromU32, U48);
uint_from_writer!(u32, u64, 7, FromU32, U56);
uint_from_writer!(u32, u64, 8, FromU32, U64);

uint_from_writer!(u64, u8, 1, FromU64, U08);
uint_from_writer!(u64, u16, 2, FromU64, U16);
uint_from_writer!(u64, u32, 3, FromU64, U24);
uint_from_writer!(u64, u32, 4, FromU64, U32);
uint_from_writer!(u64, u64, 5, FromU64, U40);
uint_from_writer!(u64, u64, 6, FromU64, U48);
uint_from_writer!(u64, u64, 7, FromU64, U56);
uint_from_writer!(u64, u64, 8, FromU64, U64);

uint_from_writer!(f32, u8, 1, FromF32, U08);
uint_from_writer!(f32, u16, 2, FromF32, U16);
uint_from_writer!(f32, u32, 3, FromF32, U24);
uint_from_writer!(f32, u32, 4, FromF32, U32);
uint_from_writer!(f32, u64, 5, FromF32, U40);
uint_from_writer!(f32, u64, 6, FromF32, U48);
uint_from_writer!(f32, u64, 7, FromF32, U56);
uint_from_writer!(f32, u64, 8, FromF32, U64);

uint_from_writer!(f64, u8, 1, FromF64, U08);
uint_from_writer!(f64, u16, 2, FromF64, U16);
uint_from_writer!(f64, u32, 3, FromF64, U24);
uint_from_writer!(f64, u32, 4, FromF64, U32);
uint_from_writer!(f64, u64, 5, FromF64, U40);
uint_from_writer!(f64, u64, 6, FromF64, U48);
uint_from_writer!(f64, u64, 7, FromF64, U56);
uint_from_writer!(f64, u64, 8, FromF64, U64);

macro_rules! float_from_writer {
    ($fromtype:ident, $totype:ident, $len:expr, $fromwrap:ident, $wrap:ident) => {
        impl<'a> From<FloatColumnWriter<'a, $fromtype, $totype, $len>>
            for AnyFixedColumnWriter<'a>
        {
            fn from(value: FloatColumnWriter<'a, $fromtype, $totype, $len>) -> Self {
                AnyFixedColumnWriter::$fromwrap(AnyColumnWriter::$wrap(value))
            }
        }
    };
}

// TODO ...
float_from_writer!(u8, f32, 4, FromU08, F32);
float_from_writer!(u8, f64, 8, FromU08, F64);

float_from_writer!(u16, f32, 4, FromU16, F32);
float_from_writer!(u16, f64, 8, FromU16, F64);

float_from_writer!(u32, f32, 4, FromU32, F32);
float_from_writer!(u32, f64, 8, FromU32, F64);

float_from_writer!(u64, f32, 4, FromU64, F32);
float_from_writer!(u64, f64, 8, FromU64, F64);

float_from_writer!(f32, f32, 4, FromF32, F32);
float_from_writer!(f32, f64, 8, FromF32, F64);

float_from_writer!(f64, f32, 4, FromF64, F32);
float_from_writer!(f64, f64, 8, FromF64, F64);

// TODO flip args to make more consistent
impl<T, const LEN: usize> IsFixed for FloatType<LEN, T>
where
    T: Clone,
    T: Default,
    AlphaNumColumnReader: From<FloatColumnReader<T, LEN>>,
    T: NumCast<u8>,
    T: NumCast<u16>,
    T: NumCast<u32>,
    T: NumCast<u64>,
    T: NumCast<f32>,
    T: NumCast<f64>,
    for<'b> AnyFixedColumnWriter<'b>: From<FloatColumnWriter<'b, u8, T, LEN>>,
    for<'b> AnyFixedColumnWriter<'b>: From<FloatColumnWriter<'b, u16, T, LEN>>,
    for<'b> AnyFixedColumnWriter<'b>: From<FloatColumnWriter<'b, u32, T, LEN>>,
    for<'b> AnyFixedColumnWriter<'b>: From<FloatColumnWriter<'b, u64, T, LEN>>,
    for<'b> AnyFixedColumnWriter<'b>: From<FloatColumnWriter<'b, f32, T, LEN>>,
    for<'b> AnyFixedColumnWriter<'b>: From<FloatColumnWriter<'b, f64, T, LEN>>,
{
    fn width(&self) -> usize {
        LEN
    }

    fn into_reader(self, nrows: usize) -> AlphaNumColumnReader {
        FloatColumnReader {
            column: vec![T::default(); nrows],
            size: self.order,
        }
        .into()
    }

    fn into_writer(self, c: &AnyFCSColumn, check: bool) -> Result<AnyFixedColumnWriter, LossError> {
        match_many_to_one!(c, AnyFCSColumn, [U08, U16, U32, U64, F32, F64], xs, {
            FCSDataType::into_writer(xs, self.order, check, |_| None).map(|w| w.into())
        })
    }
}

impl<T, const INTLEN: usize> UintColumnReader<T, INTLEN> {
    fn h_read<R: Read, const DTLEN: usize>(
        &mut self,
        h: &mut BufReader<R>,
        row: usize,
    ) -> io::Result<()>
    where
        T: IntFromBytes<DTLEN, INTLEN>,
        <T as FromStr>::Err: fmt::Display,
        // <T as FromStr>::Err: IntErr,
        T: Ord,
    {
        let x = T::h_read_int(h, &self.uint_type.byteord)?;
        self.column[row] = x.min(self.uint_type.bitmask);
        Ok(())
    }
}

impl<T, const LEN: usize> FloatColumnReader<T, LEN> {
    fn h_read<R: Read>(&mut self, h: &mut BufReader<R>, row: usize) -> io::Result<()>
    where
        T: FloatFromBytes<LEN>,
        <T as FromStr>::Err: fmt::Display,
    {
        self.column[row] = T::h_read_float(h, &self.size)?;
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

impl IsFixed for AsciiType {
    fn width(&self) -> usize {
        u8::from(self.chars).into()
    }

    fn into_reader(self, nrows: usize) -> AlphaNumColumnReader {
        AlphaNumColumnReader::Ascii(AsciiColumnReader {
            column: vec![0; nrows],
            width: self.chars,
        })
    }

    fn into_writer(
        self,
        col: &AnyFCSColumn,
        check: bool,
    ) -> Result<AnyFixedColumnWriter, LossError> {
        let c = self.chars;
        let width = u8::from(c).into();
        let go = |x: u64| {
            if x.checked_ilog10().map(|y| y + 1).unwrap_or(1) > width {
                Some(LossError::Chars)
            } else {
                None
            }
        };
        match col {
            AnyFCSColumn::U08(xs) => FCSDataType::into_writer(xs, c, check, go)
                .map(|w| AnyFixedColumnWriter::FromU08(AnyColumnWriter::Ascii(w))),
            AnyFCSColumn::U16(xs) => FCSDataType::into_writer(xs, c, check, go)
                .map(|w| AnyFixedColumnWriter::FromU16(AnyColumnWriter::Ascii(w))),
            AnyFCSColumn::U32(xs) => FCSDataType::into_writer(xs, c, check, go)
                .map(|w| AnyFixedColumnWriter::FromU32(AnyColumnWriter::Ascii(w))),
            AnyFCSColumn::U64(xs) => FCSDataType::into_writer(xs, c, check, go)
                .map(|w| AnyFixedColumnWriter::FromU64(AnyColumnWriter::Ascii(w))),
            AnyFCSColumn::F32(xs) => FCSDataType::into_writer(xs, c, check, go)
                .map(|w| AnyFixedColumnWriter::FromF32(AnyColumnWriter::Ascii(w))),
            AnyFCSColumn::F64(xs) => FCSDataType::into_writer(xs, c, check, go)
                .map(|w| AnyFixedColumnWriter::FromF64(AnyColumnWriter::Ascii(w))),
        }
    }
}

impl IsFixed for MixedType {
    fn width(&self) -> usize {
        match self {
            MixedType::Ascii(a) => u8::from(a.chars).into(),
            MixedType::Integer(i) => i.width(),
            MixedType::Float(f) => f.width(),
            MixedType::Double(d) => d.width(),
        }
    }

    fn into_reader(self, nrows: usize) -> AlphaNumColumnReader {
        match self {
            MixedType::Ascii(a) => a.into_reader(nrows),
            MixedType::Integer(i) => i.into_reader(nrows),
            MixedType::Float(f) => f.into_reader(nrows),
            MixedType::Double(d) => d.into_reader(nrows),
        }
    }

    fn into_writer(self, c: &AnyFCSColumn, check: bool) -> Result<AnyFixedColumnWriter, LossError> {
        match self {
            MixedType::Ascii(a) => a.into_writer(c, check),
            MixedType::Integer(i) => i.into_writer(c, check),
            MixedType::Float(f) => f.into_writer(c, check),
            MixedType::Double(d) => d.into_writer(c, check),
        }
    }
}

// TODO clean up errors here
impl AnyUintLayout {
    pub(crate) fn try_new<D>(
        cs: Vec<ColumnLayoutData<D>>,
        o: &ByteOrd,
    ) -> Result<Self, Vec<String>> {
        let ncols = cs.len();
        let (ws, rs): (Vec<_>, Vec<_>) = cs.into_iter().map(|c| (c.width, c.range)).unzip();
        let fixed: Vec<_> = ws.into_iter().flat_map(|w: Width| w.as_fixed()).collect();
        if fixed.len() < ncols {
            return Err(vec!["not all fixed width".into()]);
        }
        let bytes: Vec<_> = fixed.into_iter().flat_map(|f| f.bytes()).collect();
        if bytes.len() < ncols {
            return Err(vec!["bad stuff".into()]);
        }
        let mut fixed_unique = bytes.iter().unique();
        // ASSUME this won't fail
        let bytes1 = fixed_unique.next().unwrap();
        if fixed_unique.count() > 0 {
            return Err(vec!["bad stuff".into()]);
        }
        match u8::from(*bytes1) {
            1 => u8::layout(rs, o).map(AnyUintLayout::Uint08),
            2 => u16::layout(rs, o).map(AnyUintLayout::Uint16),
            3 => <u32 as IntFromBytes<4, 3>>::layout(rs, o).map(AnyUintLayout::Uint24),
            4 => <u32 as IntFromBytes<4, 4>>::layout(rs, o).map(AnyUintLayout::Uint32),
            5 => <u64 as IntFromBytes<8, 5>>::layout(rs, o).map(AnyUintLayout::Uint40),
            6 => <u64 as IntFromBytes<8, 6>>::layout(rs, o).map(AnyUintLayout::Uint48),
            7 => <u64 as IntFromBytes<8, 7>>::layout(rs, o).map(AnyUintLayout::Uint56),
            8 => <u64 as IntFromBytes<8, 8>>::layout(rs, o).map(AnyUintLayout::Uint64),
            _ => unreachable!(),
        }
    }

    fn ncols(&self) -> usize {
        match_many_to_one!(
            self,
            AnyUintLayout,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            l,
            { l.columns.len() }
        )
    }

    fn into_reader(self, data_seg: Segment, tot: Option<Tot>) -> PureSuccess<AlphaNumReader> {
        match_many_to_one!(
            self,
            AnyUintLayout,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            l,
            { l.into_reader(data_seg, tot) }
        )
    }

    fn as_writer<'a>(
        &self,
        df: &'a FCSDataFrame,
        conf: &WriteConfig,
    ) -> Result<FixedWriter<'a>, Vec<String>> {
        match_many_to_one!(
            self,
            AnyUintLayout,
            [Uint08, Uint16, Uint24, Uint32, Uint40, Uint48, Uint56, Uint64],
            l,
            { l.as_writer(df, conf) }
        )
    }
}

impl AsciiLayout {
    pub(crate) fn try_new<D>(cs: Vec<ColumnLayoutData<D>>) -> Result<Self, String> {
        let ncols = cs.len();
        let fixed: Vec<_> = cs.into_iter().flat_map(|c| c.width.as_fixed()).collect();
        if fixed.len() == ncols {
            Ok(AsciiLayout::Delimited(DelimitedLayout { ncols }))
        } else if fixed.len() == ncols {
            let columns: Vec<_> = fixed
                .into_iter()
                .flat_map(|f| f.chars())
                .map(|chars| AsciiType { chars })
                .collect();
            if columns.len() == ncols {
                Ok(AsciiLayout::Fixed(FixedLayout { columns }))
            } else {
                Err("some columns greater than 20 chars wide".into())
            }
        } else {
            Err("mixed of variable and fixed column widths".into())
        }
    }

    fn ncols(&self) -> usize {
        match self {
            AsciiLayout::Delimited(a) => a.ncols,
            AsciiLayout::Fixed(l) => l.columns.len(),
        }
    }

    fn as_writer<'a>(
        &self,
        df: &'a FCSDataFrame,
        conf: &WriteConfig,
    ) -> Result<DataWriter<'a>, Vec<String>> {
        match self {
            AsciiLayout::Fixed(a) => a.as_writer(df, conf).map(DataWriter::Fixed),
            AsciiLayout::Delimited(_) => {
                let ch = conf.check_conversion;
                let go = |c: &'a AnyFCSColumn| match c {
                    AnyFCSColumn::U08(xs) => FCSDataType::into_writer(xs, (), ch, |_| None)
                        .map(AnyDelimColumnWriter::FromU08),
                    AnyFCSColumn::U16(xs) => FCSDataType::into_writer(xs, (), ch, |_| None)
                        .map(AnyDelimColumnWriter::FromU16),
                    AnyFCSColumn::U32(xs) => FCSDataType::into_writer(xs, (), ch, |_| None)
                        .map(AnyDelimColumnWriter::FromU32),
                    AnyFCSColumn::U64(xs) => FCSDataType::into_writer(xs, (), ch, |_| None)
                        .map(AnyDelimColumnWriter::FromU64),
                    AnyFCSColumn::F32(xs) => FCSDataType::into_writer(xs, (), ch, |_| None)
                        .map(AnyDelimColumnWriter::FromF32),
                    AnyFCSColumn::F64(xs) => FCSDataType::into_writer(xs, (), ch, |_| None)
                        .map(AnyDelimColumnWriter::FromF64),
                };
                let (pass, fail): (Vec<_>, Vec<_>) = df
                    .iter_columns()
                    .enumerate()
                    .map(|(i, c)| go(c).map_err(|e: LossError| (i, e)))
                    .partition_result();
                let nbytes = df.ascii_nchars();
                if fail.is_empty() {
                    Ok(DataWriter::Delim(DelimWriter {
                        columns: pass,
                        nrows: df.nrows(),
                        nbytes,
                    }))
                } else {
                    Err(fail
                        .into_iter()
                        .map(|(i, e)| format!("Convertion error in column {i}: {e}"))
                        .collect())
                }
            }
        }
    }

    fn into_reader(self, seg: Segment, kw_tot: Option<Tot>) -> PureMaybe<ColumnReader> {
        let nbytes = seg.nbytes() as usize;
        match self {
            AsciiLayout::Delimited(l) => PureSuccess::from(kw_tot.map(|tot| {
                ColumnReader::DelimitedAscii(DelimAsciiReader {
                    ncols: l.ncols,
                    // TODO wut???
                    nrows: Some(tot),
                    nbytes,
                })
            })),
            AsciiLayout::Fixed(fl) => fl
                .into_reader(seg, kw_tot)
                .map(|x| Some(ColumnReader::AlphaNum(x))),
        }
    }
}

impl FloatLayout {
    fn ncols(&self) -> usize {
        match self {
            FloatLayout::F32(l) => l.columns.len(),
            FloatLayout::F64(l) => l.columns.len(),
        }
    }

    fn into_reader(self, seg: Segment, kw_tot: Option<Tot>) -> PureSuccess<AlphaNumReader> {
        match self {
            FloatLayout::F32(l) => l.into_reader(seg, kw_tot),
            FloatLayout::F64(l) => l.into_reader(seg, kw_tot),
        }
    }

    // TODO return type error is vague
    fn as_writer<'a>(
        &self,
        df: &'a FCSDataFrame,
        conf: &WriteConfig,
    ) -> Result<FixedWriter<'a>, Vec<String>> {
        match self {
            FloatLayout::F32(l) => l.as_writer(df, conf),
            FloatLayout::F64(l) => l.as_writer(df, conf),
        }
    }
}

impl VersionedDataLayout for DataLayout2_0 {
    type S = ByteOrd;
    type D = ();

    fn try_new(
        datatype: AlphaNumType,
        byteord: Self::S,
        columns: Vec<ColumnLayoutData<Self::D>>,
    ) -> Result<Self, Vec<String>> {
        match datatype {
            AlphaNumType::Ascii => AsciiLayout::try_new(columns)
                .map(DataLayout2_0::Ascii)
                .map_err(|e| vec![e]),
            AlphaNumType::Integer => {
                AnyUintLayout::try_new(columns, &byteord).map(DataLayout2_0::Integer)
            }
            AlphaNumType::Single => {
                f32::layout(columns, &byteord).map(|x| DataLayout2_0::Float(FloatLayout::F32(x)))
            }
            AlphaNumType::Double => {
                f64::layout(columns, &byteord).map(|x| DataLayout2_0::Float(FloatLayout::F64(x)))
            }
        }
    }

    fn try_new_from_raw(kws: &StdKeywords) -> Result<Self, Vec<String>> {
        let (datatype, byteord, columns) = kws_get_layout_2_0(kws)?;
        Self::try_new(datatype, byteord, columns)
    }

    fn ncols(&self) -> usize {
        match self {
            DataLayout2_0::Ascii(a) => a.ncols(),
            DataLayout2_0::Integer(i) => i.ncols(),
            DataLayout2_0::Float(f) => f.ncols(),
        }
    }

    fn as_writer_inner<'a>(
        &self,
        df: &'a FCSDataFrame,
        conf: &WriteConfig,
    ) -> Result<DataWriter<'a>, Vec<String>> {
        match self {
            DataLayout2_0::Ascii(a) => a.as_writer(df, conf),
            DataLayout2_0::Integer(i) => i.as_writer(df, conf).map(DataWriter::Fixed),
            DataLayout2_0::Float(f) => f.as_writer(df, conf).map(DataWriter::Fixed),
        }
    }

    fn into_reader(self, kws: &StdKeywords, data_seg: Segment) -> PureMaybe<ColumnReader> {
        let res = Tot::get_meta_opt(kws).map(|tot| tot.0);
        let nbytes = data_seg.nbytes() as usize;
        PureMaybe::from_result_1(res, PureErrorLevel::Error)
            .map(|tot| tot.flatten())
            .and_then(|tot| match self {
                DataLayout2_0::Ascii(a) => match a {
                    AsciiLayout::Delimited(dl) => {
                        PureSuccess::from(ColumnReader::DelimitedAscii(DelimAsciiReader {
                            nbytes,
                            nrows: tot,
                            ncols: dl.ncols,
                        }))
                    }
                    AsciiLayout::Fixed(fl) => {
                        fl.into_reader(data_seg, tot).map(ColumnReader::AlphaNum)
                    }
                },
                DataLayout2_0::Integer(fl) => {
                    fl.into_reader(data_seg, tot).map(ColumnReader::AlphaNum)
                }
                DataLayout2_0::Float(fl) => {
                    fl.into_reader(data_seg, tot).map(ColumnReader::AlphaNum)
                }
            })
            .map(Some)
    }
}

impl VersionedDataLayout for DataLayout3_0 {
    type S = ByteOrd;
    type D = ();

    fn try_new(
        datatype: AlphaNumType,
        byteord: Self::S,
        columns: Vec<ColumnLayoutData<Self::D>>,
    ) -> Result<Self, Vec<String>> {
        match datatype {
            AlphaNumType::Ascii => AsciiLayout::try_new(columns)
                .map(DataLayout3_0::Ascii)
                .map_err(|e| vec![e]),
            AlphaNumType::Integer => {
                AnyUintLayout::try_new(columns, &byteord).map(DataLayout3_0::Integer)
            }
            AlphaNumType::Single => {
                f32::layout(columns, &byteord).map(|x| DataLayout3_0::Float(FloatLayout::F32(x)))
            }
            AlphaNumType::Double => {
                f64::layout(columns, &byteord).map(|x| DataLayout3_0::Float(FloatLayout::F64(x)))
            }
        }
    }

    fn try_new_from_raw(kws: &StdKeywords) -> Result<Self, Vec<String>> {
        let (datatype, byteord, columns) = kws_get_layout_2_0(kws)?;
        Self::try_new(datatype, byteord, columns)
    }

    fn ncols(&self) -> usize {
        match self {
            DataLayout3_0::Ascii(a) => a.ncols(),
            DataLayout3_0::Integer(i) => i.ncols(),
            DataLayout3_0::Float(f) => f.ncols(),
        }
    }

    fn as_writer_inner<'a>(
        &self,
        df: &'a FCSDataFrame,
        conf: &WriteConfig,
    ) -> Result<DataWriter<'a>, Vec<String>> {
        match self {
            DataLayout3_0::Ascii(a) => a.as_writer(df, conf),
            DataLayout3_0::Integer(i) => i.as_writer(df, conf).map(DataWriter::Fixed),
            DataLayout3_0::Float(f) => f.as_writer(df, conf).map(DataWriter::Fixed),
        }
    }

    fn into_reader(self, kws: &StdKeywords, data_seg: Segment) -> PureMaybe<ColumnReader> {
        let res = Tot::get_meta_req(kws);
        PureMaybe::from_result_1(res, PureErrorLevel::Error).and_then(|tot| match self {
            DataLayout3_0::Ascii(a) => a.into_reader(data_seg, tot),
            DataLayout3_0::Integer(fl) => fl
                .into_reader(data_seg, tot)
                .map(|x| Some(ColumnReader::AlphaNum(x))),
            DataLayout3_0::Float(fl) => fl
                .into_reader(data_seg, tot)
                .map(|x| Some(ColumnReader::AlphaNum(x))),
        })
    }
}

impl VersionedDataLayout for DataLayout3_1 {
    type S = Endian;
    type D = ();

    fn try_new(
        datatype: AlphaNumType,
        endian: Self::S,
        columns: Vec<ColumnLayoutData<Self::D>>,
    ) -> Result<Self, Vec<String>> {
        match datatype {
            AlphaNumType::Ascii => {
                let l = AsciiLayout::try_new(columns).map_err(|e| vec![e])?;
                Ok(DataLayout3_1::Ascii(l))
            }
            AlphaNumType::Integer => {
                let l = FixedLayout::try_new(columns, endian)?;
                Ok(DataLayout3_1::Integer(l))
            }
            AlphaNumType::Single => {
                let l = f32::layout_endian(columns, endian)?;
                Ok(DataLayout3_1::Float(FloatLayout::F32(l)))
            }
            AlphaNumType::Double => {
                let l = f64::layout_endian(columns, endian)?;
                Ok(DataLayout3_1::Float(FloatLayout::F64(l)))
            }
        }
    }

    fn try_new_from_raw(kws: &StdKeywords) -> Result<Self, Vec<String>> {
        let cs = kws_get_columns(kws);
        let d = AlphaNumType::get_meta_req(kws);
        let e = Endian::get_meta_req(kws);
        match (d, e, cs) {
            (Ok(datatype), Ok(byteord), Ok(columns)) => Self::try_new(datatype, byteord, columns),
            (a, b, xs) => Err([a.err(), b.err()]
                .into_iter()
                .flatten()
                .chain(xs.err().unwrap_or_default())
                .collect()),
        }
    }

    fn ncols(&self) -> usize {
        match self {
            DataLayout3_1::Ascii(a) => a.ncols(),
            DataLayout3_1::Integer(i) => i.ncols(),
            DataLayout3_1::Float(f) => f.ncols(),
        }
    }

    fn as_writer_inner<'a>(
        &self,
        df: &'a FCSDataFrame,
        conf: &WriteConfig,
    ) -> Result<DataWriter<'a>, Vec<String>> {
        match self {
            DataLayout3_1::Ascii(a) => a.as_writer(df, conf),
            DataLayout3_1::Integer(i) => i.as_writer(df, conf).map(DataWriter::Fixed),
            DataLayout3_1::Float(f) => f.as_writer(df, conf).map(DataWriter::Fixed),
        }
    }

    fn into_reader(self, kws: &StdKeywords, data_seg: Segment) -> PureMaybe<ColumnReader> {
        let res = Tot::get_meta_req(kws);
        PureMaybe::from_result_1(res, PureErrorLevel::Error).and_then(|tot| match self {
            DataLayout3_1::Ascii(a) => a.into_reader(data_seg, tot),
            DataLayout3_1::Integer(fl) => fl
                .into_reader(data_seg, tot)
                .map(|x| Some(ColumnReader::AlphaNum(x))),
            DataLayout3_1::Float(fl) => fl
                .into_reader(data_seg, tot)
                .map(|x| Some(ColumnReader::AlphaNum(x))),
        })
    }
}

impl VersionedDataLayout for DataLayout3_2 {
    type S = Endian;
    type D = AlphaNumType;

    fn try_new(
        _: AlphaNumType,
        endian: Self::S,
        columns: Vec<ColumnLayoutData<Self::D>>,
    ) -> Result<Self, Vec<String>> {
        let unique_dt: Vec<_> = columns.iter().map(|c| c.datatype).unique().collect();
        match unique_dt[..] {
            [dt] => match dt {
                AlphaNumType::Ascii => {
                    let l = AsciiLayout::try_new(columns).map_err(|e| vec![e])?;
                    Ok(DataLayout3_2::Ascii(l))
                }
                AlphaNumType::Integer => {
                    let l = FixedLayout::try_new(columns, endian)?;
                    Ok(DataLayout3_2::Integer(l))
                }
                AlphaNumType::Single => {
                    let l = f32::layout_endian(columns, endian)?;
                    Ok(DataLayout3_2::Float(FloatLayout::F32(l)))
                }
                AlphaNumType::Double => {
                    let l = f64::layout_endian(columns, endian)?;
                    Ok(DataLayout3_2::Float(FloatLayout::F64(l)))
                }
            },
            _ => {
                let ncols = columns.len();
                let (pass, fail): (Vec<_>, Vec<_>) = columns
                    .into_iter()
                    .map(|c| MixedType::try_new(c.width, c.datatype, endian, &c.range))
                    .partition_result();
                if fail.is_empty() {
                    let cs: Vec<_> = pass.into_iter().flatten().collect();
                    if cs.len() == ncols {
                        Ok(DataLayout3_2::Mixed(FixedLayout { columns: cs }))
                    } else {
                        Err(vec![
                            "columns contain mix of variable and fixed widths".into()
                        ])
                    }
                } else {
                    Err(fail)
                }
            }
        }
    }

    fn try_new_from_raw(kws: &StdKeywords) -> Result<Self, Vec<String>> {
        let p = Par::get_meta_req(kws);
        let d = AlphaNumType::get_meta_req(kws);
        let (par, datatype) = match (p, d) {
            (Ok(par), Ok(datatype)) => Ok((par, datatype)),
            (x, y) => {
                let es: Vec<_> = [x.err(), y.err()].into_iter().flatten().collect();
                Err(es)
            }
        }?;
        let cs = {
            let (pass, fail): (Vec<_>, Vec<_>) = (0..par.0)
                .map(|i| {
                    let w = Width::get_meas_req(kws, i.into());
                    let r = Range::get_meas_req(kws, i.into());
                    let pnd = NumType::get_meas_opt(kws, i.into())
                        .map(|x| x.0.map(|y| y.into()).unwrap_or(datatype));
                    match (w, r, pnd) {
                        (Ok(width), Ok(range), Ok(pndatatype)) => Ok(ColumnLayoutData {
                            width,
                            range,
                            datatype: pndatatype,
                        }),
                        (x, y, z) => {
                            Err([x.err(), y.err(), z.err()].into_iter().flatten().collect())
                        }
                    }
                })
                .partition_result();
            if fail.is_empty() {
                Ok(pass)
            } else {
                Err(fail)
            }
        };
        let e = Endian::get_meta_req(kws);
        match (e, cs) {
            (Ok(endian), Ok(columns)) => Self::try_new(datatype, endian, columns),
            (a, xs) => Err([a.err()]
                .into_iter()
                .flatten()
                .chain(xs.err().unwrap_or_default())
                .collect()),
        }
    }

    fn ncols(&self) -> usize {
        match self {
            DataLayout3_2::Ascii(a) => a.ncols(),
            DataLayout3_2::Integer(i) => i.ncols(),
            DataLayout3_2::Float(f) => f.ncols(),
            DataLayout3_2::Mixed(m) => m.ncols(),
        }
    }

    fn as_writer_inner<'a>(
        &self,
        df: &'a FCSDataFrame,
        conf: &WriteConfig,
    ) -> Result<DataWriter<'a>, Vec<String>> {
        match self {
            DataLayout3_2::Ascii(a) => a.as_writer(df, conf),
            DataLayout3_2::Integer(i) => i.as_writer(df, conf).map(DataWriter::Fixed),
            DataLayout3_2::Float(f) => f.as_writer(df, conf).map(DataWriter::Fixed),
            DataLayout3_2::Mixed(m) => m.as_writer(df, conf).map(DataWriter::Fixed),
        }
    }

    fn into_reader(self, kws: &StdKeywords, data_seg: Segment) -> PureMaybe<ColumnReader> {
        let res = Tot::get_meta_req(kws);
        PureMaybe::from_result_1(res, PureErrorLevel::Error).and_then(|tot| match self {
            DataLayout3_2::Ascii(a) => a.into_reader(data_seg, tot),
            DataLayout3_2::Integer(fl) => fl
                .into_reader(data_seg, tot)
                .map(|x| Some(ColumnReader::AlphaNum(x))),
            DataLayout3_2::Float(fl) => fl
                .into_reader(data_seg, tot)
                .map(|x| Some(ColumnReader::AlphaNum(x))),
            DataLayout3_2::Mixed(fl) => fl
                .into_reader(data_seg, tot)
                .map(|x| Some(ColumnReader::AlphaNum(x))),
        })
    }
}

fn kws_get_layout_2_0(
    kws: &StdKeywords,
) -> Result<(AlphaNumType, ByteOrd, Vec<ColumnLayoutData<()>>), Vec<String>> {
    let cs = kws_get_columns(kws);
    let d = AlphaNumType::get_meta_req(kws);
    let b = ByteOrd::get_meta_req(kws);
    match (d, b, cs) {
        (Ok(datatype), Ok(byteord), Ok(columns)) => Ok((datatype, byteord, columns)),
        (x, y, zs) => Err([x.err(), y.err()]
            .into_iter()
            .flatten()
            .chain(zs.err().unwrap_or_default())
            .collect()),
    }
}

fn kws_get_columns(kws: &StdKeywords) -> Result<Vec<ColumnLayoutData<()>>, Vec<String>> {
    Par::get_meta_req(kws).map_err(|e| vec![e]).and_then(|par| {
        let (pass, fail): (Vec<_>, Vec<_>) = (0..par.0)
            .map(|i| {
                let w = Width::get_meas_req(kws, i.into());
                let r = Range::get_meas_req(kws, i.into());
                match (w, r) {
                    (Ok(width), Ok(range)) => Ok(ColumnLayoutData {
                        width,
                        range,
                        datatype: (),
                    }),
                    (x, y) => Err([x.err(), y.err()].into_iter().flatten().collect()),
                }
            })
            .partition_result();
        if fail.is_empty() {
            Ok(pass)
        } else {
            Err(fail)
        }
    })
}
