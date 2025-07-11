use crate::config::RawTextReadConfig;
use crate::error::*;
use crate::text::index::IndexFromOne;

use derive_more::{AsRef, Display, From};
use itertools::Itertools;
use regex::Regex;
use serde::Serialize;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::Hash;
use std::str;
use std::str::FromStr;
use unicase::Ascii;

/// A standard key.
///
/// These may only contain ASCII and must start with "$". The "$" is not
/// actually stored but will be appended when converting to a ['String'].
#[derive(Clone, Debug, PartialEq, Eq, Hash, AsRef)]
#[as_ref(KeyString, str)]
pub struct StdKey(KeyString);

/// A non-standard key.
///
/// This cannot start with '$' and may only contain ASCII characters.
#[derive(Clone, Debug, AsRef, Display, PartialEq, Eq, Hash)]
#[as_ref(KeyString, str)]
pub struct NonStdKey(KeyString);

pub type NonStdPairs = Vec<(NonStdKey, String)>;
pub type NonStdKeywords = HashMap<NonStdKey, String>;

/// The internal string for a key (standard or nonstandard).
///
/// Must be non-empty and contain only ASCII characters. Comparisons will be
/// case-insensitive.
#[derive(Clone, Debug, AsRef, Display, PartialEq, Eq, Hash)]
#[as_ref(str)]
pub struct KeyString(Ascii<String>);

/// A String that matches part of a non-standard measurement key.
///
/// This will have exactly one '%n' and not start with a '$'. The
/// '%n' will be replaced by the measurement index which will be used
/// to match keywords.
#[derive(Clone, AsRef, Display)]
#[as_ref(str)]
pub struct NonStdMeasPattern(String);

/// A list of patterns that match standard or non-standard keys.
#[derive(Clone, Default)]
pub struct KeyPatterns(Vec<KeyStringOrPattern>);

/// Either a literal string or regexp which matches a standard/non-standard key.
///
/// This exists for performance and ergononic reasons; if the goal is simply to
/// match lots of strings literally, it is faster and easier to use a hash
/// table, otherwise we need to search linearly through an array of patterns.
#[derive(Clone)]
pub enum KeyStringOrPattern {
    Literal(KeyString),
    Pattern(CaseInsRegex),
}

/// A collection dump for parsed keywords of varying quality
#[derive(Default)]
pub struct ParsedKeywords {
    /// Standard keywords (with '$')
    pub std: StdKeywords,

    /// Non-standard keywords (without '$')
    pub nonstd: NonStdKeywords,

    /// Keywords that don't have ASCII keys (not allowed)
    pub non_ascii: NonAsciiPairs,

    /// Keywords that are not valid UTF-8 strings
    pub byte_pairs: BytesPairs,
}

pub type StdKeywords = HashMap<StdKey, String>;
pub type NonAsciiPairs = Vec<(String, String)>;
pub type BytesPairs = Vec<(Vec<u8>, Vec<u8>)>;

/// ['ParsedKeywords'] without the bad stuff
#[derive(Clone, Default, Serialize)]
pub struct ValidKeywords {
    pub std: StdKeywords,
    pub nonstd: NonStdKeywords,
}

/// A string that should be used as the header in the measurement table.
#[derive(Display)]
pub struct MeasHeader(pub String);

/// A regular expression which matches a non-standard measurement key.
///
/// This must be derived from ['NonStdMeasPattern'].
#[derive(AsRef)]
#[as_ref(Regex)]
pub(crate) struct NonStdMeasRegex(CaseInsRegex);

/// A regex which ignores case when matching
#[derive(Clone, AsRef)]
pub struct CaseInsRegex(Regex);

/// A "compiled" object to match keys efficiently.
struct KeyMatcher<'a> {
    literal: HashSet<&'a KeyString>,
    pattern: Vec<&'a CaseInsRegex>,
}

/// A standard key
///
/// The constant traits is assumed to only contain ASCII characters.
// TODO const_trait_impl will be able to clean this up once stable
pub(crate) trait Key {
    const C: &'static str;

    fn std() -> StdKey {
        StdKey::new(Self::C.to_string())
    }

    fn len() -> u64 {
        (Self::C.len() + 1) as u64
    }
}

/// A standard key with on index
///
/// The constant traits are assumed to only contain ASCII characters.
pub(crate) trait IndexedKey {
    const PREFIX: &'static str;
    const SUFFIX: &'static str;

