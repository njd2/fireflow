use crate::macros::{newtype_disp, newtype_from, newtype_from_outer, newtype_fromstr};
use crate::validated::nonstandard::*;
use crate::validated::shortname::*;

use super::byteord::*;
use super::compensation::*;
use super::datetimes::*;
use super::modified_date_time::*;
use super::named_vec::NameMapping;
use super::optionalkw::*;
use super::range::*;
use super::ranged_float::*;
use super::scale::*;
use super::spillover::*;
use super::timestamps::*;
use super::unstainedcenters::*;

use chrono::NaiveTime;
use itertools::Itertools;
use nonempty::NonEmpty;
use serde::Serialize;
use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::fmt;
use std::num::{ParseFloatError, ParseIntError};
use std::str::{from_utf8, FromStr};

// TODO validate
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct StdKey(pub Vec<u8>);

impl Serialize for StdKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let s = self.to_string();
        serializer.collect_str(s.as_str())
    }
}

pub(crate) type StdKeywords = HashMap<StdKey, String>;

impl fmt::Display for StdKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        // TODO use unchecked here
        write!(f, "${}", from_utf8(&self.0[..]).unwrap())
    }
}

impl Borrow<[u8]> for StdKey {
    fn borrow(&self) -> &[u8] {
        self.0.borrow()
    }
}

impl AsRef<[u8]> for StdKey {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

/// The value of the $PnG keyword
#[derive(Clone, Copy, Serialize, PartialEq)]
pub struct Gain(pub PositiveFloat);

newtype_from!(Gain, PositiveFloat);
newtype_disp!(Gain);
newtype_fromstr!(Gain, RangedFloatError);

/// The value of the $TIMESTEP keyword
#[derive(Clone, Copy, Serialize)]
pub struct Timestep(pub PositiveFloat);

impl Default for Timestep {
    fn default() -> Self {
        // ASSUME this will never panic...1.0 is still a positive number, right?
        Timestep(PositiveFloat::try_from(1.0).ok().unwrap())
    }
}

newtype_disp!(Timestep);
newtype_fromstr!(Timestep, RangedFloatError);
newtype_from!(Timestep, PositiveFloat);

/// The value of the $VOL keyword
#[derive(Clone, Copy, Serialize)]
pub struct Vol(pub NonNegFloat);

newtype_from!(Vol, NonNegFloat);
newtype_disp!(Vol);
newtype_fromstr!(Vol, RangedFloatError);

/// The value of the $TR field (all versions)
///
/// This is formatted as 'string,f' where 'string' is a measurement name.
#[derive(Clone, Serialize)]
pub(crate) struct Trigger {
    /// The measurement name (assumed to match a '$PnN' value).
    pub measurement: Shortname,

    /// The threshold of the trigger.
    pub threshold: u32,
}

pub enum TriggerError {
    WrongFieldNumber,
    IntFormat(std::num::ParseIntError),
}

impl FromStr for Trigger {
    type Err = TriggerError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.split(",").collect::<Vec<_>>()[..] {
            [p, n1] => n1
                .parse()
                .map_err(TriggerError::IntFormat)
                .map(|threshold| Trigger {
                    measurement: Shortname::new_unchecked(p),
                    threshold,
                }),
            _ => Err(TriggerError::WrongFieldNumber),
        }
    }
}

impl fmt::Display for Trigger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "{},{}", self.measurement, self.threshold)
    }
}

impl fmt::Display for TriggerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            TriggerError::WrongFieldNumber => write!(f, "must be like 'string,f'"),
            TriggerError::IntFormat(i) => write!(f, "{}", i),
        }
    }
}

impl Linked for Trigger {
    fn names(&self) -> HashSet<&Shortname> {
        [&self.measurement].into_iter().collect()
    }

    fn reassign(&mut self, mapping: &NameMapping) {
        if let Some(new) = mapping.get(&self.measurement) {
            self.measurement = (*new).clone();
        }
    }
}

/// The values used for the $MODE key (up to 3.1)
#[derive(Clone, PartialEq, Eq, Serialize)]
pub enum Mode {
    List,
    Uncorrelated,
    Correlated,
}

pub struct ModeError;

impl FromStr for Mode {
    type Err = ModeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "C" => Ok(Mode::Correlated),
            "L" => Ok(Mode::List),
            "U" => Ok(Mode::Uncorrelated),
            _ => Err(ModeError),
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        let x = match self {
            Mode::Correlated => "C",
            Mode::List => "L",
            Mode::Uncorrelated => "U",
        };
        write!(f, "{}", x)
    }
}

impl fmt::Display for ModeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "must be one of 'C', 'L', or 'U'")
    }
}

/// The value for the $MODE key, which can only contain 'L' (3.2)
pub struct Mode3_2;

pub struct Mode3_2Error;

