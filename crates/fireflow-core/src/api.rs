use crate::config::*;
use crate::core::*;
use crate::data::*;
use crate::error::*;
use crate::header::*;
use crate::segment::*;
use crate::text::keywords::*;
use crate::text::parser::*;
use crate::text::timestamps::*;
use crate::validated::dataframe::FCSDataFrame;
use crate::validated::keys::*;

use chrono::NaiveDate;
use derive_more::{Display, From};
use itertools::Itertools;
use serde::Serialize;
use std::fmt;
use std::fs;
use std::io::{BufReader, Read, Seek};
use std::num::ParseIntError;
use std::path;

/// Read HEADER from an FCS file.
pub fn fcs_read_header(
    p: &path::PathBuf,
    conf: &HeaderConfig,
) -> IOTerminalResult<Header, (), HeaderError, HeaderFailure> {
    fs::File::options()
        .read(true)
        .open(p)
        .and_then(|file| ReadState::init(&file, conf).map(|st| (st, file)))
        .into_deferred()
        .def_and_maybe(|(st, file)| {
            let mut reader = BufReader::new(file);
            Header::h_read(&mut reader, &st).mult_to_deferred()
        })
        .def_terminate(HeaderFailure)
}

/// Read HEADER and key/value pairs from TEXT in an FCS file.
pub fn fcs_read_raw_text(
    p: &path::PathBuf,
    conf: &RawTextReadConfig,
) -> IOTerminalResult<RawTEXTOutput, ParseRawTEXTWarning, HeaderOrRawError, RawTEXTFailure> {
    read_fcs_raw_text_inner(p, conf)
        .def_map_value(|(x, _, _)| x)
        .def_terminate_maybe_warn(RawTEXTFailure, conf.warnings_are_errors, |w| {
            ImpureError::Pure(w.into())
        })
}

/// Read HEADER and standardized TEXT from an FCS file.
pub fn fcs_read_std_text(
    p: &path::PathBuf,
    conf: &StdTextReadConfig,
) -> IOTerminalResult<StdTEXTOutput, StdTEXTWarning, StdTEXTError, StdTEXTFailure> {
    read_fcs_raw_text_inner(p, &conf.raw)
        .def_map_value(|(x, _, st)| (x, st))
        .def_io_into()
        .def_and_maybe(|(raw, st)| {
            raw.into_std_text(&st.replace_inner(conf))
                .def_inner_into()
                .def_errors_liftio()
        })
        .def_terminate_maybe_warn(StdTEXTFailure, conf.raw.warnings_are_errors, |w| {
            ImpureError::Pure(StdTEXTError::from(w))
        })
}

/// Read dataset from FCS file using standardized TEXT.
pub fn fcs_read_raw_dataset(
    p: &path::PathBuf,
    conf: &DataReadConfig,
) -> IOTerminalResult<RawDatasetOutput, RawDatasetWarning, RawDatasetError, RawDatasetFailure> {
    read_fcs_raw_text_inner(p, &conf.standard.raw)
        .def_io_into()
        .def_and_maybe(|(raw, mut h, st)| {
            h_read_dataset_from_kws(
                &mut h,
                raw.version,
                &raw.keywords.std,
                raw.parse.header_segments.data,
                raw.parse.header_segments.analysis,
                &raw.parse.header_segments.other[..],
                &st.replace_inner(conf),
            )
            .def_map_value(|dataset| RawDatasetOutput { text: raw, dataset })
            .def_io_into()
        })
        .def_terminate_maybe_warn(
            RawDatasetFailure,
            conf.standard.raw.warnings_are_errors,
            |w| ImpureError::Pure(RawDatasetError::from(w)),
        )
}

/// Read dataset from FCS file using raw key/value pairs from TEXT.
pub fn fcs_read_std_dataset(
    p: &path::PathBuf,
    conf: &DataReadConfig,
) -> IOTerminalResult<StdDatasetOutput, StdDatasetWarning, StdDatasetError, StdDatasetFailure> {
    read_fcs_raw_text_inner(p, &conf.standard.raw)
        .def_io_into()
        .def_and_maybe(|(raw, mut h, st)| {
            raw.into_std_dataset(&mut h, &st.replace_inner(conf))
                .def_io_into()
        })
        .def_terminate_maybe_warn(
            StdDatasetFailure,
            conf.standard.raw.warnings_are_errors,
            |w| ImpureError::Pure(StdDatasetError::from(w)),
        )
}

/// Read DATA/ANALYSIS in FCS file using provided keywords.
pub fn fcs_read_raw_dataset_with_keywords(
    p: path::PathBuf,
    version: Version,
    std: &StdKeywords,
    data_seg: HeaderDataSegment,
    analysis_seg: HeaderAnalysisSegment,
    other_segs: Vec<OtherSegment>,
    conf: &DataReadConfig,
) -> IOTerminalResult<
    RawDatasetWithKwsOutput,
    LookupAndReadDataAnalysisWarning,
    LookupAndReadDataAnalysisError,
    RawDatasetWithKwsFailure,
