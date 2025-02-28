// TODO gating parameters not added (yet)

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use regex::Regex;
use std::collections::HashMap;

struct Bounds {
    begin: u32,
    end: u32,
}

enum Version {
    FCS2_0,
    FCS3_0,
    FCS3_1,
    FCS3_2,
}

struct Header {
    version: Version,
    text: Bounds,
    data: Bounds,
    analysis: Bounds,
}

enum AlphaNumTypes {
    Ascii,
    Integer,
    Float,
    Double,
}

enum NumTypes {
    Integer,
    Float,
    Double,
}

enum Endianness {
    BigEndian,
    LittleEndian,
}

enum ByteOrd {
    BitLittle(Endianness),
    Mixed(u8, u8, u8, u8),
}

struct Trigger {
    parameter: String,
    threshhold: u32,
}

struct TextBounds<A, D, T> {
    analysis: A,
    data: D,
    stext: T,
}

type TextBounds3_0 = TextBounds<Bounds, Bounds, Bounds>;
type TextBounds3_2 = TextBounds<Option<Bounds>, Bounds, Option<Bounds>>;

struct Timestamps2_0 {
    btim: Option<NaiveTime>,
    etim: Option<NaiveTime>,
    date: Option<NaiveDate>,
}

struct Timestamps3_2 {
    // TODO local or urc? FCS allows both
    start: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
}

// TODO this is super messy, see 3.2 spec for restrictions on this we may with
// to use further
struct LogScale {
    decades: f32,
    offset: f32,
}

enum Scale {
    Log(LogScale),
    Linear,
}

use Scale::*;

struct LinDisplay {
    lower: f32,
    upper: f32,
}

struct LogDisplay {
    offset: f32,
    decades: f32,
}

enum Display {
    Lin(LinDisplay),
    Log(LogDisplay),
}

struct Calibration {
    value: f32,
    unit: String,
}

struct InnerParameter3_0 {
    gain: Option<f32>,
}

struct InnerParameter3_1 {
    calibration: Option<Calibration>,
    display: Display,
    older: InnerParameter3_0,
}

enum MeasurementType {
    ForwardScatter,
    SideScatter,
    RawFluorescence,
    UnmixedFluorescence,
    Mass,
    Time,
    ElectronicVolume,
    Classification,
    Index,
}

enum Feature {
    Area,
    Width,
    Height,
}

struct InnerParameter3_2 {
    analyte: Option<String>,
    feature: Option<Feature>,
    measurement_type: Option<MeasurementType>,
    tag: Option<String>,
    detector_name: Option<String>,
    datatype: Option<AlphaNumTypes>,
    older: InnerParameter3_1,
}

struct Parameter<E, L, N, X> {
    bits: u32,                     // PnB
    range: u32,                    // PnR
    scale: E,                      // PnE
    shortname: N,                  // PnN
    longname: Option<String>,      // PnS
    filter: Option<String>,        // PnF
    wavelength: L,                 // PnL
    power: Option<u32>,            // PnO
    detector_type: Option<String>, // PnD
    percent_emitted: Option<u32>,  // PnP (TODO deprecated in 3.2, factor out)
    detector_voltage: Option<f32>, // PnV
    specific: X,
}

type Wavelength2_0 = Option<u32>;
type Wavelength3_1 = Vec<u32>; // TODO this should be non-empty

trait ParameterFromKeywords: Sized {
    // fn kw_to_param(kw: &str) -> Self;

    fn from_kws(kws: &mut Keywords) -> (Vec<Self>, &mut Keywords, KeywordErrors) {
        unimplemented!()
    }
}

type Parameter2_0 = Parameter<Option<Scale>, Wavelength2_0, Option<String>, ()>;
type Parameter3_0 = Parameter<Scale, Wavelength2_0, Option<String>, InnerParameter3_0>;
type Parameter3_1 = Parameter<Scale, Wavelength3_1, String, InnerParameter3_1>;
type Parameter3_2 = Parameter<Scale, Wavelength3_1, String, InnerParameter3_2>;

enum LookupResult<V> {
    Passing(V),
    Error(String, String, String),
    NotFound,
}