impl FromStr for Mode3_2 {
    type Err = Mode3_2Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "L" => Ok(Mode3_2),
            _ => Err(Mode3_2Error),
        }
    }
}

impl fmt::Display for Mode3_2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "L")
    }
}
impl fmt::Display for Mode3_2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "can only be 'L'")
    }
}

/// The value for the $PnDISPLAY key (3.1+)
#[derive(Clone, Copy, Serialize)]
pub enum Display {
    /// Linear display (value like 'Linear,<lower>,<upper>')
    Lin { lower: f32, upper: f32 },

    // TODO not clear if these can be <0
    /// Logarithmic display (value like 'Logarithmic,<offset>,<decades>')
    Log { offset: f32, decades: f32 },
}

impl FromStr for Display {
    type Err = DisplayError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.split(",").collect::<Vec<_>>()[..] {
            [which, s1, s2] => {
                let f1 = s1.parse().map_err(DisplayError::FloatError)?;
                let f2 = s2.parse().map_err(DisplayError::FloatError)?;
                match which {
                    "Linear" => Ok(Display::Lin {
                        lower: f1,
                        upper: f2,
                    }),
                    "Logarithmic" => Ok(Display::Log {
                        decades: f1,
                        offset: f2,
                    }),
                    _ => Err(DisplayError::InvalidType),
                }
            }
            _ => Err(DisplayError::FormatError),
        }
    }
}

impl fmt::Display for Display {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            Display::Lin { lower, upper } => write!(f, "Linear,{lower},{upper}"),
            Display::Log { offset, decades } => write!(f, "Log,{offset},{decades}"),
        }
    }
}

pub enum DisplayError {
    FloatError(ParseFloatError),
    InvalidType,
    FormatError,
}

impl fmt::Display for DisplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            DisplayError::FloatError(x) => write!(f, "{}", x),
            DisplayError::InvalidType => write!(f, "Type must be either 'Logarithmic' or 'Linear'"),
            DisplayError::FormatError => write!(f, "must be like 'string,f1,f2'"),
        }
    }
}

/// The three values for the $PnDATATYPE keyword (3.2+)
#[derive(Clone, Copy, Serialize)]
pub enum NumType {
    Integer,
    Single,
    Double,
}

impl FromStr for NumType {
    type Err = NumTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "I" => Ok(NumType::Integer),
            "F" => Ok(NumType::Single),
            "D" => Ok(NumType::Double),
            _ => Err(NumTypeError),
        }
    }
}

impl fmt::Display for NumType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            NumType::Integer => write!(f, "I"),
            NumType::Single => write!(f, "F"),
            NumType::Double => write!(f, "D"),
        }
    }
}

pub struct NumTypeError;

impl fmt::Display for NumTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "must be one of 'F', 'D', or 'A'")
    }
}

/// The four allowed values for the $DATATYPE keyword.
#[derive(Clone, Copy, Eq, PartialEq, PartialOrd, Ord, Hash, Serialize)]
pub enum AlphaNumType {
    Ascii,
    Integer,
    Single,
    Double,
}

impl FromStr for AlphaNumType {
    type Err = AlphaNumTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "I" => Ok(AlphaNumType::Integer),
            "F" => Ok(AlphaNumType::Single),
            "D" => Ok(AlphaNumType::Double),
            "A" => Ok(AlphaNumType::Ascii),
            _ => Err(AlphaNumTypeError),
        }
    }
}

impl fmt::Display for AlphaNumType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            AlphaNumType::Ascii => write!(f, "A"),
            AlphaNumType::Integer => write!(f, "I"),
            AlphaNumType::Single => write!(f, "F"),
            AlphaNumType::Double => write!(f, "D"),
        }
    }
}

pub struct AlphaNumTypeError;

impl fmt::Display for AlphaNumTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "must be one of 'I', 'F', 'D', or 'A'")
    }
}

impl From<NumType> for AlphaNumType {
    fn from(value: NumType) -> Self {
        match value {
            NumType::Integer => AlphaNumType::Integer,
            NumType::Single => AlphaNumType::Single,
            NumType::Double => AlphaNumType::Double,
        }
    }
}

impl TryFrom<AlphaNumType> for NumType {
    type Error = ();
    fn try_from(value: AlphaNumType) -> Result<Self, Self::Error> {
        match value {
            AlphaNumType::Integer => Ok(NumType::Integer),
            AlphaNumType::Single => Ok(NumType::Single),
            AlphaNumType::Double => Ok(NumType::Double),
            AlphaNumType::Ascii => Err(()),
        }
    }
}

/// The value for the $PnCALIBRATION key (3.1 only)
///
/// This should be formatted like '<value>,<unit>'
#[derive(Clone, Serialize)]
pub struct Calibration3_1 {
    pub value: f32,
    pub unit: String,
}

