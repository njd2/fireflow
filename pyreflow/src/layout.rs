use fireflow_core::data::{
    AnyNullBitmask, AnyOrderedLayout, DataLayout2_0, DataLayout3_0, DataLayout3_1, DataLayout3_2,
    EndianLayoutOps, FloatType, LayoutOps, NonMixedEndianLayout, NullMixedType, OrderedLayoutOps,
    UintType,
};
use fireflow_core::text::byteord::{Endian, SizedByteOrd};
use fireflow_core::text::float_decimal::{FloatDecimal, HasFloatBounds};
use fireflow_core::text::keywords::AlphaNumType;
use fireflow_core::validated::ascii_range::{AsciiRange, Chars};
use fireflow_core::validated::bitmask::Bitmask;

use crate::class::PyreflowException;

use super::macros::{py_disp, py_enum, py_eq, py_parse, py_wrap};

use bigdecimal::BigDecimal;
use nonempty::NonEmpty;
use pyo3::prelude::*;
use pyo3::types::PyType;

// $DATATYPE (all versions)
py_wrap!(pub(crate) PyAlphaNumType, AlphaNumType, "AlphaNumType");
py_eq!(PyAlphaNumType);
py_disp!(PyAlphaNumType);
py_parse!(PyAlphaNumType);
py_enum!(
    PyAlphaNumType,
    AlphaNumType,
    [Ascii, ASCII],
    [Integer, INTEGER],
    [Single, SINGLE],
    [Double, DOUBLE]
);

// All layouts
py_wrap!(pub(crate) PyDataLayout2_0, DataLayout2_0, "DataLayout2_0");
py_wrap!(pub(crate) PyDataLayout3_0, DataLayout3_0, "DataLayout3_0");
py_wrap!(pub(crate) PyDataLayout3_1, DataLayout3_1, "DataLayout3_1");
py_wrap!(pub(crate) PyDataLayout3_2, DataLayout3_2, "DataLayout3_2");

macro_rules! common_methods {
    ($t:ident) => {
        #[pymethods]
        impl $t {
            /// Return the widths of each column (ie the $PnB keyword).
            ///
            /// This will be a list of integers equal to the number of columns
            /// or an empty list if the layout is delimited Ascii (in which case
            /// it has no column widths).
            #[getter]
            fn widths(&self) -> Vec<u8> {
                self.0.widths().into_iter().map(u8::from).collect()
            }

            /// Return a list of ranges for each column.
            ///
            /// The elements of the list will be either a float or int and
            /// will depend on the underlying layout structure.
            #[getter]
            fn ranges(&self) -> Vec<BigDecimal> {
                self.0.ranges().into_iter().map(|r| r.0).collect()
            }
        }
    };
}

common_methods!(PyDataLayout2_0);
common_methods!(PyDataLayout3_0);
common_methods!(PyDataLayout3_1);
common_methods!(PyDataLayout3_2);

macro_rules! byte_order_methods {
    ($t:ident) => {
        #[pymethods]
        impl $t {
            #[getter]
            /// Return the byte order of the layout.
            ///
            /// If ASCII, return empty list. Otherwise return the byte layout
            /// as a zero-indexed list of integers.
            fn byte_order(&self) -> Vec<u8> {
                (self.0)
                    .0
                    .byte_order()
                    .map(|b| b.as_vec())
                    .unwrap_or_default()
            }
        }
    };
}

byte_order_methods!(PyDataLayout2_0);
byte_order_methods!(PyDataLayout3_0);

macro_rules! endianness_methods {
    ($t:ident) => {
        #[pymethods]
        impl $t {
            #[getter]
            /// Return the byte order of the layout.
            ///
            /// If layout is ASCII or has mixed byte order, return None.
            /// If big endian, return true, or false for little endian.
            fn endianness(&self) -> Option<bool> {
                (self.0).0.endianness().map(|e| (e == Endian::Big))
            }
        }
    };
}

endianness_methods!(PyDataLayout2_0);
endianness_methods!(PyDataLayout3_0);
endianness_methods!(PyDataLayout3_1);

macro_rules! datatype_getter {
    ($t:ident) => {
        #[pymethods]
        impl $t {
            #[getter]
            /// Return the datatype of the layout.
            fn datatype(&self) -> PyAlphaNumType {
                (self.0).0.datatype().into()
            }
        }
    };
}

datatype_getter!(PyDataLayout2_0);
datatype_getter!(PyDataLayout3_0);
datatype_getter!(PyDataLayout3_1);