> {
    fs::File::options()
        .read(true)
        .open(p)
        .and_then(|file| ReadState::init(&file, conf).map(|st| (st, file)))
        .into_deferred()
        .def_and_maybe(|(st, file)| {
            let mut h = BufReader::new(file);
            h_read_dataset_from_kws(
                &mut h,
                version,
                std,
                data_seg,
                analysis_seg,
                &other_segs[..],
                &st,
            )
        })
        .def_terminate_maybe_warn(
            RawDatasetWithKwsFailure,
            conf.standard.raw.warnings_are_errors,
            |w| ImpureError::Pure(LookupAndReadDataAnalysisError::from(w)),
        )
}

/// Read DATA/ANALYSIS in FCS file using provided keywords to be standardized.
pub fn fcs_read_std_dataset_with_keywords(
    p: &path::PathBuf,
    version: Version,
    mut kws: ValidKeywords,
    data_seg: HeaderDataSegment,
    analysis_seg: HeaderAnalysisSegment,
    other_segs: Vec<OtherSegment>,
    conf: &DataReadConfig,
) -> IOTerminalResult<
    StdDatasetWithKwsOutput,
    StdDatasetFromRawWarning,
    StdDatasetFromRawError,
    StdDatasetWithKwsFailure,
> {
    fs::File::options()
        .read(true)
        .open(p)
        .and_then(|file| ReadState::init(&file, conf).map(|st| (st, file)))
        .into_deferred()
        .def_and_maybe(|(st, file)| {
            let mut h = BufReader::new(file);
            AnyCoreDataset::parse_raw(
                &mut h,
                version,
                &mut kws.std,
                kws.nonstd,
                data_seg,
                analysis_seg,
                &other_segs[..],
                &st,
            )
            .def_map_value(|(core, d_seg, a_seg)| StdDatasetWithKwsOutput {
                standardized: DatasetWithSegments {
                    core,
                    data_seg: d_seg,
                    analysis_seg: a_seg,
                },
                pseudostandard: kws.std,
            })
        })
        .def_terminate_maybe_warn(
            StdDatasetWithKwsFailure,
            conf.standard.raw.warnings_are_errors,
            |w| ImpureError::Pure(StdDatasetFromRawError::from(w)),
        )
}

/// Output from parsing the TEXT segment.
#[derive(Serialize)]
pub struct RawTEXTOutput {
    /// FCS version
    pub version: Version,

    /// Keywords from TEXT
    pub keywords: ValidKeywords,

    /// Miscellaneous data from parsing TEXT
    pub parse: RawTEXTParseData,
}

/// Output of parsing the TEXT segment and standardizing keywords.
pub struct StdTEXTOutput {
    /// Standardized data from TEXT
    pub standardized: AnyCoreTEXT,

    /// TEXT value for $TOT
    ///
    /// This should always be Some for 3.0+ and might be None for 2.0.
    pub tot: Option<Tot>,

    /// TEXT value for $TIMESTEP if a time channel was not found (3.0+)
    pub timestep: Option<String>,

    /// Segment for DATA
    pub data: AnyDataSegment,

    /// Segment for ANALYSIS
    pub analysis: AnyAnalysisSegment,

    /// Keywords that start with '$' that are not part of the standard
    pub pseudostandard: StdKeywords,

    /// Miscellaneous data from parsing TEXT
    pub parse: RawTEXTParseData,
}

/// Output of parsing one raw dataset (TEXT+DATA) from an FCS file.
pub struct RawDatasetOutput {
    /// Output from parsing HEADER+TEXT
    pub text: RawTEXTOutput,

    /// Output from parsing DATA+ANALYSIS
    pub dataset: RawDatasetWithKwsOutput,
}

/// Output of parsing one standardized dataset (TEXT+DATA) from an FCS file.
pub struct StdDatasetOutput {
    /// Standardized data from one FCS dataset
    pub dataset: StdDatasetWithKwsOutput,

    /// Miscellaneous data from parsing TEXT
    pub parse: RawTEXTParseData,
}

/// Output of using keywords to read standardized TEXT+DATA
pub struct StdDatasetWithKwsOutput {
    /// DATA+ANALYSIS
    pub standardized: DatasetWithSegments,

    /// Keywords that start with '$' that are not part of the standard
    pub pseudostandard: StdKeywords,
}

/// Output of using keywords to read raw TEXT+DATA
pub struct RawDatasetWithKwsOutput {
    /// DATA output
    pub data: FCSDataFrame,

    /// ANALYSIS output
    pub analysis: Analysis,

    /// OTHER output(s)
    pub others: Others,

    /// offsets used to parse DATA
    pub data_seg: AnyDataSegment,

    /// offsets used to parse ANALYSIS
    pub analysis_seg: AnyAnalysisSegment,
}

/// Data pertaining to parsing the TEXT segment.
#[derive(Clone, Serialize)]
pub struct RawTEXTParseData {
    /// Offsets read from HEADER
    pub header_segments: HeaderSegments,

    /// Supplemental TEXT offsets
    ///
    /// This is not needed downstream and included here for informational
    /// purposes. It will always be None for 2.0 which does not include this.
    pub supp_text: Option<SupplementalTextSegment>,