impl FromStr for Calibration3_1 {
    type Err = CalibrationError<CalibrationFormat3_1>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.split(",").collect::<Vec<_>>()[..] {
            [svalue, unit] => {
                let value = svalue.parse().map_err(CalibrationError::Float)?;
                if value >= 0.0 {
                    Ok(Calibration3_1 {
                        value,
                        unit: String::from(unit),
                    })
                } else {
                    Err(CalibrationError::Range)
                }
            }
            _ => Err(CalibrationError::Format(CalibrationFormat3_1)),
        }
    }
}

impl fmt::Display for Calibration3_1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "{},{}", self.value, self.unit)
    }
}

pub struct CalibrationFormat3_1;

impl fmt::Display for CalibrationFormat3_1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "must be like 'f,string'")
    }
}

pub enum CalibrationError<C> {
    Float(ParseFloatError),
    Range,
    Format(C),
}

impl<C: fmt::Display> fmt::Display for CalibrationError<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            CalibrationError::Float(x) => write!(f, "{}", x),
            CalibrationError::Range => write!(f, "must be a positive float"),
            CalibrationError::Format(x) => write!(f, "{}", x),
        }
    }
}

/// The value for the $PnCALIBRATION key (3.2+)
///
/// This should be formatted like '<value>,[<offset>,]<unit>' and differs from
/// 3.1 with the optional inclusion of "offset" (assumed 0 if not included).
#[derive(Clone, Serialize)]
pub struct Calibration3_2 {
    pub value: f32,
    pub offset: f32,
    pub unit: String,
}

impl FromStr for Calibration3_2 {
    type Err = CalibrationError<CalibrationFormat3_2>;

    // TODO not dry
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (value, offset, unit) = match s.split(",").collect::<Vec<_>>()[..] {
            [svalue, unit] => {
                let f1 = svalue.parse().map_err(CalibrationError::Float)?;
                Ok((f1, 0.0, String::from(unit)))
            }
            [svalue, soffset, unit] => {
                let f1 = svalue.parse().map_err(CalibrationError::Float)?;
                let f2 = soffset.parse().map_err(CalibrationError::Float)?;
                Ok((f1, f2, String::from(unit)))
            }
            _ => Err(CalibrationError::Format(CalibrationFormat3_2)),
        }?;
        if value >= 0.0 {
            Ok(Calibration3_2 {
                value,
                offset,
                unit,
            })
        } else {
            Err(CalibrationError::Range)
        }
    }
}

impl fmt::Display for Calibration3_2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "{},{},{}", self.value, self.offset, self.unit)
    }
}

pub struct CalibrationFormat3_2;

impl fmt::Display for CalibrationFormat3_2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "must be like 'f1,[f2],string'")
    }
}

/// The value for the $PnL key (3.1).
///
/// Starting in 3.1 this is a vector rather than a scaler.
#[derive(Clone)]
pub struct Wavelengths(pub NonEmpty<u32>);

impl Serialize for Wavelengths {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.iter().collect::<Vec<_>>().serialize(serializer)
    }
}

newtype_from!(Wavelengths, NonEmpty<u32>);
newtype_from_outer!(Wavelengths, NonEmpty<u32>);

impl fmt::Display for Wavelengths {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "{}", self.0.iter().join(","))
    }
}

impl FromStr for Wavelengths {
    type Err = WavelengthsError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut ws = vec![];
        for x in s.split(",") {
            ws.push(x.parse().map_err(WavelengthsError::Int)?);
        }
        NonEmpty::from_vec(ws)
            .ok_or(WavelengthsError::Empty)
            .map(Wavelengths)
    }
}

pub enum WavelengthsError {
    Int(ParseIntError),
    Empty,
}

impl fmt::Display for WavelengthsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            WavelengthsError::Int(i) => write!(f, "{}", i),
            WavelengthsError::Empty => write!(f, "list must not be empty"),
        }
    }
}

/// The value for the $ORIGINALITY key (3.1+)
#[derive(Clone, Copy, Serialize)]
pub enum Originality {
    Original,
    NonDataModified,
    Appended,
    DataModified,
}

impl FromStr for Originality {
    type Err = OriginalityError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Original" => Ok(Originality::Original),
            "NonDataModified" => Ok(Originality::NonDataModified),
            "Appended" => Ok(Originality::Appended),
            "DataModified" => Ok(Originality::DataModified),
            _ => Err(OriginalityError),
        }
    }
}

impl fmt::Display for Originality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        let x = match self {
            Originality::Appended => "Appended",
            Originality::Original => "Original",
            Originality::NonDataModified => "NonDataModified",
            Originality::DataModified => "DataModified",
        };
        write!(f, "{x}")
    }
}

pub struct OriginalityError;

impl fmt::Display for OriginalityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "Originality must be one of 'Original', 'NonDataModified', \
                   'Appended', or 'DataModified'"
        )
    }
}