// ascii layouts for all versions
macro_rules! new_ascii_methods {
    ($t:ident, $wrap:path, $subwrap:ident) => {
        #[pymethods]
        impl $t {
            #[classmethod]
            fn new_ascii_delim(_: &Bound<'_, PyType>, ranges: Vec<u64>) -> PyResult<Self> {
                let rs = vec_to_ne(ranges)?;
                Ok($wrap($subwrap::new_ascii_delim(rs)).into())
            }

            #[classmethod]
            fn new_ascii_fixed_u64(_: &Bound<'_, PyType>, ranges: Vec<u64>) -> PyResult<Self> {
                let xs = vec_to_ne(ranges)?;
                let rs = xs.map(AsciiRange::from);
                Ok($wrap($subwrap::new_ascii_fixed(rs)).into())
            }

            #[classmethod]
            fn new_ascii_fixed_pairs(
                _: &Bound<'_, PyType>,
                ranges: Vec<(u64, u8)>,
            ) -> PyResult<Self> {
                let xs = vec_to_ne(ranges)?;
                let ys = xs
                    .try_map(|(x, c)| Chars::try_from(c).map(|y| (x, y)))
                    .map_err(|e| PyreflowException::new_err(e.to_string()))?;
                let rs = ys
                    .try_map(|(x, c)| AsciiRange::try_new(x, c))
                    .map_err(|e| PyreflowException::new_err(e.to_string()))?;
                Ok($wrap($subwrap::new_ascii_fixed(rs)).into())
            }
        }
    };
}

new_ascii_methods!(PyDataLayout2_0, DataLayout2_0, AnyOrderedLayout);
new_ascii_methods!(PyDataLayout3_0, DataLayout3_0, AnyOrderedLayout);
new_ascii_methods!(PyDataLayout3_1, DataLayout3_1, NonMixedEndianLayout);
new_ascii_methods!(
    PyDataLayout3_2,
    DataLayout3_2::NonMixed,
    NonMixedEndianLayout
);

// integer layouts for 2.0/3.0
macro_rules! new_uint_2_0 {
    ($t:ident, $wrap:path, $fn_ordered:ident, $fn_endian:ident, $uint:ident, $size:expr) => {
        #[pymethods]
        impl $t {
            #[classmethod]
            /// Make a new layout for $size-byte Uints with a given byte order.
            fn $fn_ordered(
                _: &Bound<'_, PyType>,
                ranges: Vec<$uint>,
                byteord: Vec<u8>,
            ) -> PyResult<Self> {
                let rs = vec_to_bitmasks::<$uint, $size>(ranges)?;
                let b = vec_to_byteord(byteord)?;
                Ok($wrap(AnyOrderedLayout::new_uint(rs, b)).into())
            }

            /// Make a new layout for $size-byte Uints with a given endian-ness.
            #[classmethod]
            fn $fn_endian(
                _: &Bound<'_, PyType>,
                ranges: Vec<$uint>,
                is_big: bool,
            ) -> PyResult<Self> {
                let b = SizedByteOrd::Endian(Endian::is_big(is_big));
                let rs = vec_to_bitmasks::<$uint, $size>(ranges)?;
                Ok($wrap(AnyOrderedLayout::new_uint(rs, b)).into())
            }
        }
    };
}

macro_rules! new_ordered_uint_methods {
    ($t:ident, $wrap:path) => {
        new_uint_2_0!($t, $wrap, new_uint08_ordered, new_uint08_endian, u8, 1);
        new_uint_2_0!($t, $wrap, new_uint16_ordered, new_uint16_endian, u16, 2);
        new_uint_2_0!($t, $wrap, new_uint24_ordered, new_uint24_endian, u32, 3);
        new_uint_2_0!($t, $wrap, new_uint32_ordered, new_uint32_endian, u32, 4);
        new_uint_2_0!($t, $wrap, new_uint40_ordered, new_uint40_endian, u64, 5);
        new_uint_2_0!($t, $wrap, new_uint48_ordered, new_uint48_endian, u64, 6);
        new_uint_2_0!($t, $wrap, new_uint56_ordered, new_uint56_endian, u64, 7);
        new_uint_2_0!($t, $wrap, new_uint64_ordered, new_uint64_endian, u64, 8);
    };
}

new_ordered_uint_methods!(PyDataLayout2_0, DataLayout2_0);
new_ordered_uint_methods!(PyDataLayout3_0, DataLayout3_0);