    /// NEXTDATA offset
    ///
    /// This will be copied as represented in TEXT. If it is 0, there is no next
    /// dataset, otherwise it points to the next dataset in the file.
    pub nextdata: Option<u32>,

    /// Delimiter used to parse TEXT.
    ///
    /// Included here for informational purposes.
    pub delimiter: u8,

    /// Keywords with a non-ASCII but still valid UTF-8 key.
    ///
    /// Non-ASCII keys are non-conforment but are included here in case the user
    /// wants to fix them or know they are present
    pub non_ascii: NonAsciiPairs,

    /// Keywords that could not be parsed.
    ///
    /// These have either a key or value or both that is not a UTF-8 string.
    /// Included here for debugging
    pub byte_pairs: BytesPairs,
}

// /// Raw TEXT values for $BEGIN/END* keywords
// pub struct SegmentKeywords {
//     pub begin: Option<String>,
//     pub end: Option<String>,
// }

/// Standardized TEXT+DATA+ANALYSIS with DATA+ANALYSIS offsets
pub struct DatasetWithSegments {
    /// Standardized dataset
    pub core: AnyCoreDataset,

    /// offsets used to parse DATA
    pub data_seg: AnyDataSegment,

    /// offsets used to parse ANALYSIS
    pub analysis_seg: AnyAnalysisSegment,
}

pub struct HeaderFailure;

pub struct RawTEXTFailure;

pub struct RawDatasetFailure;

pub struct RawDatasetWithKwsFailure;

pub struct StdTEXTFailure;

pub struct StdDatasetFailure;

pub struct StdDatasetWithKwsFailure;

#[derive(From, Display)]
pub enum StdTEXTWarning {
    Raw(ParseRawTEXTWarning),
    Std(StdTEXTFromRawWarning),
}

#[derive(From, Display)]
pub enum StdTEXTError {
    Raw(HeaderOrRawError),
    Std(StdTEXTFromRawError),
    Warn(StdTEXTWarning),
}

#[derive(From, Display)]
pub enum StdDatasetWarning {
    Raw(ParseRawTEXTWarning),
    Std(StdDatasetFromRawWarning),
}

#[derive(From, Display)]
pub enum StdDatasetError {
    Raw(HeaderOrRawError),
    Std(StdDatasetFromRawError),
    Warn(StdDatasetWarning),
}

#[derive(From, Display)]
pub enum RawDatasetWarning {
    Raw(ParseRawTEXTWarning),
    Read(LookupAndReadDataAnalysisWarning),
}

#[derive(From, Display)]
pub enum RawDatasetError {
    Raw(HeaderOrRawError),
    Read(LookupAndReadDataAnalysisError),
    Warn(RawDatasetWarning),
}

#[derive(From, Display)]
pub enum ParseRawTEXTWarning {
    Char(DelimCharError),
    Keywords(ParseKeywordsIssue),
    SuppOffsets(STextSegmentWarning),
    Nextdata(ParseKeyError<ParseIntError>),
    Nonstandard(NonstandardError),
}

#[derive(From, Display)]
pub enum HeaderOrRawError {
    Header(HeaderError),
    RawTEXT(ParseRawTEXTError),
    Warn(ParseRawTEXTWarning),
}

#[derive(From, Display)]
pub enum RawToReaderError {
    Layout(RawToLayoutError),
    Reader(NewDataReaderError),
}

#[derive(From, Display)]
pub enum RawToReaderWarning {
    Layout(RawToLayoutWarning),
    Reader(NewDataReaderWarning),
}

#[derive(From, Display)]
pub enum STextSegmentError {
    ReqSegment(ReqSegmentError),
    Dup(DuplicatedSuppTEXT),
}

#[derive(From, Display)]
pub enum STextSegmentWarning {
    ReqSegment(ReqSegmentError),
    OptSegment(OptSegmentError),
    Dup(DuplicatedSuppTEXT),
}

pub struct DuplicatedSuppTEXT;

#[derive(From, Display)]
pub enum ParseRawTEXTError {
    Delim(DelimVerifyError),
    Primary(ParsePrimaryTEXTError),
    Supplemental(ParseSupplementalTEXTError),
    SuppOffsets(STextSegmentError),
    Nextdata(ReqKeyError<ParseIntError>),
    NonAscii(NonAsciiKeyError),
    NonUtf8(NonUtf8KeywordError),
    Nonstandard(NonstandardError),
    Header(Box<HeaderValidationError>),
}

#[derive(From, Display)]
pub enum DelimVerifyError {
    Empty(EmptyTEXTError),
    Char(DelimCharError),
}

pub struct DelimCharError(u8);

pub struct EmptyTEXTError;

#[derive(Debug)]
pub struct BlankKeyError;

#[derive(Debug)]
pub struct UnevenWordsError;

#[derive(Debug)]
pub struct FinalDelimError;

#[derive(Debug)]
pub struct DelimBoundError;