impl<V> LookupResult<V> {
    fn to_option(self) -> LookupResult<Option<V>> {
        match self {
            Passing(x) => Passing(Some(x)),
            NotFound => Passing(None),
            Error(k, v, e) => Error(k, v, e),
        }
    }
    fn to_error(self) -> Option<(String, (String, String))> {
        match self {
            Passing(_) => None,
            NotFound => None,
            Error(k, v, e) => Some((k, (v, e))),
        }
    }
}

use LookupResult::*;

fn lookup_int<'a>(kws: &mut Keywords, param: &'static str, n: u32) -> LookupResult<u32> {
    let k = format!("P{}{}", n, param);
    kws.remove(&k).map_or(NotFound, |s| match s.parse() {
        Ok(x) => Passing(x),
        _ => Error(k, s, String::from("not a valid integer")),
    })
}

fn lookup_float<'a>(kws: &mut Keywords, param: &'static str, n: u32) -> LookupResult<f32> {
    let k = format!("P{}{}", n, param);
    kws.remove(&k).map_or(NotFound, |s| match s.parse() {
        Ok(x) => Passing(x),
        _ => Error(k, s, String::from("not a valid float")),
    })
}

fn lookup_str(kws: &mut Keywords, param: &'static str, n: u32) -> LookupResult<String> {
    let k = format!("P{}{}", n, param);
    kws.remove(&k).map_or(NotFound, |s| Passing(s))
}

// TODO check that this is in multiples of 8 for relevant specs
fn lookup_bits(kws: &mut Keywords, n: u32) -> LookupResult<u32> {
    lookup_int(kws, "B", n)
}

fn lookup_range(kws: &mut Keywords, n: u32) -> LookupResult<u32> {
    lookup_int(kws, "R", n)
}

// TODO need a vector version of this for 3.1+
fn lookup_wavelength(kws: &mut Keywords, n: u32) -> LookupResult<u32> {
    lookup_int(kws, "L", n)
}

fn lookup_power(kws: &mut Keywords, n: u32) -> LookupResult<u32> {
    lookup_int(kws, "O", n)
}

fn lookup_detector_type(kws: &mut Keywords, n: u32) -> LookupResult<String> {
    lookup_str(kws, "T", n)
}

fn lookup_shortname(kws: &mut Keywords, n: u32) -> LookupResult<String> {
    lookup_str(kws, "N", n)
}

fn lookup_longname(kws: &mut Keywords, n: u32) -> LookupResult<String> {
    lookup_str(kws, "S", n)
}

fn lookup_filter(kws: &mut Keywords, n: u32) -> LookupResult<String> {
    lookup_str(kws, "F", n)
}

fn lookup_percent_emitted(kws: &mut Keywords, n: u32) -> LookupResult<u32> {
    lookup_int(kws, "P", n)
}

fn lookup_detector_voltage(kws: &mut Keywords, n: u32) -> LookupResult<f32> {
    lookup_float(kws, "V", n)
}

fn lookup_scale(kws: &mut Keywords, n: u32) -> LookupResult<Scale> {
    let k = format!("P{}E", n);
    match kws.remove(&k) {
        Some(s) => {
            let v: Vec<&str> = s.split(",").collect();
            match v[..] {
                [ds, os] => match (ds.parse(), os.parse()) {
                    (Ok(0.0), Ok(0.0)) => Passing(Linear),
                    (Ok(decades), Ok(offset)) => Passing(Log(LogScale { decades, offset })),
                    _ => Error(k, s, String::from("invalid floats")),
                },
                _ => Error(k, s, String::from("too many fields")),
            }
        }
        _ => NotFound,
    }
}