/// The value of the $UNICODE key (3.0 only)
///
/// Formatted like 'codepage,[keys]'. This key is not actually used for anything
/// in this library and is present to be complete. The original purpose was to
/// indicate keywords which supported UTF-8, but these days it is hard to
/// write a library that does NOT support UTF-8 ;)
#[derive(Clone, Serialize)]
pub struct Unicode {
    page: u32,
    // TODO check that these are valid keywords (probably not worth it)
    kws: Vec<String>,
}

impl FromStr for Unicode {
    type Err = UnicodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut xs = s.split(",");
        if let Some(page) = xs.next().and_then(|x| x.parse().ok()) {
            let kws: Vec<String> = xs.map(String::from).collect();
            if kws.is_empty() {
                Err(UnicodeError::Empty)
            } else {
                Ok(Unicode { page, kws })
            }
        } else {
            Err(UnicodeError::BadFormat)
        }
    }
}

impl fmt::Display for Unicode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "{},{}", self.page, self.kws.iter().join(","))
    }
}

pub enum UnicodeError {
    Empty,
    BadFormat,
}

impl fmt::Display for UnicodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            UnicodeError::Empty => write!(f, "No keywords given"),
            UnicodeError::BadFormat => write!(f, "Must be like 'n,string,[[string],...]'"),
        }
    }
}

/// The value of the $PnTYPE key in optical channels (3.2+)
#[derive(Clone, Serialize, PartialEq)]
pub enum OpticalType {
    ForwardScatter,
    SideScatter,
    RawFluorescence,
    UnmixedFluorescence,
    Mass,
    ElectronicVolume,
    Classification,
    Index,
    // TODO is isn't clear if this is allowed according to the standard
    Other(String),
}

pub struct OpticalTypeError;

impl FromStr for OpticalType {
    type Err = OpticalTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Time" => Err(OpticalTypeError),
            "Forward Scatter" => Ok(OpticalType::ForwardScatter),
            "Side Scatter" => Ok(OpticalType::SideScatter),
            "Raw Fluorescence" => Ok(OpticalType::RawFluorescence),
            "Unmixed Fluorescence" => Ok(OpticalType::UnmixedFluorescence),
            "Mass" => Ok(OpticalType::Mass),
            "Electronic Volume" => Ok(OpticalType::ElectronicVolume),
            "Index" => Ok(OpticalType::Index),
            "Classification" => Ok(OpticalType::Classification),
            s => Ok(OpticalType::Other(String::from(s))),
        }
    }
}

impl fmt::Display for OpticalType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            OpticalType::ForwardScatter => write!(f, "Foward Scatter"),
            OpticalType::SideScatter => write!(f, "Side Scatter"),
            OpticalType::RawFluorescence => write!(f, "Raw Fluorescence"),
            OpticalType::UnmixedFluorescence => write!(f, "Unmixed Fluorescence"),
            OpticalType::Mass => write!(f, "Mass"),
            OpticalType::ElectronicVolume => write!(f, "Electronic Volume"),
            OpticalType::Classification => write!(f, "Classification"),
            OpticalType::Index => write!(f, "Index"),
            OpticalType::Other(s) => write!(f, "{}", s),
        }
    }
}

impl fmt::Display for OpticalTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "$PnTYPE for time measurement shall not be 'Time' if given"
        )
    }
}

/// The value of the $PnTYPE key in temporal channels (3.2+)
#[derive(Clone, Serialize, PartialEq)]
pub struct TemporalType;

pub struct TemporalTypeError;

impl FromStr for TemporalType {
    type Err = TemporalTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Time" => Ok(TemporalType),
            _ => Err(TemporalTypeError),
        }
    }
}

impl fmt::Display for TemporalType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "Time")
    }
}

impl fmt::Display for TemporalTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "$PnTYPE for time measurement shall be 'Time' if given")
    }
}

/// The value of the $PnFEATURE key (3.2+)
#[derive(Clone, Copy, Serialize)]
pub enum Feature {
    Area,
    Width,
    Height,
}

impl FromStr for Feature {
    type Err = FeatureError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Area" => Ok(Feature::Area),
            "Width" => Ok(Feature::Width),
            "Height" => Ok(Feature::Height),
            _ => Err(FeatureError),
        }
    }
}

impl fmt::Display for Feature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            Feature::Area => write!(f, "Area"),
            Feature::Width => write!(f, "Width"),
            Feature::Height => write!(f, "Height"),
        }
    }
}

pub struct FeatureError;

impl fmt::Display for FeatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "must be one of 'Area', 'Width', or 'Height'")
    }
}

/// The value of the $PnV key
#[derive(Clone, Copy, Serialize)]
pub struct DetectorVoltage(pub NonNegFloat);

newtype_from!(DetectorVoltage, NonNegFloat);
newtype_disp!(DetectorVoltage);
newtype_fromstr!(DetectorVoltage, RangedFloatError);

pub(crate) trait Key {
    const C: &'static str;

