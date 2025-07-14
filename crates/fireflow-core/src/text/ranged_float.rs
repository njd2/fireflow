use derive_more::{Add, Display, Into, Mul};
use num_derive::{One, Zero};
use serde::Serialize;
use std::fmt;
use std::num::ParseFloatError;
use std::str::FromStr;

/// A non-negative float
#[derive(Clone, Copy, Serialize, PartialEq, Display, Into, Zero, One, Mul, Add)]
#[mul(forward)]
pub struct NonNegFloat(f32);

/// A positive float
#[derive(Clone, Copy, Serialize, PartialEq, Display, Into, One, Mul)]
#[mul(forward)]
pub struct PositiveFloat(f32);

macro_rules! impl_ranged_float {
    ($type:ident, $op:tt, $zero:expr) => {
        impl FromStr for $type {
            type Err = RangedFloatError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                s.parse::<f32>()
                    .map_err(RangedFloatError::Parse)
                    .and_then(Self::try_from)
            }
        }

        impl TryFrom<f32> for $type {
            type Error = RangedFloatError;

            fn try_from(x: f32) -> Result<Self, RangedFloatError> {
                if 0.0 $op x {
                    Ok(Self(x))
                } else {
                    Err(RangedFloatError::Range {
                        x,
                        include_zero: $zero,
                    })
                }
            }
        }
    };
}

impl_ranged_float!(PositiveFloat, <, false);
impl_ranged_float!(NonNegFloat, <=, true);

impl PositiveFloat {
    pub fn unit() -> Self {
        Self(1.0)
    }
}

pub enum RangedFloatError {
    Parse(ParseFloatError),
    Range { x: f32, include_zero: bool },
}

impl fmt::Display for RangedFloatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            RangedFloatError::Parse(e) => e.fmt(f),
            RangedFloatError::Range { x, include_zero } => {
                let gt = if *include_zero {
                    "greater than/equal to"
                } else {
                    "greater than"
                };
                write!(f, "float must be {gt} zero, got {x}")
            }
        }
    }
}
