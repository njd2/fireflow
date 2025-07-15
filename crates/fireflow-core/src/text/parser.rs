use crate::config::TimePattern;
use crate::core::*;
use crate::error::*;
use crate::validated::keys::*;
use crate::validated::shortname::*;

use super::byteord::*;
use super::compensation::*;
use super::datetimes::*;
use super::index::*;
use super::keywords::*;
use super::named_vec::*;
use super::optional::*;
use super::ranged_float::RangedFloatError;
use super::scale::*;
use super::spillover::*;
use super::timestamps::*;
use super::unstainedcenters::*;

use bigdecimal::ParseBigDecimalError;
use derive_more::{Display, From};
use itertools::Itertools;
use nonempty::NonEmpty;
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::fmt;
use std::num::{ParseFloatError, ParseIntError};
use std::str::FromStr;

/// Any required key
pub(crate) trait Required {
    fn get_req<V>(kws: &StdKeywords, k: StdKey) -> ReqResult<V>
    where
        V: FromStr,
    {
        get_req(kws, k)
    }

    fn remove_req<V>(kws: &mut StdKeywords, k: StdKey) -> ReqResult<V>
    where
        V: FromStr,
    {
        remove_req(kws, k)
    }
}

/// Any optional key
pub(crate) trait Optional {
    fn get_opt<V>(kws: &StdKeywords, k: StdKey) -> OptKwResult<V>
    where
        V: FromStr,
    {
        get_opt(kws, k).map(|x| x.into())
    }

    fn remove_opt<V>(
        kws: &mut StdKeywords,
        k: StdKey,
    ) -> Result<MaybeValue<V>, ParseKeyError<V::Err>>
    where
        V: FromStr,
    {
        remove_opt(kws, k).map(|x| x.into())
    }
}

/// A required metaroot key
pub(crate) trait ReqMetarootKey
where
    Self: Required,
    Self: fmt::Display,
    Self: Key,
    Self: FromStr,
{
    fn get_metaroot_req(kws: &StdKeywords) -> ReqResult<Self> {
        Self::get_req(kws, Self::std())
    }

    fn remove_metaroot_req(kws: &mut StdKeywords) -> ReqResult<Self> {
        Self::remove_req(kws, Self::std())
    }

    fn lookup_req(kws: &mut StdKeywords) -> LookupResult<Self>
    where
        ParseReqKeyError: From<<Self as FromStr>::Err>,
    {
        Self::remove_metaroot_req(kws)
            .map_err(|e| e.inner_into())
            .map_err(Box::new)
            .into_deferred()
    }

    fn pair(&self) -> (String, String) {
        (Self::std().to_string(), self.to_string())
    }
}

/// Any required key with one index
pub(crate) trait ReqIndexedKey
where
    Self: Required,
    Self: fmt::Display,
    Self: IndexedKey,
    Self: FromStr,
{
    fn get_meas_req(kws: &StdKeywords, i: IndexFromOne) -> ReqResult<Self> {
        Self::get_req(kws, Self::std(i))
    }

    fn remove_meas_req(kws: &mut StdKeywords, i: IndexFromOne) -> ReqResult<Self> {
        Self::remove_req(kws, Self::std(i))
    }

    fn lookup_req(kws: &mut StdKeywords, i: IndexFromOne) -> LookupResult<Self>
    where
        ParseReqKeyError: From<<Self as FromStr>::Err>,
    {
        Self::remove_meas_req(kws, i)
            .map_err(|e| e.inner_into())
            .map_err(Box::new)
            .into_deferred()
    }

    fn triple(&self, i: IndexFromOne) -> (MeasHeader, String, String) {
        (
            Self::std_blank(),
            Self::std(i).to_string(),
            self.to_string(),
        )
    }

    fn pair(&self, i: IndexFromOne) -> (String, String) {
        let (_, k, v) = self.triple(i);
        (k, v)
    }
}