    fn std() -> StdKey {
        StdKey(Self::C.as_bytes().to_vec())
    }

    // fn nonstd() -> NonStdKey {
    //     NonStdKey::from_unchecked(Self::C)
    // }
}

pub(crate) trait IndexedKey {
    const PREFIX: &'static str;
    const SUFFIX: &'static str;

    fn std(i: MeasIdx) -> StdKey {
        StdKey(
            Self::PREFIX
                .as_bytes()
                .iter()
                .chain(i.to_string().as_bytes())
                .chain(Self::SUFFIX.as_bytes())
                .copied()
                .collect(),
        )
    }

    fn blank() -> String {
        format!("${}n{}", Self::PREFIX, Self::SUFFIX)
    }

    // fn fmt_sub() -> String {
    //     format!("{}%n{}", Self::PREFIX, Self::SUFFIX)
    // }

    // fn std(i: MeasIdx) -> String {
    //     format!("${}", Self::fmt(i))
    // }

    // fn std_blank() -> String {
    //     format!("${}", Self::fmt_blank())
    // }

    // fn nonstd(i: MeasIdx) -> NonStdKey {
    //     NonStdKey::from_unchecked(Self::fmt(i).as_str())
    // }

    // fn nonstd_sub() -> NonStdMeasKey {
    //     NonStdMeasKey::from_unchecked(Self::fmt_sub().as_str())
    // }

    // /// Return true if a key matches the prefix/suffix.
    // ///
    // /// Specifically, test if string is like <PREFIX><N><SUFFIX> where
    // /// N is an integer greater than zero.
    // fn matches(other: &str, std: bool) -> bool {
    //     if std {
    //         other.strip_prefix("$")
    //     } else {
    //         Some(other)
    //     }
    //     .and_then(|s| s.strip_prefix(Self::PREFIX))
    //     .and_then(|s| s.strip_suffix(Self::SUFFIX))
    //     .and_then(|s| s.parse::<u32>().ok())
    //     .is_some_and(|x| x > 0)
    // }
}

/// Raw TEXT key/value pairs
pub(crate) type RawKeywords = HashMap<String, String>;

type ReqResult<T> = Result<T, String>;
type OptResult<T> = Result<OptionalKw<T>, String>;

pub(crate) fn get_req<T>(kws: &StdKeywords, k: &StdKey) -> Result<T, String>
where
    T: FromStr,
    <T as FromStr>::Err: fmt::Display,
{
    match kws.get(k) {
        Some(v) => v
            .parse()
            .map_err(|e| format!("{e} (key='{k}', value='{v}')")),
        None => Err(format!("missing required key: {k}")),
    }
}

pub(crate) fn get_opt<T>(kws: &StdKeywords, k: &StdKey) -> Result<Option<T>, String>
where
    T: FromStr,
    <T as FromStr>::Err: fmt::Display,
{
    match kws.get(k) {
        Some(v) => v
            .parse()
            .map(Some)
            .map_err(|e| format!("{e} (key='{k}', value='{v}')")),
        None => Ok(None),
    }
}

// TODO not DRY
pub(crate) fn remove_req<T>(kws: &mut StdKeywords, k: &StdKey) -> Result<T, String>
where
    T: FromStr,
    <T as FromStr>::Err: fmt::Display,
{
    match kws.remove(k) {
        Some(v) => v
            .parse()
            .map_err(|e| format!("{e} (key='{k}', value='{v}')")),
        None => Err(format!("missing required key: {k}")),
    }
}

pub(crate) fn remove_opt<T>(kws: &mut StdKeywords, k: &StdKey) -> Result<Option<T>, String>
where
    T: FromStr,
    <T as FromStr>::Err: fmt::Display,
{
    match kws.remove(k) {
        Some(v) => v
            .parse()
            .map(Some)
            .map_err(|e| format!("{e} (key='{k}', value='{v}')")),
        None => Ok(None),
    }
}

pub(crate) trait Required {
    fn get_req<V>(kws: &StdKeywords, k: &StdKey) -> Result<V, String>
    where
        V: FromStr,
        <V as FromStr>::Err: fmt::Display,
    {
        get_req(kws, k)
    }

    fn remove_req<V>(kws: &mut StdKeywords, k: &StdKey) -> Result<V, String>
    where
        V: FromStr,
        <V as FromStr>::Err: fmt::Display,
    {
        remove_req(kws, k)
    }
}

pub(crate) trait Optional {
    fn get_opt<V>(kws: &StdKeywords, k: &StdKey) -> Result<OptionalKw<V>, String>
    where
        V: FromStr,
        <V as FromStr>::Err: fmt::Display,
    {
        get_opt(kws, k).map(|x| x.into())
    }