    fn std(i: IndexFromOne) -> StdKey {
        // reserve enough space for prefix, suffix, and a number with 3 digits
        let n = Self::PREFIX.len() + 3 + Self::SUFFIX.len();
        let mut s = String::with_capacity(n);
        s.push_str(Self::PREFIX);
        s.push_str(i.to_string().as_str());
        s.push_str(Self::SUFFIX);
        StdKey::new(s)
    }

    fn std_blank() -> MeasHeader {
        // reserve enough space for '$', prefix, suffix, and 'n'
        let n = Self::PREFIX.len() + 2 + Self::SUFFIX.len();
        let mut s = String::new();
        s.reserve_exact(n);
        s.push('$');
        s.push_str(Self::PREFIX);
        s.push('n');
        s.push_str(Self::SUFFIX);
        MeasHeader(s)
    }

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

/// A standard key with two indices
///
/// The constant traits are assumed to only contain ASCII characters.
pub(crate) trait BiIndexedKey {
    const PREFIX: &'static str;
    const MIDDLE: &'static str;
    const SUFFIX: &'static str;

    fn std(i: IndexFromOne, j: IndexFromOne) -> StdKey {
        // reserve enough space for prefix, middle, suffix, and two numbers with
        // 2 digits
        let n = Self::PREFIX.len() + Self::MIDDLE.len() + Self::SUFFIX.len() + 4;
        let mut s = String::with_capacity(n);
        s.push_str(Self::PREFIX);
        s.push_str(i.to_string().as_str());
        s.push_str(Self::MIDDLE);
        s.push_str(j.to_string().as_str());
        s.push_str(Self::SUFFIX);
        StdKey::new(s)
    }

    // fn std_blank() -> String {
    //     // reserve enough space for '$', prefix, middle, suffix, and 'n'/'m'
    //     let n = Self::PREFIX.len() + 2 + Self::SUFFIX.len();
    //     let mut s = String::new();
    //     s.reserve_exact(n);
    //     s.push('$');
    //     s.push_str(Self::PREFIX);
    //     s.push('m');
    //     s.push_str(Self::MIDDLE);
    //     s.push('n');
    //     s.push_str(Self::SUFFIX);
    //     s
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

impl KeyString {
    fn new(s: String) -> Self {
        Self(Ascii::new(s))
    }

    fn from_bytes(xs: &[u8]) -> Self {
        Self::new(unsafe { String::from_utf8_unchecked(xs.to_vec()) })
    }
}

impl StdKey {
    fn new(s: String) -> Self {
        Self(KeyString::new(s))
    }
}

impl NonStdKey {
    fn new(s: String) -> Self {
        Self(KeyString::new(s))
    }
}

impl Serialize for StdKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.as_ref().serialize(serializer)
    }
}

impl Serialize for NonStdKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.as_ref().serialize(serializer)
    }
}

impl fmt::Display for StdKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "${}", self.0)
    }
}

impl FromStr for KeyString {
    type Err = AsciiStringError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if is_printable_ascii(s.as_ref()) {
            Ok(Self(Ascii::new(s.to_string())))
        } else {
            Err(AsciiStringError(s.to_string()))
        }
    }
}

impl FromStr for StdKey {
    type Err = KeyStringError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<KeyString>()
            .map_err(KeyStringError::Ascii)
            .and_then(|x| {
                if has_std_prefix(x.as_ref().as_bytes()) {
                    Ok(Self::new(x.to_string()))
                } else {
                    Err(KeyStringError::Prefix(true, x.to_string()))
                }
            })
    }
}

impl FromStr for NonStdKey {
    type Err = KeyStringError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<KeyString>()
            .map_err(KeyStringError::Ascii)
            .and_then(|x| {
                if has_no_std_prefix(x.as_ref().as_bytes()) {
                    Ok(Self::new(x.to_string()))
                } else {
                    Err(KeyStringError::Prefix(false, x.to_string()))
                }
            })
    }
}

impl FromStr for NonStdMeasPattern {
    type Err = NonStdMeasPatternError;

    fn from_str(s: &str) -> Result<Self, NonStdMeasPatternError> {
        if has_no_std_prefix(s.as_bytes()) || s.match_indices("%n").count() == 1 {
            Ok(NonStdMeasPattern(s.to_string()))
        } else {
            Err(NonStdMeasPatternError(s.to_string()))
        }
    }
}