/// An optional metaroot key
pub(crate) trait OptMetarootKey
where
    Self: Optional,
    Self: fmt::Display,
    Self: Key,
    Self: FromStr,
{
    fn get_metaroot_opt(kws: &StdKeywords) -> OptKwResult<Self> {
        Self::get_opt(kws, Self::std())
    }

    fn remove_metaroot_opt(kws: &mut StdKeywords) -> OptKwResult<Self> {
        Self::remove_opt(kws, Self::std())
    }

    fn lookup_opt<E>(kws: &mut StdKeywords) -> LookupTentative<MaybeValue<Self>, E>
    where
        ParseOptKeyWarning: From<<Self as FromStr>::Err>,
    {
        process_opt(Self::remove_metaroot_opt(kws))
    }

    fn lookup_opt_dep(
        kws: &mut StdKeywords,
        disallow_dep: bool,
    ) -> LookupTentative<MaybeValue<Self>, DeprecatedError>
    where
        ParseOptKeyWarning: From<<Self as FromStr>::Err>,
    {
        let mut x = Self::lookup_opt(kws);
        eval_dep_maybe(&mut x, Self::std(), disallow_dep);
        x
    }

    fn pair_opt(opt: &MaybeValue<Self>) -> (String, Option<String>) {
        (
            Self::std().to_string(),
            opt.0.as_ref().map(|s| s.to_string()),
        )
    }

    fn pair(opt: &Self) -> (String, String) {
        (Self::std().to_string(), opt.to_string())
    }
}

/// Any optional key with an index
pub(crate) trait OptIndexedKey
where
    Self: Optional,
    Self: fmt::Display,
    Self: IndexedKey,
    Self: FromStr,
{
    fn get_meas_opt(kws: &StdKeywords, i: IndexFromOne) -> OptKwResult<Self> {
        Self::get_opt(kws, Self::std(i))
    }

    fn remove_meas_opt(kws: &mut StdKeywords, i: IndexFromOne) -> OptKwResult<Self> {
        Self::remove_opt(kws, Self::std(i))
    }

    fn lookup_opt<E>(
        kws: &mut StdKeywords,
        i: IndexFromOne,
        // dep: bool,
    ) -> LookupTentative<MaybeValue<Self>, E>
    where
        ParseOptKeyWarning: From<<Self as FromStr>::Err>,
    {
        process_opt(Self::remove_meas_opt(kws, i))
    }

    fn lookup_opt_dep(
        kws: &mut StdKeywords,
        i: IndexFromOne,
        disallow_dep: bool,
    ) -> LookupTentative<MaybeValue<Self>, DeprecatedError>
    where
        ParseOptKeyWarning: From<<Self as FromStr>::Err>,
    {
        let mut x = Self::lookup_opt(kws, i);
        eval_dep_maybe(&mut x, Self::std(i), disallow_dep);
        x
    }

    fn triple(opt: &MaybeValue<Self>, i: IndexFromOne) -> (MeasHeader, String, Option<String>) {
        (
            Self::std_blank(),
            Self::std(i).to_string(),
            opt.0.as_ref().map(|s| s.to_string()),
        )
    }

    fn pair_opt(opt: &MaybeValue<Self>, i: IndexFromOne) -> (String, Option<String>) {
        let (_, k, v) = Self::triple(opt, i);
        (k, v)
    }

    fn pair(&self, i: IndexFromOne) -> (String, String) {
        (Self::std(i).to_string(), self.to_string())
    }
}

/// Any key which references $PnN in its value
pub(crate) trait OptLinkedKey
where
    Self: Key,
    Self: Optional,
    Self: fmt::Display,
    Self: FromStr,
    Self: Sized,
{
    fn check_link(&self, names: &HashSet<&Shortname>) -> Result<(), LinkedNameError> {
        NonEmpty::collect(self.names().difference(names).copied().cloned())
            .map(|common_names| LinkedNameError {
                names: common_names,
                key: Self::std(),
            })
            .map(Err)
            .unwrap_or(Ok(()))
    }

    fn lookup_opt<E>(
        kws: &mut StdKeywords,
        names: &HashSet<&Shortname>,
    ) -> LookupTentative<MaybeValue<Self>, E>
    where
        ParseOptKeyWarning: From<<Self as FromStr>::Err>,
    {
        process_opt(Self::remove_opt(kws, Self::std())).and_tentatively(|maybe| {
            if let Some(x) = maybe.0 {
                Self::check_link(&x, names).map_or_else(
                    |w| Tentative::new(None, vec![w.into()], vec![]),
                    |_| Tentative::new1(Some(x)),
                )
            } else {
                Tentative::new1(None)
            }
            .map(|x| x.into())
        })
    }

    // TODO not DRY
    fn pair_opt(opt: &MaybeValue<Self>) -> (String, Option<String>) {
        (
            Self::std().to_string(),
            opt.0.as_ref().map(|s| s.to_string()),
        )
    }

    fn reassign(&mut self, mapping: &NameMapping);

    fn names(&self) -> HashSet<&Shortname>;
}