    fn remove_opt<V>(kws: &mut StdKeywords, k: &StdKey) -> Result<OptionalKw<V>, String>
    where
        V: FromStr,
        <V as FromStr>::Err: fmt::Display,
    {
        remove_opt(kws, k).map(|x| x.into())
    }
}

pub(crate) trait ReqMetaKey
where
    Self: Required,
    Self: fmt::Display,
    Self: Key,
    Self: FromStr,
    <Self as FromStr>::Err: fmt::Display,
{
    fn get_meta_req(kws: &StdKeywords) -> ReqResult<Self> {
        Self::get_req(kws, &Self::std())
    }

    fn remove_meta_req(kws: &mut StdKeywords) -> ReqResult<Self> {
        Self::remove_req(kws, &Self::std())
    }

    fn pair(&self) -> (String, String) {
        (Self::std().to_string(), self.to_string())
    }
}

pub(crate) trait ReqMeasKey
where
    Self: Required,
    Self: fmt::Display,
    Self: IndexedKey,
    Self: FromStr,
    <Self as FromStr>::Err: fmt::Display,
{
    fn get_meas_req(kws: &StdKeywords, n: MeasIdx) -> ReqResult<Self> {
        Self::get_req(kws, &Self::std(n))
    }

    fn remove_meas_req(kws: &mut StdKeywords, n: MeasIdx) -> ReqResult<Self> {
        Self::remove_req(kws, &Self::std(n))
    }

    fn triple(&self, n: MeasIdx) -> (String, String, String) {
        (Self::blank(), Self::std(n).to_string(), self.to_string())
    }

    fn pair(&self, n: MeasIdx) -> (String, String) {
        let (_, k, v) = self.triple(n);
        (k, v)
    }
}

pub(crate) trait OptMetaKey
where
    Self: Optional,
    Self: fmt::Display,
    Self: Key,
    Self: FromStr,
    <Self as FromStr>::Err: fmt::Display,
{
    fn get_meta_opt(kws: &StdKeywords) -> OptResult<Self> {
        Self::get_opt(kws, &Self::std())
    }

    fn remove_meta_opt(kws: &mut StdKeywords) -> OptResult<Self> {
        Self::remove_opt(kws, &Self::std())
    }

    fn pair(opt: &OptionalKw<Self>) -> (String, Option<String>) {
        (
            Self::std().to_string(),
            opt.0.as_ref().map(|s| s.to_string()),
        )
    }
}

pub(crate) trait OptMeasKey
where
    Self: Optional,
    Self: fmt::Display,
    Self: IndexedKey,
    Self: FromStr,
    <Self as FromStr>::Err: fmt::Display,
{
    fn get_meas_opt(kws: &StdKeywords, n: MeasIdx) -> OptResult<Self> {
        Self::get_opt(kws, &Self::std(n))
    }

    fn remove_meas_opt(kws: &mut StdKeywords, n: MeasIdx) -> OptResult<Self> {
        Self::remove_opt(kws, &Self::std(n))
    }

    fn triple(opt: &OptionalKw<Self>, n: MeasIdx) -> (String, String, Option<String>) {
        (
            Self::blank(),
            Self::std(n).to_string(),
            opt.0.as_ref().map(|s| s.to_string()),
        )
    }

    fn pair(opt: &OptionalKw<Self>, n: MeasIdx) -> (String, Option<String>) {
        let (_, k, v) = Self::triple(opt, n);
        (k, v)
    }
}

pub(crate) trait Linked
where
    Self: Key,
    Self: Sized,
{
    fn check_link(&self, names: &HashSet<&Shortname>) -> Result<(), String> {
        let k = Self::std();
        let bad_names: Vec<_> = self.names().difference(names).copied().collect();
        let bad_names_str = bad_names.iter().join(", ");
        if bad_names.is_empty() {
            Ok(())
        } else if bad_names.len() == 1 {
            Err(format!(
                "{k} references non-existent $PnN name: {bad_names_str}"
            ))
        } else {
            Err(format!(
                "{k} references non-existent $PnN names: {bad_names_str}"
            ))
        }
    }

    fn reassign(&mut self, mapping: &NameMapping);

    fn names(&self) -> HashSet<&Shortname>;
}

macro_rules! newtype_string {
    ($t:ident) => {
        #[derive(Clone, Serialize)]
        pub struct $t(pub String);

        newtype_disp!($t);
        newtype_fromstr!($t, Infallible);
        newtype_from!($t, String);
        newtype_from_outer!($t, String);
    };
}

macro_rules! newtype_int {
    ($t:ident, $type:ident) => {
        #[derive(Clone, Copy, Serialize)]
        pub struct $t(pub $type);

        newtype_disp!($t);
        newtype_fromstr!($t, ParseIntError);
        newtype_from!($t, $type);
        newtype_from_outer!($t, $type);
    };
}

macro_rules! kw_meta {
    ($t:ident, $k:expr) => {
        impl Key for $t {
            const C: &'static str = $k;
        }
    };
}

