use crate::core::{AnyMetarootKeyLossError, UnitaryKeyLossError};
use crate::error::*;
use crate::validated::keys::*;

use super::optional::MaybeValue;
use super::parser::*;

use chrono::{DateTime, FixedOffset};
use derive_more::{Display, From, FromStr, Into};
use serde::Serialize;
use std::fmt;
use std::str::FromStr;

/// A convenient bundle for the $BEGINDATETIME and $ENDDATETIME keys (3.2+)
#[derive(Clone, Serialize, Default)]
pub struct Datetimes {
    /// Value for the $BEGINDATETIME key.
    begin: Option<BeginDateTime>,

    /// Value for the $ENDDATETIME key.
    end: Option<EndDateTime>,
}

#[derive(Clone, Copy, Serialize, From, Into, Display, FromStr)]
pub struct BeginDateTime(pub FCSDateTime);

#[derive(Clone, Copy, Serialize, From, Into, Display, FromStr)]
pub struct EndDateTime(pub FCSDateTime);

/// A datetime as used in the $(BEGIN|END)DATETIME keys (3.2+ only)
#[derive(Clone, Copy, Serialize, From, Into)]
pub struct FCSDateTime(pub DateTime<FixedOffset>);

macro_rules! get_set {
    ($fn_get_naive:ident, $fn:ident, $fn_naive:ident, $in:path, $field:ident) => {
        pub fn $field(&self) -> MaybeValue<$in> {
            MaybeValue(self.$field)
        }

        pub fn $fn_get_naive(&self) -> Option<DateTime<FixedOffset>> {
            self.$field().0.map(|x| x.0.into())
        }

        pub fn $fn(&mut self, x: MaybeValue<$in>) -> DatetimesResult<()> {
            let tmp = self.$field;
            self.$field = x.0;
            if self.valid() {
                Ok(())
            } else {
                self.$field = tmp;
                Err(ReversedDatetimes)
            }
        }

        pub fn $fn_naive(&mut self, x: Option<DateTime<FixedOffset>>) -> DatetimesResult<()> {
            self.$fn(x.map(|y| FCSDateTime(y).into()).into())
        }
    };
}

impl Datetimes {
    pub fn try_new(
        begin: MaybeValue<BeginDateTime>,
        end: MaybeValue<EndDateTime>,
    ) -> DatetimesResult<Self> {
        let ret = Self {
            begin: begin.0,
            end: end.0,
        };
        if ret.valid() {
            Ok(ret)
        } else {
            Err(ReversedDatetimes)
        }
    }

    get_set!(
        begin_naive,
        set_begin,
        set_begin_naive,
        BeginDateTime,
        begin
    );

    get_set!(end_naive, set_end, set_end_naive, EndDateTime, end);

    pub fn valid(&self) -> bool {
        if let (Some(b), Some(e)) = (&self.begin, &self.end) {
            (b.0).0 < (e.0).0
        } else {
            true
        }
    }

    pub(crate) fn lookup<E>(kws: &mut StdKeywords) -> LookupTentative<Self, E> {
        let b = BeginDateTime::lookup_opt(kws);
        let e = EndDateTime::lookup_opt(kws);
        b.zip(e).and_tentatively(|(begin, end)| {
            Datetimes::try_new(begin, end)
                .map(Tentative::new1)
                .unwrap_or_else(|w| {
                    let ow = LookupKeysWarning::Relation(w.into());
                    Tentative::new(Datetimes::default(), vec![ow], vec![])
                })
        })
    }

    pub(crate) fn opt_keywords(&self) -> impl Iterator<Item = (String, String)> {
        [
            OptMetarootKey::pair_opt(&self.begin()),
            OptMetarootKey::pair_opt(&self.end()),
        ]
        .into_iter()
        .flat_map(|(k, v)| v.map(|x| (k, x)))
    }

    pub(crate) fn check_loss(self, lossless: bool) -> BiTentative<(), AnyMetarootKeyLossError> {
        let mut tnt = Tentative::new1(());
        if self.begin_naive().is_some() {
            tnt.push_error_or_warning(UnitaryKeyLossError::<BeginDateTime>::default(), lossless);
        }
        if self.end_naive().is_some() {
            tnt.push_error_or_warning(UnitaryKeyLossError::<EndDateTime>::default(), lossless);
        }
        tnt
    }
}

pub struct ReversedDatetimes;

type DatetimesResult<T> = Result<T, ReversedDatetimes>;

impl fmt::Display for ReversedDatetimes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "$BEGINDATETIME is after $ENDDATETIME")
    }
}

impl FromStr for FCSDateTime {
    type Err = FCSDateTimeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let formats = [
            "%Y-%m-%dT%H:%M:%S%.f",
            "%Y-%m-%dT%H:%M:%S%.f%#z",
            "%Y-%m-%dT%H:%M:%S%.f%:z",
            "%Y-%m-%dT%H:%M:%S%.f%::z",
            "%Y-%m-%dT%H:%M:%S%.f%:::z",
        ];
        for f in formats {
            if let Ok(t) = DateTime::parse_from_str(s, f) {
                return Ok(FCSDateTime(t));
            }
        }
        Err(FCSDateTimeError)
    }
}

impl fmt::Display for FCSDateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "{}", self.0.format("%Y-%m-%dT%H:%M:%S%.f%:z"))
    }
}

pub struct FCSDateTimeError;

impl fmt::Display for FCSDateTimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "must be formatted like 'yyyy-mm-ddThh:mm:ss[TZD]'")
    }
}