// float layouts for 2.0/3.0
macro_rules! new_float_2_0 {
    ($t:ident, $wrap:path, $fn_ordered:ident, $fn_endian:ident, $num:ident, $subfn:ident) => {
        #[pymethods]
        impl $t {
            #[classmethod]
            /// Make a new $num layout with a given byte order.
            fn $fn_ordered(
                _: &Bound<'_, PyType>,
                ranges: Vec<$num>,
                byteord: Vec<u8>,
            ) -> PyResult<Self> {
                let rs = vec_to_float_ranges(ranges)?;
                let b = vec_to_byteord(byteord)?;
                Ok($wrap(AnyOrderedLayout::$subfn(rs, b)).into())
            }

            #[classmethod]
            /// Make a new $num layout with a given endian-ness.
            fn $fn_endian(
                _: &Bound<'_, PyType>,
                ranges: Vec<$num>,
                is_big: bool,
            ) -> PyResult<Self> {
                let e = SizedByteOrd::Endian(Endian::is_big(is_big));
                let rs = vec_to_float_ranges(ranges)?;
                Ok($wrap(AnyOrderedLayout::$subfn(rs, e)).into())
            }
        }
    };
}

new_float_2_0!(
    PyDataLayout2_0,
    DataLayout2_0,
    new_f32_ordered,
    new_f32_endian,
    f32,
    new_f32
);
new_float_2_0!(
    PyDataLayout2_0,
    DataLayout2_0,
    new_f64_ordered,
    new_f64_endian,
    f64,
    new_f64
);
new_float_2_0!(
    PyDataLayout3_0,
    DataLayout3_0,
    new_f32_ordered,
    new_f32_endian,
    f32,
    new_f32
);
new_float_2_0!(
    PyDataLayout3_0,
    DataLayout3_0,
    new_f64_ordered,
    new_f64_endian,
    f64,
    new_f64
);

// float layouts for 3.1/3.2
macro_rules! new_float_3_1 {
    ($t:ident, $wrap:path, $fn:ident, $num:ident) => {
        #[pymethods]
        impl $t {
            #[classmethod]
            /// Make a new $num layout with a given endian-ness.
            fn $fn(_: &Bound<'_, PyType>, ranges: Vec<$num>, is_big: bool) -> PyResult<Self> {
                let e = Endian::is_big(is_big);
                let rs = vec_to_float_ranges(ranges)?;
                Ok($wrap(NonMixedEndianLayout::$fn(rs, e)).into())
            }
        }
    };
}

new_float_3_1!(PyDataLayout3_1, DataLayout3_1, new_f32, f32);
new_float_3_1!(PyDataLayout3_1, DataLayout3_1, new_f64, f64);
new_float_3_1!(PyDataLayout3_2, DataLayout3_2::NonMixed, new_f32, f32);
new_float_3_1!(PyDataLayout3_2, DataLayout3_2::NonMixed, new_f64, f64);

// uint layout for 3.1/3.2
macro_rules! new_uint_3_1 {
    ($t:ident, $wrap:path) => {
        #[pymethods]
        impl $t {
            #[classmethod]
            /// Make a new Uint layout with a given endian-ness.
            ///
            /// Width of each column (in bytes) will depend in the input range.
            fn new_uint(_: &Bound<'_, PyType>, ranges: Vec<u64>, is_big: bool) -> PyResult<Self> {
                let e = Endian::is_big(is_big);
                let xs = vec_to_ne(ranges)?;
                let rs = xs.map(AnyNullBitmask::from_u64);
                Ok($wrap(NonMixedEndianLayout::new_uint(rs, e)).into())
            }
        }
    };
}

new_uint_3_1!(PyDataLayout3_1, DataLayout3_1);
new_uint_3_1!(PyDataLayout3_2, DataLayout3_2::NonMixed);

#[pymethods]
impl PyDataLayout3_2 {
    #[classmethod]
    /// Make a new mixed layout with a given endian-ness.
    ///
    /// Columns must be specified as pairs like (flag, value) where 'flag'
    /// is one of "A", "I", "F", or "D" corresponding to Ascii, Integer, Float,
    /// or Double datatypes. The 'value' field should be an integer for "A" or
    /// "I" and a float for "F" or "D".
    fn new_mixed(
        _: &Bound<'_, PyType>,
        ranges: Vec<Bound<'_, PyAny>>,
        is_big: bool,
    ) -> PyResult<Self> {
        let e = Endian::is_big(is_big);
        let rs = vec_to_mixed_type(ranges)?;
        Ok(DataLayout3_2::new_mixed(rs, e).into())
    }