impl NonStdMeasPattern {
    pub(crate) fn apply_index(
        &self,
        n: IndexFromOne,
    ) -> Result<NonStdMeasRegex, NonStdMeasRegexError> {
        self.0
            .replace("%n", n.to_string().as_str())
            .as_str()
            .parse::<CaseInsRegex>()
            .map_err(|error| NonStdMeasRegexError { error, index: n })
            .map(NonStdMeasRegex)
    }
}

impl FromStr for CaseInsRegex {
    type Err = regex::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        regex::RegexBuilder::new(s)
            .case_insensitive(true)
            .build()
            .map(Self)
    }
}

impl KeyPatterns {
    pub fn extend(&mut self, other: Self) {
        self.0.extend(other.0)
    }

    pub fn try_from_literals(ss: Vec<String>) -> Result<Self, AsciiStringError> {
        ss.into_iter()
            .unique()
            .map(|s| s.parse::<KeyString>().map(KeyStringOrPattern::Literal))
            .collect::<Result<Vec<_>, _>>()
            .map(KeyPatterns)
    }

    pub fn try_from_patterns(ss: Vec<String>) -> Result<Self, regex::Error> {
        ss.into_iter()
            .unique()
            .map(|s| s.parse::<CaseInsRegex>().map(KeyStringOrPattern::Pattern))
            .collect::<Result<Vec<_>, _>>()
            .map(KeyPatterns)
    }

    fn as_matcher(&self) -> KeyMatcher<'_> {
        let (literal, pattern): (HashSet<_>, Vec<_>) = self
            .0
            .iter()
            .map(|x| match x {
                KeyStringOrPattern::Literal(l) => Ok(l),
                KeyStringOrPattern::Pattern(p) => Err(p),
            })
            .partition_result();
        KeyMatcher { literal, pattern }
    }
}

impl KeyMatcher<'_> {
    fn is_match(&self, other: &KeyString) -> bool {
        self.literal.contains(other)
            || self
                .pattern
                .iter()
                .any(|p| p.as_ref().is_match(other.as_ref()))
    }
}

impl ParsedKeywords {
    pub(crate) fn insert(
        &mut self,
        k: &[u8],
        v: &[u8],
        conf: &RawTextReadConfig,
    ) -> Result<(), Leveled<KeywordInsertError>> {
        // ASSUME key and value are never blank since we checked both prior to
        // calling this. The FCS standards do not allow either to be blank.
        let n = k.len();

        let to_std = conf.promote_to_standard.as_matcher();
        let to_nonstd = conf.demote_from_standard.as_matcher();
        // TODO this also should skip keys before throwing a blank error
        let ignore = conf.ignore_standard_keys.as_matcher();

        match std::str::from_utf8(v) {
            Ok(vv) => {
                // Trim whitespace from value if desired. Warn (or halt) if this
                // results in a blank.
                let value = if conf.trim_value_whitespace {
                    let trimmed = vv.trim();
                    if trimmed.is_empty() {
                        let w = BlankValueError(k.to_vec());
                        return Err(Leveled::new(w.into(), !conf.allow_empty));
                    } else {
                        trimmed.to_string()
                    }
                } else {
                    vv.to_string()
                };
                if n > 1 && k[0] == STD_PREFIX && is_printable_ascii(&k[1..]) {
                    // Standard key: starts with '$', check that remaining chars
                    // are ASCII
                    let kk = KeyString::from_bytes(&k[1..]);
                    if ignore.is_match(&kk) {
                        Ok(())
                    } else if to_nonstd.is_match(&kk) {
                        insert_nonunique(&mut self.nonstd, NonStdKey(kk), value, conf)
                    } else {
                        let rk = conf.rename_standard_keys.get(&kk).cloned().unwrap_or(kk);
                        insert_nonunique(&mut self.std, StdKey(rk), value, conf)
                    }
                } else if n > 0 && is_printable_ascii(k) {
                    // Non-standard key: does not start with '$' but is still
                    // ASCII
                    let kk = KeyString::from_bytes(k);
                    if to_std.is_match(&kk) {
                        insert_nonunique(&mut self.std, StdKey(kk), value, conf)
                    } else {
                        insert_nonunique(&mut self.nonstd, NonStdKey(kk), value, conf)
                    }
                } else if let Ok(kk) = String::from_utf8(k.to_vec()) {
                    // Non-ascii key: these are technically not allowed but save
                    // them anyways in case the user cares. If key isn't UTF-8
                    // then give up.
                    self.non_ascii.push((kk, value));
                    Ok(())
                } else {
                    self.byte_pairs.push((k.to_vec(), value.into()));
                    Ok(())
                }
            }
            _ => {
                self.byte_pairs.push((k.to_vec(), v.to_vec()));
                Ok(())
            }
        }
    }