#[derive(From, Display)]
pub enum ParsePrimaryTEXTError {
    Keywords(ParseKeywordsIssue),
    Empty(NoTEXTWordsError),
}

pub struct NoTEXTWordsError;

#[derive(Debug, Display, From)]
pub enum ParseKeywordsIssue {
    BlankKey(BlankKeyError),
    BlankValue(BlankValueError),
    Uneven(UnevenWordsError),
    Final(FinalDelimError),
    Insert(KeywordInsertError),
    Bound(DelimBoundError),
    // this is only for supp TEXT but seems less wasteful/convoluted to put here
    Mismatch(DelimMismatch),
}

#[derive(From, Display)]
pub enum ParseSupplementalTEXTError {
    Keywords(ParseKeywordsIssue),
    Mismatch(DelimMismatch),
}

#[derive(Debug, Clone)]
pub struct DelimMismatch {
    supp: u8,
    delim: u8,
}

pub struct NonAsciiKeyError(String);

pub struct NonUtf8KeywordError {
    key: Vec<u8>,
    value: Vec<u8>,
}

pub struct NonstandardError;

#[allow(clippy::type_complexity)]
fn read_fcs_raw_text_inner<'a>(
    p: &path::PathBuf,
    conf: &'a RawTextReadConfig,
) -> DeferredResult<
    (
        RawTEXTOutput,
        BufReader<fs::File>,
        ReadState<'a, RawTextReadConfig>,
    ),
    ParseRawTEXTWarning,
    ImpureError<HeaderOrRawError>,
> {
    fs::File::options()
        .read(true)
        .open(p)
        .and_then(|file| ReadState::init(&file, conf).map(|st| (st, file)))
        .into_deferred()
        .def_and_maybe(|(st, file)| {
            let mut h = BufReader::new(file);
            RawTEXTOutput::h_read(&mut h, &st).def_map_value(|x| (x, h, st))
        })
}

fn h_read_dataset_from_kws<R: Read + Seek>(
    h: &mut BufReader<R>,
    version: Version,
    kws: &StdKeywords,
    data_seg: HeaderDataSegment,
    analysis_seg: HeaderAnalysisSegment,
    other_segs: &[OtherSegment],
    st: &ReadState<DataReadConfig>,
) -> IODeferredResult<
    RawDatasetWithKwsOutput,
    LookupAndReadDataAnalysisWarning,
    LookupAndReadDataAnalysisError,
> {
    kws_to_df_analysis(version, h, kws, data_seg, analysis_seg, st)
        .def_inner_into()
        .def_and_maybe(|(data, analysis, _data_seg, _analysis_seg)| {
            let or = OthersReader { segs: other_segs };
            or.h_read(h)
                .into_deferred()
                .def_map_value(|others| RawDatasetWithKwsOutput {
                    data,
                    analysis,
                    others,
                    data_seg: _data_seg,
                    analysis_seg: _analysis_seg,
                })
        })
}

impl RawTEXTOutput {
    fn h_read<R: Read + Seek>(
        h: &mut BufReader<R>,
        st: &ReadState<RawTextReadConfig>,
    ) -> DeferredResult<Self, ParseRawTEXTWarning, ImpureError<HeaderOrRawError>> {
        Header::h_read(h, &st.map_inner(|conf| &conf.header))
            .mult_to_deferred()
            .def_map_errors(|e: ImpureError<HeaderError>| e.inner_into())
            .def_and_maybe(|header| {
                h_read_raw_text_from_header(h, header, st).def_map_errors(|e| e.inner_into())
            })
    }

    fn into_std_text(
        self,
        st: &ReadState<StdTextReadConfig>,
    ) -> DeferredResult<StdTEXTOutput, StdTEXTFromRawWarning, StdTEXTFromRawError> {
        let mut kws = self.keywords;
        let header = &self.parse.header_segments;
        AnyCoreTEXT::parse_raw(
            self.version,
            &mut kws.std,
            kws.nonstd,
            header.data,
            header.analysis,
            st,
        )
        .def_map_value(|(standardized, offsets)| {
            let timestep = kws.std.remove(&Timestep::std());
            StdTEXTOutput {
                parse: self.parse,
                standardized,
                tot: offsets.tot,
                timestep,
                data: offsets.data,
                analysis: offsets.analysis,
                pseudostandard: kws.std,
            }
        })
    }

    fn into_std_dataset<R: Read + Seek>(
        self,
        h: &mut BufReader<R>,
        st: &ReadState<DataReadConfig>,
    ) -> DeferredResult<
        StdDatasetOutput,
        StdDatasetFromRawWarning,
        ImpureError<StdDatasetFromRawError>,
    > {
        let mut kws = self.keywords;
        AnyCoreDataset::parse_raw(
            h,
            self.version,
            &mut kws.std,
            kws.nonstd,
            self.parse.header_segments.data,
            self.parse.header_segments.analysis,
            &self.parse.header_segments.other[..],
            st,
        )
        .def_map_value(|(core, data_seg, analysis_seg)| StdDatasetOutput {
            dataset: StdDatasetWithKwsOutput {
                standardized: DatasetWithSegments {
                    core,
                    data_seg,
                    analysis_seg,
                },
                pseudostandard: kws.std,
            },
            parse: self.parse,
        })
    }
}