    #[getter]
    /// Return true if layout has more than one datatype.
    fn is_mixed(&self) -> bool {
        matches!(self.0, DataLayout3_2::Mixed(_))
    }

    #[getter]
    /// Return list of datatypes for each column.
    ///
    /// If not a mixed layout, datatypes will be identical.
    fn datatypes(&self) -> Vec<PyAlphaNumType> {
        Vec::from(self.0.datatypes())
            .into_iter()
            .map(|x| x.into())
            .collect()
    }

    // TODO not DRY, but didn't feel like making the above macro more flexible
    #[getter]
    /// Return the byte order of the layout.
    ///
    /// If layout is ASCII or has mixed byte order, return None.
    /// If big endian, return true, or false for little endian.
    fn endianness(&self) -> Option<bool> {
        (self.0).endianness().map(|e| (e == Endian::Big))
    }
}

fn vec_to_float_ranges<T, const LEN: usize>(xs: Vec<T>) -> PyResult<NonEmpty<FloatType<T, LEN>>>
where
    T: HasFloatBounds,
    FloatDecimal<T>: TryFrom<T>,
    <FloatDecimal<T> as TryFrom<T>>::Error: std::fmt::Display,
{
    let ns = vec_to_ne(xs)?;
    ns.try_map(|r| FloatDecimal::try_from(r).map(FloatType::new))
        .map_err(|e| PyreflowException::new_err(e.to_string()))
}

fn vec_to_bitmasks<T, const LEN: usize>(xs: Vec<T>) -> PyResult<NonEmpty<UintType<T, LEN>>>
where
    T: Copy + std::fmt::Display + num_traits::PrimInt,
{
    let ns = vec_to_ne(xs)?;
    ns.try_map(|x| {
        let (y, trunc) = Bitmask::from_native(x);
        if trunc {
            let e = format!("could not make {LEN}-byte bitmask from {x}");
            Err(PyreflowException::new_err(e))
        } else {
            Ok(y)
        }
    })
}

fn vec_to_mixed_type(xs: Vec<Bound<'_, PyAny>>) -> PyResult<NonEmpty<NullMixedType>> {
    let ns = vec_to_ne(xs)?;
    ns.try_map(any_to_mixed_type)
}

fn any_to_mixed_type(x: Bound<'_, PyAny>) -> PyResult<NullMixedType> {
    if let Ok((flag, value)) = x.extract::<(String, Bound<'_, PyAny>)>() {
        match flag.as_str() {
            "F" => value
                .extract::<f32>()
                .map_err(|e| e.to_string())
                .and_then(|y| FloatDecimal::<f32>::try_from(y).map_err(|e| e.to_string()))
                .map(|range| NullMixedType::F32(FloatType::new(range))),
            "D" => value
                .extract::<f64>()
                .map_err(|e| e.to_string())
                .and_then(|y| FloatDecimal::<f64>::try_from(y).map_err(|e| e.to_string()))
                .map(|range| NullMixedType::F64(FloatType::new(range))),
            "I" => value
                .extract()
                .map(|y| NullMixedType::Uint(AnyNullBitmask::from_u64(y)))
                .map_err(|e| e.to_string()),
            "A" => value
                .extract::<u64>()
                .map(|y| NullMixedType::Ascii(AsciiRange::from(y)))
                .map_err(|e| e.to_string()),
            _ => Err("flag must be one of 'A', 'F', 'D', or 'I'".to_string()),
        }
    } else {
        Err("must be pair like '(flag, value)'".to_string())
    }
    .map_err(PyreflowException::new_err)
}

fn vec_to_ne<X>(xs: Vec<X>) -> PyResult<NonEmpty<X>> {
    NonEmpty::from_vec(xs).ok_or(PyreflowException::new_err(
        "list must not be empty".to_string(),
    ))
}

fn vec_to_byteord<const LEN: usize>(xs: Vec<u8>) -> PyResult<SizedByteOrd<LEN>>
where
    SizedByteOrd<LEN>: TryFrom<Vec<u8>>,
    <SizedByteOrd<LEN> as TryFrom<Vec<u8>>>::Error: std::fmt::Display,
{
    SizedByteOrd::<LEN>::try_from(xs).map_err(|e| PyreflowException::new_err(e.to_string()))
}
