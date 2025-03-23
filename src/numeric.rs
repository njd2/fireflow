use std::io;
use std::io::{BufReader, Read, Seek};

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
pub enum Endian {
    Big,
    Little,
}

pub enum Series {
    F32(Vec<f32>),
    F64(Vec<f64>),
    U8(Vec<u8>),
    U16(Vec<u16>),
    U32(Vec<u32>),
    U64(Vec<u64>),
}

impl Endian {
    fn is_big(&self) -> bool {
        matches!(self, Endian::Big)
    }
}

impl Series {
    pub fn len(&self) -> usize {
        match self {
            Series::F32(x) => x.len(),
            Series::F64(x) => x.len(),
            Series::U8(x) => x.len(),
            Series::U16(x) => x.len(),
            Series::U32(x) => x.len(),
            Series::U64(x) => x.len(),
        }
    }

    pub fn format(&self, r: usize) -> String {
        match self {
            Series::F32(x) => format!("{}", x[r]),
            Series::F64(x) => format!("{}", x[r]),
            Series::U8(x) => format!("{}", x[r]),
            Series::U16(x) => format!("{}", x[r]),
            Series::U32(x) => format!("{}", x[r]),
            Series::U64(x) => format!("{}", x[r]),
        }
    }
}

pub trait IntMath: Sized {
    fn next_power_2(x: Self) -> Self;
}

pub trait NumProps<const DTLEN: usize>: Sized + Copy {
    // TODO use From trait
    // fn into_series(x: Vec<Self>) -> Series;

    fn zero() -> Self;

    fn from_big(buf: [u8; DTLEN]) -> Self;

    fn from_little(buf: [u8; DTLEN]) -> Self;

    fn read_from_big<R: Read + Seek>(h: &mut BufReader<R>) -> io::Result<Self> {
        let mut buf = [0; DTLEN];
        h.read_exact(&mut buf)?;
        Ok(Self::from_big(buf))
    }

    fn read_from_little<R: Read + Seek>(h: &mut BufReader<R>) -> io::Result<Self> {
        let mut buf = [0; DTLEN];
        h.read_exact(&mut buf)?;
        Ok(Self::from_little(buf))
    }

    fn read_from_endian<R: Read + Seek>(h: &mut BufReader<R>, endian: Endian) -> io::Result<Self> {
        if endian.is_big() {
            Self::read_from_big(h)
        } else {
            Self::read_from_little(h)
        }
    }
}

macro_rules! impl_num_props {
    ($size:expr, $zero:expr, $t:ty, $p:ident) => {
        impl From<Vec<$t>> for Series {
            fn from(value: Vec<$t>) -> Self {
                Series::$p(value)
            }
        }

        impl NumProps<$size> for $t {
            // fn into_series(x: Vec<Self>) -> Series {
            //     Series::$p(x)
            // }

            fn zero() -> Self {
                $zero
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

impl_num_props!(1, 0, u8, U8);
impl_num_props!(2, 0, u16, U16);
impl_num_props!(4, 0, u32, U32);
impl_num_props!(8, 0, u64, U64);
impl_num_props!(4, 0.0, f32, F32);
impl_num_props!(8, 0.0, f64, F64);

macro_rules! impl_int_math {
    ($t:ty) => {
        impl IntMath for $t {
            fn next_power_2(x: Self) -> Self {
                Self::checked_next_power_of_two(x).unwrap_or(Self::MAX)
            }
        }
    };
}

impl_int_math!(u8);
impl_int_math!(u16);
impl_int_math!(u32);
impl_int_math!(u64);