fn kws_to_df_analysis<R: Read + Seek>(
    version: Version,
    h: &mut BufReader<R>,
    kws: &StdKeywords,
    data: HeaderDataSegment,
    analysis: HeaderAnalysisSegment,
    st: &ReadState<DataReadConfig>,
) -> IODeferredResult<
    (FCSDataFrame, Analysis, AnyDataSegment, AnyAnalysisSegment),
    LookupAndReadDataAnalysisWarning,
    LookupAndReadDataAnalysisError,
> {
    match version {
        Version::FCS2_0 => Version2_0::h_lookup_and_read(h, kws, data, analysis, st),
        Version::FCS3_0 => Version3_0::h_lookup_and_read(h, kws, data, analysis, st),
        Version::FCS3_1 => Version3_1::h_lookup_and_read(h, kws, data, analysis, st),
        Version::FCS3_2 => Version3_2::h_lookup_and_read(h, kws, data, analysis, st),
    }
}

fn h_read_raw_text_from_header<R: Read + Seek>(
    h: &mut BufReader<R>,
    header: Header,
    st: &ReadState<RawTextReadConfig>,
) -> IODeferredResult<RawTEXTOutput, ParseRawTEXTWarning, ParseRawTEXTError> {
    let conf = &st.conf;
    let mut buf = vec![];
    let ptext_seg = header.segments.text;
    ptext_seg
        .inner
        .h_read_contents(h, &mut buf)
        .into_deferred()?;

    let tnt_delim = split_first_delim(&buf, conf)
        .def_inner_into()
        .def_errors_liftio()?;

    let kws_res = tnt_delim
        .and_maybe(|(delim, bytes)| {
            let kws = ParsedKeywords::default();
            split_raw_primary_text(kws, delim, bytes, conf)
                .def_inner_into()
                .def_errors_liftio()
                .def_map_value(|_kws| (delim, _kws))
        })
        .def_and_maybe(|(delim, mut kws)| {
            if conf.ignore_supp_text {
                // NOTE rip out the STEXT keywords so they don't trigger a false
                // positive pseudostandard keyword error later
                let _ = kws.std.remove(&Beginstext::std());
                let _ = kws.std.remove(&Endstext::std());
                Ok(Tentative::new1((delim, kws, None)))
            } else {
                lookup_stext_offsets(&mut kws.std, header.version, ptext_seg, st)
                    .errors_into()
                    .errors_liftio()
                    .warnings_into()
                    .map(|s| (s, kws))
                    .and_maybe(|(maybe_supp_seg, _kws)| {
                        let tnt_supp_kws = if let Some(seg) = maybe_supp_seg {
                            buf.clear();
                            seg.inner
                                .h_read_contents(h, &mut buf)
                                .map_err(|e| DeferredFailure::new1(e.into()))?;
                            split_raw_supp_text(_kws, delim, &buf, conf)
                                .inner_into()
                                .errors_liftio()
                        } else {
                            Tentative::new1(_kws)
                        };
                        Ok(tnt_supp_kws.map(|k| (delim, k, maybe_supp_seg)))
                    })
            }
        });

    let repair_res = kws_res.def_and_tentatively(|(delim, mut kws, supp_text_seg)| {
        repair_keywords(&mut kws.std, conf);
        append_keywords(&mut kws, conf)
            .map_or_else(
                |es| {
                    Leveled::many_to_tentative(es.into())
                        .map_errors(KeywordInsertError::from)
                        .map_errors(ParseKeywordsIssue::from)
                        .map_errors(ParsePrimaryTEXTError::from)
                        .map_warnings(KeywordInsertError::from)
                        .map_warnings(ParseKeywordsIssue::from)
                        .inner_into()
                        .errors_liftio()
                },
                |_| Tentative::default(),
            )
            .map(|_| (delim, kws, supp_text_seg))
    });

    repair_res.def_and_tentatively(|(delimiter, kws, supp_text_seg)| {
        let mut tnt_parse = lookup_nextdata(&kws.std, conf.allow_missing_nextdata)
            .errors_into()
            .map(|nextdata| RawTEXTParseData {
                header_segments: header.segments,
                supp_text: supp_text_seg,
                nextdata,
                delimiter,
                non_ascii: kws.non_ascii,
                byte_pairs: kws.byte_pairs,
            });

        // throw errors if we found any non-ascii keywords and we want to know
        tnt_parse.eval_errors(|pd| {
            if conf.allow_non_ascii_keywords {
                vec![]
            } else {
                pd.non_ascii
                    .iter()
                    .map(|(k, _)| ParseRawTEXTError::NonAscii(NonAsciiKeyError(k.clone())))
                    .collect()
            }
        });

        // throw errors if we found any non-utf8 keywords and we want to know
        tnt_parse.eval_errors(|pd| {
            if conf.allow_non_utf8 {
                vec![]
            } else {
                pd.byte_pairs
                    .iter()
                    .map(|(k, v)| {
                        ParseRawTEXTError::NonUtf8(NonUtf8KeywordError {
                            key: k.clone(),
                            value: v.clone(),
                        })
                    })
                    .collect()
            }
        });

        // throw errors if the supp text segment overlaps with HEADER or
        // anything else
        tnt_parse.eval_errors(|pd| {
            if let Some(s) = pd.supp_text {
                let x = pd.header_segments.contains_text_segment(s).into_mult();
                let y = pd.header_segments.overlaps_with(s).mult_errors_into();
                x.mult_zip(y)
                    .mult_map_errors(Box::new)
                    .mult_map_errors(ParseRawTEXTError::Header)
                    .err()
                    .map(|n| n.into())
                    .unwrap_or_default()
            } else {
                vec![]
            }
        });

        tnt_parse
            .inner_into()
            .map(|parse| RawTEXTOutput {
                version: header.version,
                parse,
                keywords: ValidKeywords {
                    std: kws.std,
                    nonstd: kws.nonstd,
                },
            })
            .errors_liftio()
    })
}