impl ParameterFromKeywords for Parameter2_0 {
    fn from_kws(kws: &mut Keywords) -> (Vec<Parameter2_0>, &mut Keywords, KeywordErrors) {
        let mut ps: Vec<Parameter2_0> = vec![];
        let mut errors: KeywordErrors = HashMap::new();
        let mut n = 1;
        loop {
            // lookup bits since this should be present in all versions, if not
            // present then consider the previous index to be the last parameter
            // index
            match lookup_bits(kws, n) {
                Passing(bits) => {
                    let pnr = lookup_range(kws, n);
                    let pne = lookup_scale(kws, n).to_option();
                    let pnn = lookup_shortname(kws, n).to_option();
                    let pns = lookup_longname(kws, n).to_option();
                    let pnf = lookup_filter(kws, n).to_option();
                    let pnl = lookup_wavelength(kws, n).to_option();
                    let pno = lookup_power(kws, n).to_option();
                    let pnt = lookup_detector_type(kws, n).to_option();
                    let pnp = lookup_percent_emitted(kws, n).to_option();
                    let pnv = lookup_detector_voltage(kws, n).to_option();
                    match (pnr, pne, pnn, pns, pnf, pnl, pno, pnt, pnp, pnv) {
                        (
                            Passing(range),
                            Passing(scale),
                            Passing(shortname),
                            Passing(longname),
                            Passing(filter),
                            Passing(wavelength),
                            Passing(power),
                            Passing(detector_type),
                            Passing(percent_emitted),
                            Passing(detector_voltage),
                        ) => {
                            let p = Parameter {
                                bits,
                                range,
                                scale,
                                shortname,
                                longname,
                                filter,
                                wavelength,
                                power,
                                detector_type,
                                percent_emitted,
                                detector_voltage,
                                specific: (),
                            };
                            ps.push(p);
                        }
                        (r, e, n, s, f, l, o, t, p, v) => {
                            let es = [
                                r.to_error(),
                                e.to_error(),
                                n.to_error(),
                                s.to_error(),
                                f.to_error(),
                                l.to_error(),
                                o.to_error(),
                                t.to_error(),
                                p.to_error(),
                                v.to_error(),
                            ];
                            errors.extend(es.into_iter().flatten());
                        }
                    }
                }
                NotFound => return (ps, kws, errors),
                Error(k, v, e) => {
                    errors.insert(k, (v, e));
                    return (ps, kws, errors);
                }
            }
            n = n + 1;
        }
    }
}

enum Originality {
    Original,
    NonDataModified,
    Appended,
    DataModified,
}

struct ModificationData {
    last_modifier: Option<String>,
    list_modified: Option<DateTime<Utc>>,
    originality: Option<Originality>,
}

struct PlateData {
    plateid: Option<String>,
    platename: Option<String>,
    wellid: Option<String>,
}

struct UnstainedData {
    unstainedcenters: HashMap<String, f32>,
    unstainedinfo: Option<String>,
}

struct CarrierData {
    carrierid: Option<String>,
    carriertype: Option<String>,
    locationid: Option<String>,
}

type Timestep = Option<f32>;

type CytSN = Option<String>;

type Vol = Option<f32>;

struct OptionalCommon<C, I, M, P, S, T, U, V> {
    abrt: Option<u32>,
    com: Option<String>,
    cells: Option<String>,
    exp: Option<String>,
    fil: Option<String>,
    inst: Option<String>,
    lost: Option<u32>,
    op: Option<String>,
    proj: Option<String>,
    smno: Option<String>,
    src: Option<String>,
    sys: Option<String>,
    tr: Option<Trigger>,
    carrier: C,
    timestamps: I,
    modified: M,
    plate: P,
    cytsn: S,
    timestep: T,
    unstained: U,
    vol: V,
}

type OptionalCommon2_0 = OptionalCommon<(), Timestamps2_0, (), (), (), (), (), ()>;

type OptionalCommon3_0 = OptionalCommon<(), Timestamps2_0, (), (), CytSN, Timestep, (), ()>;

type OptionalCommon3_1 =
    OptionalCommon<(), Timestamps2_0, ModificationData, PlateData, CytSN, Timestep, (), Vol>;

type OptionalCommon3_2 = OptionalCommon<
    CarrierData,
    Timestamps3_2,
    ModificationData,
    PlateData,
    CytSN,
    Timestep,
    UnstainedData,
    Vol,
>;

struct Spillover {} // TODO, can probably get away with using a matrix for this

struct RequiredCommon<B, C, D, M, P, T> {
    par: u32,
    tot: T, // weirdly not required in 2.0
    mode: M,
    byteord: B,
    datatype: D,
    nextdata: u32,
    cyt: C,
    spillover: Spillover,
    parameters: Vec<P>,
}