    pub(crate) fn append_std(
        &mut self,
        new: &HashMap<KeyString, String>,
        allow_nonunique: bool,
    ) -> MultiResult<(), Leveled<StdPresent>> {
        new.iter()
            .map(|(k, v)| match self.std.entry(StdKey(k.clone())) {
                Entry::Occupied(e) => {
                    let key = e.key().clone();
                    let value = v.clone();
                    let w = KeyPresent { key, value };
                    Err(Leveled::new(w, !allow_nonunique))
                }
                Entry::Vacant(e) => {
                    e.insert(v.clone());
                    Ok(())
                }
            })
            .gather()
            .void()
    }
}

#[derive(Debug, Display, From)]
pub enum KeywordInsertError {
    StdPresent(StdPresent),
    NonStdPresent(NonStdPresent),
    Blank(BlankValueError),
}

#[derive(Debug)]
pub struct BlankValueError(pub Vec<u8>);

#[derive(Debug)]
pub struct KeyPresent<T> {
    pub key: T,
    pub value: String,
}

pub type StdPresent = KeyPresent<StdKey>;
pub type NonStdPresent = KeyPresent<NonStdKey>;

pub struct AsciiStringError(String);

#[derive(From)]
pub enum KeyStringError {
    Ascii(AsciiStringError),
    Prefix(bool, String),
}

pub struct NonStdMeasKeyError(String);

#[derive(Debug)]
pub struct NonStdMeasPatternError(String);

pub struct NonStdMeasRegexError {
    error: regex::Error,
    index: IndexFromOne,
}

impl<T: fmt::Display> fmt::Display for KeyPresent<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "key '{}' already present, has value '{}'",
            self.key, self.value
        )
    }
}

impl fmt::Display for AsciiStringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "string should only have ASCII characters and not be empty, found '{}'",
            self.0
        )
    }
}

impl fmt::Display for KeyStringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            Self::Ascii(x) => x.fmt(f),
            Self::Prefix(is_std, s) => {
                let k = if *is_std {
                    "Standard key must start with '$'"
                } else {
                    "Non-standard key must not start with '$'"
                };
                write!(f, "{k}, found '{s}'")
            }
        }
    }
}

impl fmt::Display for NonStdMeasKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "Non standard measurement pattern must not \
             start with '$', have only ASCII characters, \
             and should have one '%n', found '{}'",
            self.0
        )
    }
}

impl fmt::Display for NonStdMeasPatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "Non standard measurement pattern must not \
             start with '$' and should have one '%n', found '{}'",
            self.0
        )
    }
}

impl fmt::Display for NonStdMeasRegexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "Regexp error for measurement {}: {}",
            self.index, self.error
        )
    }
}

impl fmt::Display for BlankValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        let s = str::from_utf8(&self.0[..]).map_or_else(
            |_| format!("key's bytes were {}", self.0.iter().join(",")),
            |s| format!("key was {s}"),
        );
        write!(f, "skipping key with blank value, {s}")
    }
}

fn is_printable_ascii(xs: &[u8]) -> bool {
    xs.iter().all(|x| 32 <= *x && *x <= 126)
}

fn has_std_prefix(xs: &[u8]) -> bool {
    xs.first().is_some_and(|x| *x == STD_PREFIX)
}

fn has_no_std_prefix(xs: &[u8]) -> bool {
    xs.first().is_some_and(|x| *x != STD_PREFIX)
}

fn insert_nonunique<K>(
    kws: &mut HashMap<K, String>,
    k: K,
    value: String,
    conf: &RawTextReadConfig,
) -> Result<(), Leveled<KeywordInsertError>>
where
    K: std::hash::Hash + Eq + Clone + AsRef<KeyString>,
    KeywordInsertError: From<KeyPresent<K>>,
{
    match kws.entry(k) {
        Entry::Occupied(e) => {
            let key = e.key().clone();
            let w = KeyPresent { key, value };
            Err(Leveled::new(w.into(), !conf.allow_nonunique))
        }
        Entry::Vacant(e) => {
            let v = conf
                .replace_standard_key_values
                .get(e.key().as_ref())
                .map(|v| v.to_string())
                .unwrap_or(value);
            e.insert(v);
            Ok(())
        }
    }
}

const STD_PREFIX: u8 = 36; // '$'