fn split_first_delim<'a>(
    bytes: &'a [u8],
    conf: &RawTextReadConfig,
) -> DeferredResult<(u8, &'a [u8]), DelimCharError, DelimVerifyError> {
    if let Some((delim, rest)) = bytes.split_first() {
        let mut tnt = Tentative::new1((*delim, rest));
        if !(1..=126).contains(delim) {
            tnt.push_error_or_warning(DelimCharError(*delim), !conf.allow_non_ascii_delim);
        }
        Ok(tnt)
    } else {
        Err(DeferredFailure::new1(EmptyTEXTError.into()))
    }
}

fn split_raw_primary_text(
    kws: ParsedKeywords,
    delim: u8,
    bytes: &[u8],
    conf: &RawTextReadConfig,
) -> DeferredResult<ParsedKeywords, ParseKeywordsIssue, ParsePrimaryTEXTError> {
    if bytes.is_empty() {
        Err(DeferredFailure::new1(NoTEXTWordsError.into()))
    } else {
        Ok(split_raw_text_inner(kws, delim, bytes, conf).errors_into())
    }
}

fn split_raw_supp_text(
    kws: ParsedKeywords,
    delim: u8,
    bytes: &[u8],
    conf: &RawTextReadConfig,
) -> Tentative<ParsedKeywords, ParseKeywordsIssue, ParseSupplementalTEXTError> {
    if let Some((byte0, rest)) = bytes.split_first() {
        let mut tnt = split_raw_text_inner(kws, *byte0, rest, conf).errors_into();
        if *byte0 != delim {
            let x = DelimMismatch {
                delim,
                supp: *byte0,
            };
            if conf.allow_stext_own_delim {
                tnt.push_error(x.into());
            } else {
                tnt.push_warning(x.into());
            }
        }
        tnt
    } else {
        // if empty do nothing, this is expected for most files
        Tentative::new1(kws)
    }
}

fn split_raw_text_inner(
    kws: ParsedKeywords,
    delim: u8,
    bytes: &[u8],
    conf: &RawTextReadConfig,
) -> Tentative<ParsedKeywords, ParseKeywordsIssue, ParseKeywordsIssue> {
    if conf.use_literal_delims {
        split_raw_text_literal_delim(kws, delim, bytes, conf)
    } else {
        split_raw_text_escaped_delim(kws, delim, bytes, conf)
    }
}

fn split_raw_text_literal_delim(
    mut kws: ParsedKeywords,
    delim: u8,
    bytes: &[u8],
    conf: &RawTextReadConfig,
) -> Tentative<ParsedKeywords, ParseKeywordsIssue, ParseKeywordsIssue> {
    let mut errors = vec![];
    let mut warnings = vec![];

    let mut push_issue = |is_warning, error| {
        if is_warning {
            warnings.push(error);
        } else {
            errors.push(error);
        }
    };

    // ASSUME input slice does not start with delim
    let mut it = bytes.split(|x| *x == delim);
    let mut prev_was_blank = false;
    let mut prev_was_key = false;

    while let Some(key) = it.next() {
        prev_was_key = true;
        prev_was_blank = key.is_empty();
        if key.is_empty() {
            if let Some(value) = it.next() {
                prev_was_key = false;
                prev_was_blank = value.is_empty();
                push_issue(conf.allow_empty, BlankKeyError.into());
            } else {
                // if everything is correct, we should exit here since the
                // last word will be the blank slice after the final delim
                break;
            }
        } else if let Some(value) = it.next() {
            prev_was_key = false;
            prev_was_blank = value.is_empty();
            if value.is_empty() {
                push_issue(conf.allow_empty, BlankValueError(key.to_vec()).into());
            } else if let Err(lvl) = kws.insert(key, value, conf) {
                match lvl.inner_into() {
                    Leveled::Error(e) => push_issue(false, e),
                    Leveled::Warning(w) => push_issue(true, w),
                }
            }
        } else {
            // exiting here means we found a key without a value and also didn't
            // end with a delim
            break;
        }
    }

    if !prev_was_key {
        push_issue(conf.allow_odd, UnevenWordsError.into());
    }

    if !prev_was_blank {
        push_issue(conf.allow_missing_final_delim, FinalDelimError.into());
    }

    Tentative::new(kws, warnings, errors)
}