macro_rules! kw_meas {
    ($t:ident, $sfx:expr) => {
        impl IndexedKey for $t {
            const PREFIX: &'static str = "P";
            const SUFFIX: &'static str = $sfx;
        }
    };
}

macro_rules! kw_meta_string {
    ($t:ident, $kw:expr) => {
        newtype_string!($t);

        impl Key for $t {
            const C: &'static str = $kw;
        }
    };
}

macro_rules! kw_meas_int {
    ($t:ident, $type:ident, $sfx:expr) => {
        newtype_int!($t, $type);
        kw_meas!($t, $sfx);
    };
}

macro_rules! kw_meta_int {
    ($t:ident, $type:ident, $kw:expr) => {
        newtype_int!($t, $type);

        impl Key for $t {
            const C: &'static str = $kw;
        }
    };
}

macro_rules! kw_meas_string {
    ($t:ident, $sfx:expr) => {
        newtype_string!($t);
        kw_meas!($t, $sfx);
    };
}

macro_rules! req_meta {
    ($t:ident) => {
        impl Required for $t {}
        impl ReqMetaKey for $t {}
    };
}

macro_rules! opt_meta {
    ($t:ident) => {
        impl Optional for $t {}
        impl OptMetaKey for $t {}
    };
}

macro_rules! req_meas {
    ($t:ident) => {
        impl Required for $t {}
        impl ReqMeasKey for $t {}
    };
}

macro_rules! opt_meas {
    ($t:ident) => {
        impl Optional for $t {}
        impl OptMeasKey for $t {}
    };
}

macro_rules! kw_req_meta {
    ($t:ident, $sfx:expr) => {
        kw_meta!($t, $sfx);
        req_meta!($t);
    };
}

macro_rules! kw_opt_meta {
    ($t:ident, $sfx:expr) => {
        kw_meta!($t, $sfx);
        opt_meta!($t);
    };
}

macro_rules! kw_req_meas {
    ($t:ident, $sfx:expr) => {
        kw_meas!($t, $sfx);
        req_meas!($t);
    };
}

macro_rules! kw_opt_meas {
    ($t:ident, $sfx:expr) => {
        kw_meas!($t, $sfx);
        opt_meas!($t);
    };
}

macro_rules! kw_opt_meta_string {
    ($t:ident, $sfx:expr) => {
        kw_meta_string!($t, $sfx);
        opt_meta!($t);
    };
}

macro_rules! kw_opt_meas_string {
    ($t:ident, $sfx:expr) => {
        kw_meas_string!($t, $sfx);
        opt_meas!($t);
    };
}

macro_rules! kw_req_meta_int {
    ($t:ident, $type:ident, $sfx:expr) => {
        kw_meta_int!($t, $type, $sfx);
        req_meta!($t);
    };
}

macro_rules! kw_opt_meta_int {
    ($t:ident, $type:ident, $sfx:expr) => {
        kw_meta_int!($t, $type, $sfx);
        opt_meta!($t);
    };
}

macro_rules! kw_opt_meas_int {
    ($t:ident, $type:ident, $sfx:expr) => {
        kw_meas_int!($t, $type, $sfx);
        opt_meas!($t);
    };
}

macro_rules! kw_time {
    ($outer:ident, $wrap:ident, $inner:ident, $err:ident, $key:expr) => {
        type $outer = $wrap<$inner>;

        impl From<$inner> for $outer {
            fn from(value: $inner) -> Self {
                $wrap(value)
            }
        }

        impl FromStr for $outer {
            type Err = $err;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                s.parse().map($wrap)
            }
        }

        newtype_from_outer!($outer, $inner);
        newtype_disp!($outer);
        kw_opt_meta!($outer, $key);

        impl From<NaiveTime> for $outer {
            fn from(value: NaiveTime) -> Self {
                $wrap($inner(value))
            }
        }
    };
}

// all versions
kw_req_meta!(AlphaNumType, "DATATYPE");
kw_opt_meta_int!(Abrt, u32, "ABRT");
kw_opt_meta_string!(Cytsn, "CYTSN");
kw_opt_meta_string!(Com, "COM");
kw_opt_meta_string!(Cells, "CELLS");
kw_opt_meta!(FCSDate, "DATE");
kw_opt_meta_string!(Exp, "EXP");
kw_opt_meta_string!(Fil, "FIL");
kw_opt_meta_string!(Inst, "INST");
kw_opt_meta_int!(Lost, u32, "LOST");
kw_opt_meta_string!(Op, "OP");
kw_req_meta_int!(Par, usize, "PAR");
kw_opt_meta_string!(Proj, "PROJ");
kw_opt_meta_string!(Smno, "SMNO");
kw_opt_meta_string!(Src, "SRC");
kw_opt_meta_string!(Sys, "SYS");
kw_opt_meta!(Trigger, "TR");