struct Cyt(String);

struct Tot(u32);

enum Mode {
    List,
    Uncorrelated,
    Correlated,
}

type RequiredCommon2_0 =
    RequiredCommon<ByteOrd, Option<Cyt>, AlphaNumTypes, Mode, Parameter2_0, Option<Tot>>;

type RequiredCommon3_0 =
    RequiredCommon<ByteOrd, Option<Cyt>, AlphaNumTypes, Mode, Parameter3_0, Tot>;

type RequiredCommon3_1 =
    RequiredCommon<Endianness, Option<Cyt>, AlphaNumTypes, Mode, Parameter3_1, Tot>;

type RequiredCommon3_2 = RequiredCommon<Endianness, Cyt, NumTypes, (), Parameter3_2, Tot>;

struct StdText<O, P, R, X> {
    required: R,
    optional: O,
    parameters: Vec<P>,
    // random place for deprecated kws that I don't feel like putting in the
    // main required/optional structs
    misc: X,
}

struct MiscText3_0 {
    unicode: Unicode,
}

type StdText2_0 = StdText<RequiredCommon2_0, OptionalCommon2_0, Parameter2_0, ()>;
type StdText3_0 = StdText<RequiredCommon3_0, OptionalCommon3_0, Parameter3_0, MiscText3_0>;
type StdText3_1 = StdText<RequiredCommon3_1, OptionalCommon3_1, Parameter3_1, ()>;
type StdText3_2 = StdText<RequiredCommon3_2, OptionalCommon3_2, Parameter3_2, ()>;

struct StdTextResult<T> {
    text: T,
    errors: Keywords,
    nonstandard: Keywords,
}

// trait HasStandard {
//     fn from_kws(kws: Keywords) -> StdTextResult<StdText2_0> {
//         unimplemented!()
//         // return StdTextResult(text = 0, errors = 1, nonstandard = 2);
//     }
// }

// impl<O, P, R, X> StdText<O, P, R, X> {
//     fn from_kws(kws: Keywords) -> StdTextResult<O, P, R, X> {
//         unimplemented!()
//         // return StdTextResult(text = 0, errors = 1, nonstandard = 2);
//     }
// }

impl StdText2_0 {
    fn from_kws(kws: Keywords) -> StdTextResult<StdText2_0> {
        unimplemented!()
        // return StdTextResult(text = 0, errors = 1, nonstandard = 2);
    }
}

struct TEXT<S> {
    // TODO add the offsets here as well? offsets are needed before parsing
    // everything else
    standard: S,
    standard_errors: HashMap<String, String>,
    nonstandard: HashMap<String, String>,
    deviant: HashMap<String, String>,
}

type TEXT2_0 = TEXT<StdText2_0>;
type TEXT3_0 = TEXT<StdText3_0>;
type TEXT3_1 = TEXT<StdText3_1>;
type TEXT3_2 = TEXT<StdText3_2>;

type Keywords = HashMap<String, String>;
type KeywordErrors = HashMap<String, (String, String)>;

fn split_nonstandard(kws: Keywords) -> (Keywords, Keywords) {
    unimplemented!()
}

impl<T> TEXT<T> {
    fn from_kws(kws: Keywords) -> TEXT<T> {
        let res = T::from_kws(kws);
        let (ns, dv) = split_nonstandard(res.nonstandard);
        return TEXT {
            standard: res.text,
            standard_errors: res.errors,
            nonstandard: ns,
            deviant: dv,
        };
    }
}

// struct Correction {
//     from: u32,
//     to: u32,
//     frac: u32, // percent
// }

// struct Text2_0 {
//     corrections: Vec<Correction>,
// }

struct Unicode {
    page: u32,
    kws: Vec<String>,
}

// TODO this is basically a matrix, probably a crate I can use
struct Comp {}

struct Text3_0 {
    unicode: Option<Unicode>,
    comp: Comp,
    // TODO pull out
    analysis: Bounds,
    data: Bounds,
    text: Bounds,
}

struct Text3_1 {}

struct Text3_2 {}

// struct Text {
//     little_endian: bool,
//     datatype: Datatype,
// }

fn main() {
    println!("Hello, world!");
}