fn split_raw_text_escaped_delim(
    mut kws: ParsedKeywords,
    delim: u8,
    bytes: &[u8],
    conf: &RawTextReadConfig,
) -> Tentative<ParsedKeywords, ParseKeywordsIssue, ParseKeywordsIssue> {
    let mut ews = (vec![], vec![]);

    let push_issue = |_ews: &mut (Vec<_>, Vec<_>), is_warning, error| {
        let warnings = &mut _ews.0;
        let errors = &mut _ews.1;
        if is_warning {
            warnings.push(error);
        } else {
            errors.push(error);
        }
    };

    let mut push_pair = |_ews: &mut (Vec<_>, Vec<_>), kb: &Vec<_>, vb: &Vec<_>| {
        if let Err(lvl) = kws.insert(kb, vb, conf) {
            match lvl.inner_into() {
                Leveled::Error(e) => push_issue(_ews, false, e),
                Leveled::Warning(w) => push_issue(_ews, true, w),
            }
        }
    };

    let push_delim = |kb: &mut Vec<_>, vb: &mut Vec<_>, k: usize| {
        let n = (k + 1) / 2;
        let buf = if vb.is_empty() { kb } else { vb };
        for _ in 0..n {
            buf.push(delim);
        }
    };

    // ASSUME input slice does not start with delim
    let mut consec_blanks = 0;
    let mut keybuf: Vec<u8> = vec![];
    let mut valuebuf: Vec<u8> = vec![];

    for segment in bytes.split(|x| *x == delim) {
        if segment.is_empty() {
            consec_blanks += 1;
        } else {
            if consec_blanks & 1 == 0 {
                // Previous number of delimiters is odd, treat this as a word
                // boundary
                if !valuebuf.is_empty() {
                    push_pair(&mut ews, &keybuf, &valuebuf);
                    keybuf.clear();
                    valuebuf.clear();
                    keybuf.extend_from_slice(segment);
                } else if !keybuf.is_empty() {
                    valuebuf.extend_from_slice(segment);
                } else {
                    // this should only be reached on first iteration
                    keybuf.extend_from_slice(segment);
                }
                if consec_blanks > 0 {
                    push_issue(
                        &mut ews,
                        conf.allow_delim_at_boundary,
                        DelimBoundError.into(),
                    );
                }
            } else {
                // Previous consecutive delimiter sequence was even. Push n / 2
                // delimiters to whatever the current word is. Then push to
                // key or value
                push_delim(&mut keybuf, &mut valuebuf, consec_blanks);
                if !valuebuf.is_empty() {
                    valuebuf.extend_from_slice(segment);
                } else {
                    keybuf.extend_from_slice(segment);
                }
            }
            consec_blanks = 0;
        }
    }

    // If all went perfectly, we should have one consecutive blank at this point
    // since the space between the last delim and the end will show up as a
    // blank.
    //
    // If we have 0, then there was no delim at the end, which is an error.
    //
    // If number of blanks is even and not 0, then the last word ended with one
    // or more escaped delimiters, but the TEXT didn't (2 errors, delim at
    // boundary and no delim ending TEXT). Note that here, blanks = number of
    // literal delimiters, whereas in the loop, this corresponded to blanks + 1
    // delimiters.
    //
    // If number of blanks is odd but not 1, the last word ended with one or
    // more escaped delimiters (error: on a boundary) and the TEXT ended with a
    // delimiter (not an error).

    if consec_blanks == 0 {
        push_issue(
            &mut ews,
            conf.allow_missing_final_delim,
            FinalDelimError.into(),
        );
    } else if consec_blanks > 1 {
        push_issue(
            &mut ews,
            conf.allow_delim_at_boundary,
            DelimBoundError.into(),
        );
        push_delim(&mut keybuf, &mut valuebuf, consec_blanks);

        if consec_blanks & 1 == 1 {
            push_issue(
                &mut ews,
                conf.allow_missing_final_delim,
                FinalDelimError.into(),
            );
        }
    }

    if valuebuf.is_empty() {
        push_issue(&mut ews, conf.allow_odd, UnevenWordsError.into());
    } else {
        push_pair(&mut ews, &keybuf, &valuebuf);
    }

    Tentative::new(kws, ews.0, ews.1)
}

fn repair_keywords(kws: &mut StdKeywords, conf: &RawTextReadConfig) {
    for (key, v) in kws.iter_mut() {
        // TODO generalized this and possibly put in a trait
        if key == &FCSDate::std() {
            if let Some(pattern) = &conf.date_pattern {
                if let Ok(d) = NaiveDate::parse_from_str(v, pattern.as_ref()) {
                    *v = FCSDate(d).to_string();
                }
            }
        }
    }
}