/// Find a required standard key in a hash table
pub(crate) fn get_req<T>(kws: &StdKeywords, k: StdKey) -> ReqResult<T>
where
    T: FromStr,
{
    match kws.get(&k) {
        Some(v) => v
            .parse()
            .map_err(|error| ParseKeyError {
                error,
                key: k,
                value: v.clone(),
            })
            .map_err(ReqKeyError::Parse),
        None => Err(ReqKeyError::Missing(k)),
    }
}

/// Find an optional standard key in a hash table
pub(crate) fn get_opt<T>(kws: &StdKeywords, k: StdKey) -> OptResult<T>
where
    T: FromStr,
{
    kws.get(&k)
        .map(|v| {
            v.parse().map_err(|error| ParseKeyError {
                error,
                key: k,
                value: v.clone(),
            })
        })
        .transpose()
}

/// Find a required standard key in a hash table and remove it
pub(crate) fn remove_req<T>(kws: &mut StdKeywords, k: StdKey) -> ReqResult<T>
where
    T: FromStr,
{
    match kws.remove(&k) {
        Some(v) => v
            .parse()
            .map_err(|error| ParseKeyError {
                error,
                key: k,
                value: v,
            })
            .map_err(ReqKeyError::Parse),
        None => Err(ReqKeyError::Missing(k)),
    }
}

/// Find an optional standard key in a hash table and remove it
pub(crate) fn remove_opt<T>(kws: &mut StdKeywords, k: StdKey) -> OptResult<T>
where
    T: FromStr,
{
    kws.remove(&k)
        .map(|v| {
            v.parse().map_err(|error| ParseKeyError {
                error,
                key: k,
                value: v,
            })
        })
        .transpose()
}

pub struct LinkedNameError {
    pub key: StdKey,
    pub names: NonEmpty<Shortname>,
}

impl fmt::Display for LinkedNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        let ns = if self.names.tail.is_empty() {
            "name"
        } else {
            "names"
        };
        let bad = self.names.iter().join(", ");
        write!(f, "{} references non-existent $PnN {ns}: {bad}", self.key)
    }
}

pub(crate) fn lookup_temporal_gain_3_0(
    kws: &mut StdKeywords,
    i: IndexFromOne,
) -> LookupTentative<MaybeValue<Gain>, LookupKeysError> {
    let mut tnt_gain = Gain::lookup_opt(kws, i);
    tnt_gain.eval_error(|gain| {
        if gain.0.is_some() {
            Some(LookupKeysError::Misc(TemporalError::HasGain.into()))
        } else {
            None
        }
    });
    tnt_gain
}

pub(crate) fn process_opt_dep<V>(
    res: Result<MaybeValue<V>, ParseKeyError<<V as FromStr>::Err>>,
    k: StdKey,
    disallow_dep: bool,
) -> Tentative<MaybeValue<V>, LookupKeysWarning, DeprecatedError>
where
    V: FromStr,
    ParseOptKeyWarning: From<<V as FromStr>::Err>,
{
    let mut x = process_opt(res);
    eval_dep_maybe(&mut x, k, disallow_dep);
    x
}

pub(crate) fn process_opt<V, E>(
    res: Result<MaybeValue<V>, ParseKeyError<<V as FromStr>::Err>>,
) -> Tentative<MaybeValue<V>, LookupKeysWarning, E>
where
    V: FromStr,
    ParseOptKeyWarning: From<<V as FromStr>::Err>,
{
    res.map_or_else(
        |e| {
            Tentative::new(
                None.into(),
                vec![LookupKeysWarning::Parse(e.inner_into())],
                vec![],
            )
        },
        Tentative::new1,
    )
}

pub(crate) type RawKeywords = HashMap<String, String>;