// time for 2.0
kw_time!(Btim2_0, Btim, FCSTime, FCSTimeError, "BTIM");
kw_time!(Etim2_0, Etim, FCSTime, FCSTimeError, "ETIM");

// time for 3.0
kw_time!(Btim3_0, Btim, FCSTime60, FCSTime60Error, "BTIM");
kw_time!(Etim3_0, Etim, FCSTime60, FCSTime60Error, "ETIM");

// time for 3.1-3.2
kw_time!(Btim3_1, Btim, FCSTime100, FCSTime100Error, "BTIM");
kw_time!(Etim3_1, Etim, FCSTime100, FCSTime100Error, "ETIM");

// 3.0 only
kw_opt_meta!(Compensation, "COMP");
kw_opt_meta!(Unicode, "UNICODE");

// for 3.0+
kw_req_meta!(Timestep, "TIMESTEP");

// for 3.1+
kw_opt_meta_string!(LastModifier, "LAST_MODIFIER");
kw_opt_meta!(Originality, "ORIGINALITY");
kw_opt_meta!(ModifiedDateTime, "LAST_MODIFIED");

kw_opt_meta_string!(Plateid, "PLATEID");
kw_opt_meta_string!(Platename, "PLATENAME");
kw_opt_meta_string!(Wellid, "WELLID");

kw_opt_meta!(Spillover, "SPILLOVER");

kw_opt_meta!(Vol, "VOL");

// for 3.2+
kw_opt_meta_string!(Carrierid, "CARRIERID");
kw_opt_meta_string!(Carriertype, "CARRIERTYPE");
kw_opt_meta_string!(Locationid, "LOCATIONID");

kw_opt_meta!(BeginDateTime, "BEGINDATETIME");
kw_opt_meta!(EndDateTime, "ENDDATETIME");

kw_opt_meta!(UnstainedCenters, "UNSTAINEDCENTERS");
kw_opt_meta_string!(UnstainedInfo, "UNSTAINEDINFO");

kw_opt_meta_string!(Flowrate, "FLOWRATE");

// version-specific
kw_opt_meta_int!(Tot, usize, "TOT"); // optional in 2.0
req_meta!(Tot); // required in 3.0+

kw_req_meta!(Mode, "MODE"); // for 2.0-3.1
kw_opt_meta!(Mode3_2, "MODE"); // for 3.2+

kw_opt_meta_string!(Cyt, "CYT"); // optional for 2.0-3.1
req_meta!(Cyt); // required for 3.2+

kw_req_meta!(Endian, "BYTEORD"); // 2.0 to 3.0
kw_req_meta!(ByteOrd, "BYTEORD"); // 3.1+

// all versions
kw_req_meas!(Width, "B");
kw_opt_meas_string!(Filter, "F");
kw_opt_meas_int!(Power, u32, "O");
kw_opt_meas_string!(PercentEmitted, "P");
kw_req_meas!(Range, "R");
kw_opt_meas_string!(Longname, "S");
kw_opt_meas_string!(DetectorType, "T");
kw_opt_meas!(DetectorVoltage, "V");

// 3.0+
kw_opt_meas!(Gain, "G");

// 3.1+
kw_opt_meas!(Display, "D");

// 3.2+
kw_opt_meas!(Feature, "FEATURE");
kw_opt_meas!(OpticalType, "TYPE");
kw_opt_meas!(TemporalType, "TYPE");
kw_opt_meas!(NumType, "DATATYPE");
kw_opt_meas_string!(Analyte, "ANALYTE");
kw_opt_meas_string!(Tag, "TAG");
kw_opt_meas_string!(DetectorName, "DET");

// version specific
kw_opt_meas!(Shortname, "N"); // optional for 2.0/3.0
req_meas!(Shortname); // required for 3.1+

kw_opt_meas!(Scale, "E"); // optional for 2.0
req_meas!(Scale); // required for 3.0+

kw_opt_meas_int!(Wavelength, u32, "L"); // scaler in 2.0/3.0
kw_opt_meas!(Wavelengths, "L"); // vector in 3.1+

kw_opt_meas!(Calibration3_1, "CALIBRATION"); // 3.1 doesn't have offset
kw_opt_meas!(Calibration3_2, "CALIBRATION"); // 3.2+ includes offset

pub struct Beginanalysis;
pub struct Endanalysis;
pub struct Begindata;
pub struct Enddata;
pub struct Beginstext;
pub struct Endstext;
pub struct Nextdata;

kw_meta!(Beginanalysis, "BEGINANALYSIS");
kw_meta!(Endanalysis, "ENDANALYSIS");
kw_meta!(Begindata, "BEGINDATA");
kw_meta!(Enddata, "ENDDATA");
kw_meta!(Beginstext, "BEGINSTEXT");
kw_meta!(Endstext, "ENDSTEXT");
kw_meta!(Nextdata, "NEXTDATA");
