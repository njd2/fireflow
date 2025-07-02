use crate::text::index::MeasIndex;

use derive_more::{AsRef, Display};
use serde::Serialize;
use std::fmt;
use std::str::FromStr;

/// The value for the $PnN key (all versions).
///
/// This cannot contain commas.
#[derive(Clone, Serialize, Eq, PartialEq, Hash, Debug, AsRef, Display)]
#[as_ref(str)]
pub struct Shortname(String);

/// A prefix that can be made into a shortname by appending an index
///
/// This cannot contain commas.
#[derive(Clone, Serialize, Eq, PartialEq, Hash, AsRef, Display)]
#[as_ref(str)]
pub struct ShortnamePrefix(Shortname);

impl Shortname {
    pub fn new_unchecked<T: AsRef<str>>(s: T) -> Self {
        Shortname(s.as_ref().to_owned())
    }
}

impl FromStr for Shortname {
    type Err = ShortnameError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.contains(',') {
            Err(ShortnameError(s.to_string()))
        } else {
            Ok(Shortname(s.to_string()))
        }
    }
}

impl FromStr for ShortnamePrefix {
    type Err = ShortnameError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<Shortname>().map(ShortnamePrefix)
    }
}

impl ShortnamePrefix {
    pub fn as_indexed(&self, i: MeasIndex) -> Shortname {
        Shortname(format!("{}{i}", self))
    }
}

impl Default for ShortnamePrefix {
    fn default() -> ShortnamePrefix {
        ShortnamePrefix(Shortname("P".into()))
    }
}

pub struct ShortnameError(String);

impl fmt::Display for ShortnameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "commas are not allowed in name '{}'", self.0)
    }
}