pub(crate) type ReqResult<T> = Result<T, ReqKeyError<<T as FromStr>::Err>>;
pub(crate) type OptResult<T> = Result<Option<T>, ParseKeyError<<T as FromStr>::Err>>;
pub(crate) type OptKwResult<T> = Result<MaybeValue<T>, ParseKeyError<<T as FromStr>::Err>>;

pub(crate) type LookupResult<V> = DeferredResult<V, LookupKeysWarning, LookupKeysError>;
pub(crate) type LookupTentative<V, E> = Tentative<V, LookupKeysWarning, E>;
pub(crate) type LookupOptional<V, E> = Tentative<MaybeValue<V>, LookupKeysWarning, E>;

// TODO this could be nested better
#[derive(From, Display)]
pub enum LookupKeysError {
    Parse(Box<ReqKeyError<ParseReqKeyError>>),
    Dep(DeprecatedError),
    Misc(LookupMiscError),
}

#[derive(From, Display)]
pub enum LookupKeysWarning {
    Parse(ParseKeyError<ParseOptKeyWarning>),
    Relation(LookupRelationalWarning),
    Linked(LinkedNameError),
    Dep(DeprecatedError),
}

#[derive(From, Display)]
pub enum DeprecatedError {
    Key(DepKeyWarning),
    Value(DepValueWarning),
}

/// Error encountered when parsing a required key from a string
#[derive(From, Display)]
pub enum ParseReqKeyError {
    Range(ParseBigDecimalError),
    AlphaNumType(AlphaNumTypeError),
    String(Infallible),
    Int(ParseIntError),
    Scale(ScaleError),
    TemporalScale(TemporalScaleError),
    RangedFloat(RangedFloatError),
    Mode(ModeError),
    ByteOrd(ParseByteOrdError),
    Endian(NewEndianError),
    Shortname(ShortnameError),
}

/// Error encountered when parsing an optional key from a string
#[derive(From, Display)]
pub enum ParseOptKeyWarning {
    NumType(NumTypeError),
    Trigger(TriggerError),
    Scale(ScaleError),
    TemporalScale(TemporalScaleError),
    Float(ParseFloatError),
    RangedFloat(RangedFloatError),
    Feature(FeatureError),
    Wavelengths(WavelengthsError),
    Calibration3_1(CalibrationError<CalibrationFormat3_1>),
    Calibration3_2(CalibrationError<CalibrationFormat3_2>),
    Int(ParseIntError),
    String(Infallible),
    FCSDate(FCSDateError),
    FCSTime(FCSTimeError),
    FCSTime60(FCSTime60Error),
    FCSTime100(FCSTime100Error),
    FCSDateTime(FCSDateTimeError),
    ModifiedDateTime(ModifiedDateTimeError),
    Originality(OriginalityError),
    UnstainedCenter(ParseUnstainedCenterError),
    Mode3_2(Mode3_2Error),
    TemporalType(TemporalTypeError),
    OpticalType(OpticalTypeError),
    Shortname(ShortnameError),
    Display(DisplayError),
    Unicode(UnicodeError),
    Spillover(ParseSpilloverError),
    Compensation(ParseCompError),
    GateRange(ParseBigDecimalError),
    GateRegionIndex2_0(RegionGateIndexError<ParseIntError>),
    GateRegionIndex3_0(RegionGateIndexError<MeasOrGateIndexError>),
    GateRegionIndex3_2(RegionGateIndexError<PrefixedMeasIndexError>),
    GateRegionWindow(GatePairError),
    Gating(GatingError),
}

/// Misc errors encountered when looking up keywords for standardization
#[derive(From, Display)]
pub enum LookupMiscError {
    // TODO this should be a configurable warning
    Temporal(TemporalError),
    NamedVec(NewNamedVecError),
    MissingTime(MissingTime),
}

/// Error triggered when time measurement is missing but required.
pub struct MissingTime(pub TimePattern);

/// Errors triggered when time measurement keyword value is invalid
// TODO add other optical keywords that shouldn't be set for time.
pub enum TemporalError {
    NonLinear,
    HasGain,
}

