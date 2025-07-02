//! Types to represent the $PnB and $PnR values for a uint column.

use crate::error::BiTentative;
use crate::text::float_or_int::{FloatOrInt, IntRangeError, ToIntError};

use derive_more::{Display, From};
use num_traits::PrimInt;
use serde::Serialize;
use std::fmt;

/// A number representing a value with bitmask up to LEN bits
#[derive(PartialEq, Clone, Copy, Serialize, PartialOrd)]
pub struct Bitmask<T, const LEN: usize> {
    /// The value to be masked.
    ///
    /// This can be any integer up to LEN bits.
    value: T,

    /// The bitmask corresponding to ['range'].
    ///
    /// Will always be a power of 2 minus 1 (ie, some number of contiguous bits
    /// in binary). This will be able to hold ['range'] but will mask out any
    /// bits beyond those needed to express ['range'].
    bitmask: T,
}

macro_rules! bitmask_type {
    ($name:ident, $wrapper:ident, $native:ty, $size:expr) => {
        pub type $name = $wrapper<$native, $size>;

        impl From<$name> for FloatOrInt {
            fn from(value: $name) -> Self {
                FloatOrInt::from(u64::from(value.value))
            }
        }
    };
}

bitmask_type!(Bitmask08, Bitmask, u8, 1);
bitmask_type!(Bitmask16, Bitmask, u16, 2);
bitmask_type!(Bitmask24, Bitmask, u32, 3);
bitmask_type!(Bitmask32, Bitmask, u32, 4);
bitmask_type!(Bitmask40, Bitmask, u64, 5);
bitmask_type!(Bitmask48, Bitmask, u64, 6);
bitmask_type!(Bitmask56, Bitmask, u64, 7);
bitmask_type!(Bitmask64, Bitmask, u64, 8);

impl<T, const LEN: usize> Bitmask<T, LEN> {
    /// Make new bitmask from float or integer.
    pub(crate) fn from_range(range: FloatOrInt, notrunc: bool) -> BiTentative<Self, BitmaskError>
    where
        T: TryFrom<FloatOrInt, Error = ToIntError<T>> + PrimInt,
        u64: From<T>,
    {
        // TODO subtract 1 from here (or somewhere)
        range
            .as_uint(notrunc)
            .inner_into()
            .and_tentatively(|x| Bitmask::from_native_tnt(x, notrunc).inner_into())
    }

    pub(crate) fn value(&self) -> T
    where
        T: Copy,
    {
        self.value
    }

    pub(crate) fn bitmask(&self) -> T
    where
        T: Copy,
    {
        self.bitmask
    }

    pub(crate) fn apply(&self, value: T) -> T
    where
        T: Ord + Copy,
    {
        self.bitmask.min(value)
    }

    pub(crate) fn from_native_tnt(
        value: T,
        notrunc: bool,
    ) -> BiTentative<Self, BitmaskTruncationError>
    where
        T: PrimInt,
        u64: From<T>,
    {
        let (bitmask, truncated) = Bitmask::from_native(value);
        let error = if truncated {
            Some(BitmaskTruncationError {
                bytes: Self::bits(),
                value: u64::from(value),
            })
        } else {
            None
        };
        BiTentative::new_either1(bitmask, error, notrunc)
    }

    // fn from_u64_tnt(value: u64, notrunc: bool) -> BiTentative<Self, BitmaskTruncationError>
    // where
    //     T: PrimInt + TryFrom<u64>,
    // {
    //     let (bitmask, truncated) = Bitmask::from_u64(value);
    //     let error = if truncated {
    //         Some(BitmaskTruncationError {
    //             bytes: Self::bits(),
    //             value,
    //         })
    //     } else {
    //         None
    //     };
    //     BiTentative::new_either1(bitmask, error, notrunc)
    // }

    pub fn from_native(value: T) -> (Self, bool)
    where
        T: PrimInt,
    {
        // ASSUME number of bits will never exceed 64 (or 255 for that matter)
        // and thus will fit in a u8
        let native_bits = (std::mem::size_of::<T>() * 8) as u8;
        let value_bits = native_bits - (value.leading_zeros() as u8);
        let truncated = value_bits > Self::bits();
        let bits = value_bits.min(Self::bits());
        let mask = if bits == 0 {
            T::zero()
        } else if bits == native_bits {
            T::max_value()
        } else {
            (T::one() << usize::from(value_bits)) - T::one()
        };
        (
            Self {
                bitmask: mask,
                value: value.min(mask),
            },
            truncated,
        )
    }

    pub(crate) fn from_u64(value: u64) -> (Self, bool)
    where
        T: PrimInt + TryFrom<u64>,
    {
        T::try_from(value)
            .map(Self::from_native)
            .unwrap_or((Self::max(), true))
    }

    fn max() -> Self
    where
        T: PrimInt,
    {
        Bitmask::from_native(T::max_value()).0
    }

    fn bytes() -> u8 {
        LEN as u8
    }

    fn bits() -> u8 {
        Self::bytes() * 8
    }
}

pub struct BitmaskTruncationError {
    bytes: u8,
    value: u64,
}

impl fmt::Display for BitmaskTruncationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "could not make bitmask for {} which exceeds {} bytes",
            self.value, self.bytes
        )
    }
}

#[derive(Display, From)]
pub enum BitmaskError {
    ToInt(IntRangeError),
    Trunc(BitmaskTruncationError),
}