fn append_keywords(
    kws: &mut ParsedKeywords,
    conf: &RawTextReadConfig,
) -> MultiResult<(), Leveled<StdPresent>> {
    kws.append_std(&conf.append_standard_keywords, conf.allow_nonunique)
}

fn lookup_stext_offsets(
    kws: &mut StdKeywords,
    version: Version,
    text_segment: PrimaryTextSegment,
    st: &ReadState<RawTextReadConfig>,
) -> Tentative<Option<SupplementalTextSegment>, STextSegmentWarning, STextSegmentError> {
    let conf = &st.conf;
    let seg_conf = NewSegmentConfig {
        corr: conf.supp_text_correction,
        file_len: Some(st.file_len.into()),
        truncate_offsets: conf.header.truncate_offsets,
    };
    match version {
        Version::FCS2_0 => Tentative::new1(None),
        Version::FCS3_0 | Version::FCS3_1 => KeyedReqSegment::get_mult(kws, &seg_conf).map_or_else(
            |es| Tentative::new_either(None, es.into(), conf.allow_missing_stext),
            |t| Tentative::new1(Some(t)),
        ),
        Version::FCS3_2 => KeyedOptSegment::get(kws, &seg_conf).warnings_into(),
    }
    .and_tentatively(|x| {
        x.map(|seg| {
            if seg.inner.as_u64() == text_segment.inner.as_u64() {
                Tentative::new_either(None, vec![DuplicatedSuppTEXT], !conf.allow_duplicated_stext)
            } else {
                Tentative::new1(Some(seg))
            }
        })
        .unwrap_or(Tentative::new1(None))
    })
}

// TODO the reason we use get instead of remove here is because we don't want to
// mess up the keyword list for raw mode, but in standardized mode we are
// consuming the hash table as a way to test for pseudostandard keywords (ie
// those that are left over). In order to reconcile these, we either need to
// make two raw text reader functions which either take immutable or mutable kws
// or use a more clever hash table that marks keys when we see them.
fn lookup_nextdata(
    kws: &StdKeywords,
    enforce: bool,
) -> Tentative<Option<u32>, ParseKeyError<ParseIntError>, ReqKeyError<ParseIntError>> {
    let k = Nextdata::std();
    if enforce {
        get_req(kws, k).map_or_else(
            |e| Tentative::new(None, vec![], vec![e]),
            |t| Tentative::new1(Some(t)),
        )
    } else {
        get_opt(kws, k).map_or_else(|w| Tentative::new(None, vec![w], vec![]), Tentative::new1)
    }
}

impl fmt::Display for DuplicatedSuppTEXT {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.write_str("primary and supplemental TEXT are duplicated")
    }
}

impl fmt::Display for DelimCharError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "delimiter must be ASCII character 1-126 inclusive, got {}",
            self.0
        )
    }
}

impl fmt::Display for EmptyTEXTError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "TEXT segment is empty")
    }
}

impl fmt::Display for BlankKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "encountered blank key, skipping key and its value")
    }
}

impl fmt::Display for UnevenWordsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "TEXT segment has uneven number of words",)
    }
}

impl fmt::Display for FinalDelimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "TEXT does not end with delim",)
    }
}

impl fmt::Display for DelimBoundError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "delimiter encountered at word boundary",)
    }
}

impl fmt::Display for NoTEXTWordsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "TEXT has a delimiter and no words",)
    }
}

impl fmt::Display for DelimMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(
            f,
            "first byte of supplemental TEXT ({}) does not match delimiter of primary TEXT ({})",
            self.supp, self.delim
        )
    }
}

impl fmt::Display for NonAsciiKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "non-ASCII key encountered and dropped: {}", self.0)
    }
}

impl fmt::Display for NonUtf8KeywordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        let n = 10;
        write!(
            f,
            "non UTF-8 key/value pair encountered and dropped, \
             first 10 bytes of both are ({})/({})",
            self.key.iter().take(n).join(","),
            self.value.iter().take(n).join(",")
        )
    }
}

impl fmt::Display for NonstandardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "nonstandard keywords detected")
    }
}

impl fmt::Display for HeaderFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "could not parse HEADER")
    }
}

impl fmt::Display for RawTEXTFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "could not parse TEXT segment")
    }
}

impl fmt::Display for StdTEXTFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "could not standardize TEXT segment")
    }
}

impl fmt::Display for StdDatasetFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "could not read DATA with standardized TEXT")
    }
}

impl fmt::Display for RawDatasetFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "could not read DATA with raw TEXT")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_text_escape() {
        let kws = ParsedKeywords::default();
        let conf = RawTextReadConfig::default();
        // NOTE should not start with delim
        let bytes = "$P4F/700//75 BP/".as_bytes();
        let delim = 47;
        let out = split_raw_text_escaped_delim(kws, delim, bytes, &conf);
        let v = out
            .value()
            .std
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .next()
            .unwrap();
        let es = out.errors();
        let ws = out.warnings();
        assert_eq!(("$P4F".to_string(), "700/75 BP".to_string()), v);
        assert!(es.is_empty(), "errors: {:?}", es);
        assert!(ws.is_empty(), "warnings: {:?}", ws);
    }
}