/// Error encountered when relation between two or more keys is invalid
#[derive(From, Display)]
pub enum LookupRelationalWarning {
    Timestamp(ReversedTimestamps),
    Datetime(ReversedDatetimes),
    CompShape(NewCompError),
    GateRegion(MismatchedIndexAndWindowError),
    GateRegionLink(GateRegionLinkError),
    GateMeasLink(GateMeasurementLinkError),
}

/// Error/warning triggered when encountering a key which is deprecated
pub struct DepKeyWarning(pub StdKey);

/// Error/warning triggered when encountering a key value which is deprecated
pub enum DepValueWarning {
    DatatypeASCII,
    ModeCorrelated,
    ModeUncorrelated,
}

pub struct MismatchedIndexAndWindowError;

impl fmt::Display for MismatchedIndexAndWindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "values for $RnI and $RnW must both be univariate or bivariate"
        )
    }
}

impl fmt::Display for DepValueWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        let s = match self {
            Self::DatatypeASCII => "$DATATYPE=A is deprecated",
            Self::ModeCorrelated => "$MODE=C is deprecated",
            Self::ModeUncorrelated => "$MODE=U is deprecated",
        };
        write!(f, "{s}")
    }
}

impl fmt::Display for DepKeyWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "deprecated key: {}", self.0)
    }
}

impl fmt::Display for MissingTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "Could not find time measurement matching {}", self.0)
    }
}

impl fmt::Display for TemporalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        // TODO include meas idx here
        match self {
            TemporalError::NonLinear => write!(f, "$PnE must be '0,0'"),
            TemporalError::HasGain => write!(f, "$PnG must not be set"),
        }
    }
}

/// Error denoting that pseudostandard keyword was found.
pub struct PseudostandardError(pub StdKey);

impl fmt::Display for PseudostandardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "pseudostandard keyword found: {}", self.0)
    }
}

pub struct ParseKeyError<E> {
    pub error: E,
    pub key: StdKey,
    pub value: String,
}

impl<E> ParseKeyError<E> {
    pub fn inner_into<F>(self) -> ParseKeyError<F>
    where
        F: From<E>,
    {
        ParseKeyError {
            error: self.error.into(),
            key: self.key,
            value: self.value,
        }
    }

    pub fn with_error<F, X>(self, f: F) -> Result<X, Self>
    where
        F: FnOnce(E) -> Result<X, E>,
    {
        f(self.error).map_err(|error| Self {
            error,
            key: self.key,
            value: self.value,
        })
    }
}

impl<E> fmt::Display for ParseKeyError<E>
where
    E: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "{} (key='{}', value='{}')",
            self.error, self.key, self.value
        )
    }
}

pub enum ReqKeyError<E> {
    Parse(ParseKeyError<E>),
    Missing(StdKey),
}

impl<E> ReqKeyError<E> {
    pub fn inner_into<F>(self) -> ReqKeyError<F>
    where
        F: From<E>,
    {
        match self {
            ReqKeyError::Parse(e) => ReqKeyError::Parse(e.inner_into()),
            ReqKeyError::Missing(e) => ReqKeyError::Missing(e),
        }
    }

    pub fn with_parse_error<F, X>(self, f: F) -> Result<X, Self>
    where
        F: FnOnce(E) -> Result<X, E>,
    {
        match self {
            ReqKeyError::Parse(p) => p.with_error(f).map_err(ReqKeyError::Parse),
            ReqKeyError::Missing(m) => Err(ReqKeyError::Missing(m)),
        }
    }
}

impl<E> fmt::Display for ReqKeyError<E>
where
    E: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            ReqKeyError::Parse(x) => x.fmt(f),
            ReqKeyError::Missing(k) => write!(f, "missing required key: {k}"),
        }
    }
}

fn eval_dep_maybe<T>(
    x: &mut LookupTentative<MaybeValue<T>, DeprecatedError>,
    key: StdKey,
    disallow_dep: bool,
) {
    if disallow_dep {
        x.eval_error(|v| eval_dep(v, key));
    } else {
        x.eval_warning(|v| eval_dep(v, key).map(|e| e.into()));
    }
}

fn eval_dep<T>(v: &MaybeValue<T>, key: StdKey) -> Option<DeprecatedError> {
    if v.0.is_some() {
        Some(DeprecatedError::Key(DepKeyWarning(key)))
    } else {
        None
    }
}
