use crate::config::StdTextReadConfig;
use crate::error::*;
use crate::text::index::MeasIndex;
use crate::text::optional::*;
use crate::text::parser::*;
use crate::text::ranged_float::*;
use crate::validated::keys::*;

use serde::Serialize;
use std::fmt;
use std::num::ParseFloatError;
use std::str::FromStr;

use super::parser::LookupTentative;

/// The value for the $PnE key (all versions).
///
/// Format is assumed to be 'f1,f2'
#[derive(Clone, Copy, PartialEq, Serialize)]
pub enum Scale {
    /// Linear scale (ie '0,0')
    Linear,

    /// Log scale, where both numbers are positive
    Log(LogScale),
}

#[derive(Clone, Copy, PartialEq, Serialize)]
pub struct LogScale {
    decades: PositiveFloat,
    offset: PositiveFloat,
}

impl Scale {
    pub fn try_new_log(decades: f32, offset: f32) -> Result<Self, LogRangeError> {
        let d = PositiveFloat::try_from(decades);
        let o = PositiveFloat::try_from(offset);
        d.zip(o)
            .map(|(dx, ox)| {
                Scale::Log(LogScale {
                    decades: dx,
                    offset: ox,
                })
            })
            .map_err(|_| LogRangeError { decades, offset })
    }
}

impl FromStr for Scale {
    type Err = ScaleError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.split(",").collect::<Vec<_>>()[..] {
            [ds, os] => {
                let f1 = ds.parse().map_err(ScaleError::FloatError)?;
                let f2 = os.parse().map_err(ScaleError::FloatError)?;
                match (f1, f2) {
                    (0.0, 0.0) => Ok(Scale::Linear),
                    (decades, offset) => {
                        Scale::try_new_log(decades, offset).map_err(ScaleError::LogRange)
                    }
                }
            }
            _ => Err(ScaleError::WrongFormat),
        }
    }
}

impl Scale {
    pub(crate) fn lookup_fixed_req(
        kws: &mut StdKeywords,
        i: MeasIndex,
        try_fix: bool,
    ) -> LookupResult<Scale> {
        let res = Scale::remove_meas_req(kws, i.into());
        if try_fix {
            res.map_or_else(
                |e| {
                    e.with_parse_error(|se| {
                        if let ScaleError::LogRange(le) = se {
                            le.try_fix_offset()
                                .map(Scale::Log)
                                .map_err(ScaleError::LogRange)
                        } else {
                            Err(se)
                        }
                    })
                },
                Ok,
            )
        } else {
            res
        }
        .map_err(|e| e.inner_into())
        .map_err(Box::new)
        .into_deferred()
    }

    pub(crate) fn lookup_fixed_opt<E>(
        kws: &mut StdKeywords,
        i: MeasIndex,
        conf: &StdTextReadConfig,
    ) -> LookupTentative<MaybeValue<Scale>, E> {
        let res = Self::lookup_fixed_opt_inner(kws, i, conf.fix_log_scale_offsets);
        process_opt(res)
    }

    pub(crate) fn lookup_fixed_opt_dep(
        kws: &mut StdKeywords,
        i: MeasIndex,
        conf: &StdTextReadConfig,
    ) -> LookupTentative<MaybeValue<Scale>, DeprecatedError> {
        let dd = conf.disallow_deprecated;
        let res = Self::lookup_fixed_opt_inner(kws, i, conf.fix_log_scale_offsets);
        process_opt_dep(res, Scale::std(i.into()), dd)
    }

    fn lookup_fixed_opt_inner(
        kws: &mut StdKeywords,
        i: MeasIndex,
        try_fix: bool,
    ) -> OptKwResult<Scale> {
        let res = Scale::remove_meas_opt(kws, i.into());
        if try_fix {
            res.map_or_else(
                |e| {
                    e.with_error(|se| {
                        if let ScaleError::LogRange(le) = se {
                            le.try_fix_offset()
                                .map(|x| Some(Scale::Log(x)).into())
                                .map_err(ScaleError::LogRange)
                        } else {
                            Err(se)
                        }
                    })
                },
                Ok,
            )
        } else {
            res
        }
    }
}

impl fmt::Display for Scale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            Scale::Log(LogScale { decades, offset }) => write!(f, "{decades},{offset}"),
            Scale::Linear => write!(f, "0,0"),
        }
    }
}

pub enum ScaleError {
    FloatError(ParseFloatError),
    LogRange(LogRangeError),
    WrongFormat,
}

impl fmt::Display for ScaleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            ScaleError::FloatError(x) => write!(f, "{}", x),
            ScaleError::WrongFormat => write!(f, "must be like 'f1,f2'"),
            ScaleError::LogRange(r) => r.fmt(f),
        }
    }
}

pub struct LogRangeError {
    decades: f32,
    offset: f32,
}

impl LogRangeError {
    /// Try to 'fix' log scales which are 'X,0' where X is positive.
    ///
    /// The 'recommended' way to fix these is to make the 0 and 1, which is
    /// what this does. This is a heuristic hack to get some files to work
    /// which didn't write $PnE correctly.
    pub(crate) fn try_fix_offset(self) -> Result<LogScale, Self> {
        if self.offset == 0.0 {
            if let Ok(decades) = PositiveFloat::try_from(self.decades) {
                return Ok(LogScale {
                    decades,
                    offset: PositiveFloat::unit(),
                });
            }
        }
        Err(self)
    }
}

impl fmt::Display for LogRangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "decades/offset must both be positive, got '{},{}'",
            self.decades, self.offset,
        )
    }
}
