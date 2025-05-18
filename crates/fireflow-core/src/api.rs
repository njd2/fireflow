use crate::config::*;
pub use crate::core::*;
pub use crate::data::*;
use crate::error::*;
pub use crate::header::*;
pub use crate::header_text::*;
pub use crate::segment::*;
pub use crate::text::keywords::*;
use crate::text::timestamps::*;
use crate::validated::dataframe::FCSDataFrame;
use crate::validated::nonstandard::NonStdKey;
use crate::validated::nonstandard::NonStdKeywords;

use chrono::NaiveDate;
use itertools::Itertools;
use serde::Serialize;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fs;
use std::io::{BufReader, Read, Seek};
use std::path;
use std::str;
use std::time::{Duration, Instant};

// TODO gating parameters not added (yet)

/// Output from parsing the TEXT segment.
///
/// This is derived from the HEADER which should be parsed in order to obtain
/// this.
///
/// The purpose of this is to obtain the TEXT keywords (primary and
/// supplemental) using the least amount of processing, which should increase
/// performance and minimize potential errors thrown if this is what the user
/// desires.
///
/// This will also be used as input downstream to 'standardize' the TEXT segment
/// according to version, and also to parse DATA if either is desired.
#[derive(Clone, Serialize)]
pub struct RawTEXT {
    /// FCS Version from HEADER
    pub version: Version,

    /// Keyword pairs from TEXT
    pub keywords: AllKeywords,

    /// Data used for parsing TEXT which might be used later to parse remainder.
    ///
    /// This will include primary TEXT, DATA, and ANALYSIS offsets as seen in
    /// HEADER. It will also include $BEGIN/ENDSTEXT as found in TEXT (if found)
    /// which will be used to parse the supplemental TEXT segment if it exists.
    ///
    /// $NEXTDATA will also be included if found.
    ///
    /// The delimiter used to parse the keywords will also be included.
    pub parse: ParseParameters,
}

/// Output of parsing the TEXT segment and standardizing keywords.
///
/// This is derived from ['RawTEXT'].
///
/// The process of "standardization" involves gathering version specific
/// keywords in the TEXT segment and parsing their values such that they
/// conform to the types specified in the standard.
///
/// Version is not included since this is implied by the standardized structs
/// used.
#[derive(Clone)]
pub struct StandardizedTEXT {
    /// Structured data derived from TEXT specific to the indicated FCS version.
    ///
    /// All keywords that were included in the ['RawTEXT'] used to create this
    /// will be included here. Anything standardized will be put into a field
    /// that can be readily accessed directly and returned with the proper type.
    /// Anything nonstandard will be kept in a hash table whose values will
    /// be strings.
    pub standardized: AnyCoreTEXT,

    /// Raw standard keywords remaining after the standardization process
    ///
    /// This only should include $TOT, $BEGINDATA, $ENDDATA, $BEGINANALISYS, and
    /// $ENDANALYSIS. These are only needed to process the data segment and are
    /// not necessary to create the CoreTEXT, and thus are not included.
    pub remainder: StdKeywords,

    /// Raw keywords that are not standard but start with '$'
    pub deviant: StdKeywords,

    /// Data used for parsing TEXT which might be used later to parse remainder.
    ///
    /// The is analogous to that of [`RawTEXT`] and is copied as-is when
    /// creating this.
    pub parse: ParseParameters,
}

/// Output of parsing one raw dataset (TEXT+DATA) from an FCS file.
///
/// Computationally this will be created by skipping (most of) the
/// standardization step and instead parsing the minimal-required keywords
/// to parse DATA (BYTEORD, DATATYPE, etc).
pub struct RawDataset {
    /// FCS Version from HEADER
    pub version: Version,

    /// Keyword pairs from TEXT
    pub keywords: AllKeywords,

    /// DATA segment as a polars DataFrame
    ///
    /// The type of each column is such that each measurement is encoded with
    /// zero loss. This will/should never contain NULL values despite the
    /// underlying arrow framework allowing NULLs to exist.
    pub data: FCSDataFrame,

    /// ANALYSIS segment
    ///
    /// This will be empty if ANALYSIS either doesn't exist or the computation
    /// fails. This has not standard structure, so the best we can capture is a
    /// byte sequence.
    pub analysis: Analysis,

    /// Data used for parsing the FCS file.
    ///
    /// This will include all offsets, $NEXTDATA (if found) and the TEXT
    /// delimiter. The DATA and ANALYSIS offsets will reflect those actually
    /// used to parse these segments, which may or may not reflect the HEADER.
    pub parse: ParseParameters,
}

/// Output of parsing one standardized dataset (TEXT+DATA) from an FCS file.
#[derive(Clone)]
pub struct StandardizedDataset {
    /// Structured data derived from TEXT specific to the indicated FCS version.
    pub dataset: AnyCoreDataset,

    /// Raw standard keywords remaining after processing.
    ///
    /// This should be empty if everything worked. Here for debugging.
    pub remainder: StdKeywords,

    /// Non-standard keywords that start with '$'.
    pub deviant: StdKeywords,

    /// Data used for parsing the FCS file.
    ///
    /// This will include all offsets, $NEXTDATA (if found) and the TEXT
    /// delimiter. The DATA and ANALYSIS offsets will reflect those actually
    /// used to parse these segments, which may or may not reflect the HEADER.
    pub parse: ParseParameters,
}

/// Parameters used to parse the FCS file.
///
/// Includes offsets, TEXT delimiter, and $NEXTDATA (if present).
#[derive(Clone, Serialize)]
pub struct ParseParameters {
    /// Primary TEXT offsets
    ///
    /// The offsets that were used to parse the TEXT segment. Included here for
    /// informational purposes.
    pub prim_text: Segment,

    /// Supplemental TEXT offsets
    ///
    /// This is not needed downstream and included here for informational
    /// purposes. It will always be None for 2.0 which does not include this.
    pub supp_text: Option<Segment>,

    /// DATA offsets
    ///
    /// The offsets pointing to the DATA segment. When this struct is present
    /// in [RawTEXT] or [StandardizedTEXT], this will reflect what is in the
    /// HEADER. In [StandardizedDataset], this will reflect the values from
    /// $BEGIN/ENDDATA if applicable.
    ///
    /// This will be 0,0 if DATA has no data or if there was an error acquiring
    /// the offsets.
    pub data: Segment,

    /// ANALYSIS offsets.
    ///
    /// The meaning of this is analogous to [data] above.
    pub analysis: Segment,

    /// NEXTDATA offset
    ///
    /// This will be copied as represented in TEXT. If it is 0, there is no next
    /// dataset, otherwise it points to the next dataset in the file.
    pub nextdata: Option<u32>,

    /// Delimiter used to parse TEXT.
    ///
    /// Included here for informational purposes.
    pub delimiter: u8,
}

/// Return header in an FCS file.
///
/// The header contains the version and offsets for the TEXT, DATA, and ANALYSIS
/// segments, all of which are present in fixed byte offset segments. This
/// function will fail and return an error if the file does not follow this
/// structure. Will also check that the begin and end segments are not reversed.
///
/// Depending on the version, all of these except the TEXT offsets might be 0
/// which indicates they are actually stored in TEXT due to size limitations.
pub fn read_fcs_header(p: &path::PathBuf, conf: &HeaderConfig) -> ImpureResult<Header> {
    let file = fs::File::options().read(true).open(p)?;
    let mut reader = BufReader::new(file);
    h_read_header(&mut reader, conf)
}

/// Return header and raw key/value metadata pairs in an FCS file.
///
/// First will parse the header according to [`read_fcs_header`]. If this fails
/// an error will be returned.
///
/// Next will use the offset information in the header to parse the TEXT segment
/// for key/value pairs. On success will return these pairs as-is using Strings
/// in a HashMap. No other processing will be performed.
pub fn read_fcs_raw_text(p: &path::PathBuf, conf: &RawTextReadConfig) -> ImpureResult<RawTEXT> {
    let file = fs::File::options().read(true).open(p)?;
    let mut h = BufReader::new(file);
    RawTEXT::h_read(&mut h, conf)
}

/// Return header and standardized metadata in an FCS file.
///
/// Begins by parsing header and raw keywords according to [`read_fcs_raw_text`]
/// and will return error if this function fails.
///
/// Next, all keywords in the TEXT segment will be validated to conform to the
/// FCS standard indicated in the header and returned in a struct storing each
/// key/value pair in a standardized manner. This will halt and return any
/// errors encountered during this process.
pub fn read_fcs_std_text(
    p: &path::PathBuf,
    conf: &StdTextReadConfig,
) -> ImpureResult<StandardizedTEXT> {
    let raw_succ = read_fcs_raw_text(p, &conf.raw)?;
    let out = raw_succ.try_map(|raw| raw.into_std(conf))?;
    Ok(out)
}

/// Return header, raw metadata, and data in an FCS file.
///
/// In contrast to [`read_fcs_file`], this will return the keywords as a flat
/// list of key/value pairs. Only the bare minimum of these will be read in
/// order to determine how to parse the DATA segment (including $DATATYPE,
/// $BYTEORD, etc). No other checks will be performed to ensure the metadata
/// conforms to the FCS standard version indicated in the header.
///
/// This might be useful for applications where one does not necessarily need
/// the strict structure of the standardized metadata, or if one does not care
/// too much about the degree to which the metadata conforms to standard.
///
/// Other than this, behavior is identical to [`read_fcs_file`],
pub fn read_fcs_raw_file(p: path::PathBuf, conf: &DataReadConfig) -> ImpureResult<RawDataset> {
    let file = fs::File::options().read(true).open(p)?;
    let mut h = BufReader::new(file);
    RawTEXT::h_read(&mut h, &conf.standard.raw)?
        .try_map(|raw| h_read_raw_dataset(&mut h, raw, conf))
}

/// Return header, structured metadata, and data in an FCS file.
///
/// Begins by parsing header and raw keywords according to [`read_fcs_text`]
/// and will return error if this function fails.
///
/// Next, the DATA segment will be parsed according to the metadata present
/// in TEXT.
///
/// On success will return all three of the above segments along with any
/// non-critical warnings.
///
/// The [`conf`] argument can be used to control the behavior of each reading
/// step, including the repair of non-conforming files.
pub fn read_fcs_file(
    p: &path::PathBuf,
    conf: &DataReadConfig,
) -> ImpureResult<StandardizedDataset> {
    let file = fs::File::options().read(true).open(p)?;
    let mut h = BufReader::new(file);
    let start = Instant::now();
    let res = RawTEXT::h_read(&mut h, &conf.standard.raw)?
        .try_map(|raw| raw.into_std(&conf.standard))?
        .try_map(|std| h_read_std_dataset(&mut h, std, conf));
    println!("total: {:?}", start.elapsed());
    res
}

fn h_read_raw_dataset<R: Read + Seek>(
    h: &mut BufReader<R>,
    raw: RawTEXT,
    conf: &DataReadConfig,
) -> ImpureResult<RawDataset> {
    let anal_succ =
        lookup_analysis_offsets(&raw.keywords.std, conf, raw.version, &raw.parse.analysis);
    lookup_data_offsets(&raw.keywords.std, conf, raw.version, &raw.parse.data)
        .and_then(|data_seg| {
            raw.as_reader(data_seg)
                .combine(anal_succ, |reader, analysis_seg| {
                    (reader, data_seg, analysis_seg)
                })
        })
        .try_map(|(reader_maybe, data_seg, analysis_seg)| {
            let dmsg = "could not create data parser".to_string();
            let data_parser = reader_maybe.ok_or(Failure::new(dmsg))?;
            let data = data_parser.h_read(h)?;
            let analysis = h_read_analysis(h, &analysis_seg)?;
            Ok(PureSuccess::from(RawDataset {
                version: raw.version,
                keywords: raw.keywords,
                data,
                analysis,
                parse: ParseParameters {
                    data: data_seg,
                    analysis: analysis_seg,
                    ..raw.parse
                },
            }))
        })
}

fn h_read_std_dataset<R: Read + Seek>(
    h: &mut BufReader<R>,
    std: StandardizedTEXT,
    conf: &DataReadConfig,
) -> ImpureResult<StandardizedDataset> {
    let mut kws = std.remainder;
    let version = std.standardized.version();
    let anal_succ = lookup_analysis_offsets(&kws, conf, version, &std.parse.analysis);
    lookup_data_offsets(&kws, conf, version, &std.parse.data)
        .and_then(|data_seg| {
            std.standardized
                .as_data_reader(&mut kws, conf, data_seg)
                .combine(anal_succ, |data_parser, analysis_seg| {
                    (data_parser, data_seg, analysis_seg)
                })
        })
        .try_map(|(reader_maybe, data_seg, analysis_seg)| {
            let dmsg = "could not create data reader".to_string();
            let reader = reader_maybe.ok_or(Failure::new(dmsg))?;
            let columns = reader.h_read(h)?;
            let analysis = h_read_analysis(h, &analysis_seg)?;
            let dataset =
                AnyCoreDataset::from_coretext_unchecked(std.standardized, columns, analysis);
            Ok(PureSuccess::from(StandardizedDataset {
                parse: ParseParameters {
                    data: data_seg,
                    analysis: analysis_seg,
                    ..std.parse
                },
                remainder: kws,
                // ASSUME we have checked that the dataframe has the same number
                // of columns as number of measurements; therefore, this should
                // not fail.
                dataset,
                deviant: std.deviant,
            }))
        })
}

impl RawTEXT {
    fn h_read<R: Read + Seek>(
        h: &mut BufReader<R>,
        conf: &RawTextReadConfig,
    ) -> ImpureResult<Self> {
        h_read_header(h, &conf.header)?
            .try_map(|header| h_read_raw_text_from_header(h, &header, conf))
    }

    fn into_std(self, conf: &StdTextReadConfig) -> PureResult<StandardizedTEXT> {
        AnyCoreTEXT::parse_raw(self.version, self.keywords, conf).map(|std_succ| {
            std_succ.map({
                |(standardized, kws_rem)| {
                    let (remainder, deviant) = split_remainder(kws_rem);
                    StandardizedTEXT {
                        parse: self.parse,
                        standardized,
                        remainder,
                        deviant,
                    }
                }
            })
        })
    }

    fn as_reader(&self, data_seg: Segment) -> PureMaybe<DataReader> {
        let kws = &self.keywords.std;
        match self.version {
            // TODO clean this up
            Version::FCS2_0 => {
                let res = DataLayout2_0::try_new_from_raw(kws);
                PureMaybe::from_result_errors(res).and_then_opt(|dl| dl.into_reader(kws, data_seg))
            }
            Version::FCS3_0 => {
                let res = DataLayout3_0::try_new_from_raw(kws);
                PureMaybe::from_result_errors(res).and_then_opt(|dl| dl.into_reader(kws, data_seg))
            }
            Version::FCS3_1 => {
                let res = DataLayout3_1::try_new_from_raw(kws);
                PureMaybe::from_result_errors(res).and_then_opt(|dl| dl.into_reader(kws, data_seg))
            }
            Version::FCS3_2 => {
                let res = DataLayout3_2::try_new_from_raw(kws);
                PureMaybe::from_result_errors(res).and_then_opt(|dl| dl.into_reader(kws, data_seg))
            }
        }
        .map(|x| {
            x.map(|column_reader| DataReader {
                column_reader,
                // TODO fix cast
                begin: data_seg.begin() as u64,
            })
        })
    }
}

fn verify_delim(xs: &[u8], conf: &RawTextReadConfig) -> PureSuccess<u8> {
    // First character is the delimiter
    let delimiter: u8 = xs[0];

    // Check that it is a valid UTF8 character
    //
    // TODO we technically don't need this to be true in the case of double
    // delimiters, but this is non-standard anyways and probably rare
    let mut res = PureSuccess::from(delimiter);
    if String::from_utf8(vec![delimiter]).is_err() {
        res.push_error(format!(
            "Delimiter {delimiter} is not a valid utf8 character"
        ));
    }

    // Check that the delim is valid; this is technically only written in the
    // spec for 3.1+ but for older versions this should still be true since
    // these were ASCII-everywhere
    if !(1..=126).contains(&delimiter) {
        let msg = format!("Delimiter {delimiter} is not an ASCII character b/t 1-126");
        res.push_msg_leveled(msg, conf.force_ascii_delim);
    }
    res
}

// TODO this might be optimized further by splitting into "double delim" and
// non-double-delim versions, the former case can be done much more simply. In
// either case, it will probably be possible to do more operations on raw bytes
// rather than convert to strings.
fn split_raw_text(
    xs: &[u8],
    mut kws: AllKeywords,
    delim: u8,
    conf: &RawTextReadConfig,
) -> PureSuccess<AllKeywords> {
    let mut deferred = PureErrorBuf::default();
    let textlen = xs.len();

    // Record delim positions
    let delim_positions: Vec<_> = xs
        .iter()
        .enumerate()
        // TODO could check for non-unicode characters here, which might come
        // in handy later
        .filter_map(|(i, c)| if *c == delim { Some(i) } else { None })
        .collect();

    // bail if we only have two positions
    if delim_positions.len() <= 2 {
        return PureSuccess::from(kws);
    }

    // Reduce position list to 'boundary list' which will be tuples of position
    // of a given delim and length until next delim.
    let mut raw_boundaries = delim_positions
        .iter()
        .tuple_windows()
        .map(|(a, b)| (*a, b - a));

    struct WordBound {
        start: usize,
        length: usize,
        has_delim: bool,
    }

    // Compute word boundaries depending on if we want to "escape" delims or
    // not. Technically all versions of the standard allow double delimiters to
    // be used in a word to represented a single delimiter. However, this means
    // we also can't have blank values. Many FCS files unfortunately use blank
    // values, so we need to be able to toggle this behavior.
    let boundaries = if conf.allow_double_delim {
        raw_boundaries
            .map(|(start, length)| WordBound {
                start,
                length,
                has_delim: false,
            })
            .collect()
    } else {
        // Remove "escaped" delimiters from position vector. Because we disallow
        // blank values and also disallow delimiters at the start/end of words,
        // this implies that we should only see delimiters by themselves or in a
        // consecutive sequence whose length is even. Any odd-length'ed runs will
        // be treated as one delimiter if config permits

        let mut filtered_boundaries = Vec::with_capacity(delim_positions.len());
        let mut consec_delims = 0;
        let mut has_delim = false;
        if let Some((mut prev_start, mut prev_len)) = raw_boundaries.next() {
            for (start, len) in raw_boundaries {
                if len == 1 {
                    consec_delims += 1;
                } else {
                    if consec_delims == 0 {
                        // No delimiters encountered, this is a new word, and
                        // the previous word can be added.
                        filtered_boundaries.push(WordBound {
                            start: prev_start,
                            length: prev_len,
                            has_delim,
                        });
                        prev_start = start;
                        prev_len = len;
                        has_delim = false;
                    } else if consec_delims % 2 == 0 {
                        // Previous consecutive delimiter sequence was even,
                        // treat as an escape. The current bound is part of the
                        // previous bound, so add the current length to the
                        // previous, including whatever length the delimiters
                        // have.
                        prev_len = prev_len + consec_delims + len;
                        has_delim = true;
                    } else {
                        // Previous consecutive delimiter sequence was odd,
                        // treat the previous non-delim bound as a word and
                        // the current bound as the next word. Push the previous
                        // bound as-is, which treats the delimiter sequence as
                        // one delimiter separating the two words
                        filtered_boundaries.push(WordBound {
                            start: prev_start,
                            length: prev_len,
                            has_delim,
                        });
                        prev_start = start;
                        prev_len = len;
                        has_delim = false;
                        deferred.push_warning("delim at word boundary".to_string());
                    }
                    consec_delims = 0;
                }
            }
            filtered_boundaries.push(WordBound {
                start: prev_start,
                length: prev_len,
                has_delim,
            });
        };

        // TODO these should be assertions I think? A user shouldn't ever see
        // these, and if they do I screwed something up

        // If all went well in the previous step, we should have the following:
        // 1. at least one boundary
        // 2. first entry coincides with start of TEXT
        // 3. last entry coincides with end of TEXT
        if let (Some(b0), Some(b1)) = (filtered_boundaries.first(), filtered_boundaries.last()) {
            if b0.start > 0 {
                let msg = format!("first key starts with a delim '{delim}'");
                deferred.push_error(msg);
            }
            if b1.start + b1.length < textlen - 1 {
                let msg = format!("final value ends with a delim '{delim}'");
                deferred.push_error(msg);
            }
        } else {
            return PureSuccess {
                data: kws,
                deferred,
            };
        }
        filtered_boundaries
    };

    // Check that the last char is also a delim, if not file probably sketchy
    // ASSUME this will not fail since we have at least one delim by definition
    if !delim_positions.last().unwrap() == xs.len() - 1 {
        deferred.push_msg_leveled(
            "Last char is not a delimiter".to_string(),
            conf.enforce_final_delim,
        );
    }

    let delim2 = [delim, delim];
    let delim1 = [delim];
    // ASSUME these won't fail as we checked the delimiter is an ASCII character
    let escape_from = str::from_utf8(&delim2).unwrap();
    let escape_to = str::from_utf8(&delim1).unwrap();

    struct WordBoundFinal {
        start: usize,
        end: usize,
        has_delim: bool,
    }

    let final_boundaries: Vec<_> = boundaries
        .into_iter()
        .map(|b| WordBoundFinal {
            start: b.start + 1,
            end: b.start + b.length,
            has_delim: b.has_delim,
        })
        .collect();

    // TODO this loop takes 10-15% total execution time when reading a file
    // from start to finish in standardized mode
    for chunk in final_boundaries.chunks(2) {
        if let [kb, vb] = chunk {
            // TODO lots of optimization opportunities here:
            // - I should know if the bytes at this point are valid, since I've
            //   already scanned through them once
            // - None of the standard keys should have non-ascii in them
            //   so that check is useless
            // - I can split the standard and non-standard kws into two hash
            //   tables and deal with them separately (since they don't need to
            //   mix except at the bounds of the program)
            // - If I split the tables then I don't need to store the '$' which
            //   lowers memory/hashing cost, esp for all those little PnX keys
            // - I don't need to store strings for standard keys, I just need to
            //   store ASCII characters
            // - For standard keys I can probably just iterate through and
            //   convert to uppercase myself without allocating a new string
            if let (k, Ok(v)) = (&xs[kb.start..kb.end], str::from_utf8(&xs[vb.start..vb.end])) {
                // test if keyword is ascii
                if !k.is_ascii() {
                    // TODO actually include keyword here
                    deferred.push_msg_leveled(
                        "keywords must be ASCII".to_string(),
                        conf.enforce_keyword_ascii,
                    )
                }
                // let kupper = k.to_uppercase();
                // if delimiters were escaped, replace them here
                if conf.allow_double_delim {
                    // Test for empty values if we don't allow delim escaping;
                    // anything empty will either drop or produce an error
                    // depending on user settings
                    if v.is_empty() {
                        // TODO tell the user that this key will be dropped
                        let msg = format!("key <FIXME> has a blank value");
                        deferred.push_msg_leveled(msg, conf.enforce_nonempty);
                    } else {
                        kws.insert(k, v.to_string());
                    }
                } else {
                    // TODO actually replace delims
                    // let krep = if kb.has_delim {
                    //     kupper.replace(escape_from, escape_to)
                    // } else {
                    //     kupper
                    // };
                    let vrep = if vb.has_delim {
                        v.replace(escape_from, escape_to)
                    } else {
                        v.to_string()
                    };
                    // TODO warn when key already exists
                    kws.insert(k, vrep);
                };
            } else {
                let msg = "invalid UTF-8 byte encountered when parsing TEXT".to_string();
                deferred.push_msg_leveled(msg, conf.error_on_invalid_utf8)
            }
        } else {
            deferred.push_msg_leveled("number of words is not even".to_string(), conf.enforce_even)
        }
    }

    PureSuccess {
        data: kws,
        deferred,
    }
}

fn repair_keywords(kws: &mut StdKeywords, conf: &RawTextReadConfig) {
    for (key, v) in kws.iter_mut() {
        let k = key.as_ref();
        // TODO generalized this and possibly put in a trait
        if k == FCSDate::std().as_ref() {
            if let Some(pattern) = &conf.date_pattern {
                if let Ok(d) = NaiveDate::parse_from_str(v, pattern.as_ref()) {
                    *v = format!("{}", FCSDate(d))
                }
            }
        }
    }
}

fn hash_raw_pairs(pairs: RawPairs, conf: &RawTextReadConfig) -> PureSuccess<RawKeywords> {
    let standard: HashMap<_, _> = HashMap::new();
    let mut res = PureSuccess::from(standard);
    // TODO filter keywords based on pattern somewhere here
    for (key, value) in pairs.into_iter() {
        match res.data.entry(key) {
            Entry::Occupied(e) => {
                let msg = format!("Skipping already-inserted key: {}", e.key());
                res.push_msg_leveled(msg, conf.enforce_unique);
            }
            Entry::Vacant(e) => {
                e.insert(value);
            }
        }
    }
    res
}

fn pad_zeros(s: &str) -> String {
    let len = s.len();
    let trimmed = s.trim_start();
    let newlen = trimmed.len();
    ("0").repeat(len - newlen) + trimmed
}

fn repair_offsets(mut kws: AllKeywords, conf: &RawTextReadConfig) -> AllKeywords {
    let mut go = |k| {
        if let Some(v) = kws.std.get_mut(&k) {
            *v = pad_zeros(v.as_str())
        }
    };
    if conf.repair_offset_spaces {
        go(Begindata::std());
        go(Enddata::std());
        go(Beginstext::std());
        go(Endstext::std());
        go(Beginanalysis::std());
        go(Endanalysis::std());
        go(Nextdata::std());
        // for (key, v) in pairs.iter_mut() {
        //     if key == BEGINDATA
        //         || key == ENDDATA
        //         || key == BEGINSTEXT
        //         || key == ENDSTEXT
        //         || key == BEGINANALYSIS
        //         || key == ENDANALYSIS
        //         || key == NEXTDATA
        //     {
        //         *v = pad_zeros(v.as_str())
        //     }
        // }
    }
    kws
}

// TODO use non-empty here (and everywhere else we return multiple errors)
fn lookup_req_segment(
    kws: &StdKeywords,
    bk: &StdKey,
    ek: &StdKey,
    corr: OffsetCorrection,
    id: SegmentId,
) -> Result<Segment, Vec<String>> {
    let x0 = get_req(kws, bk);
    let x1 = get_req(kws, ek);
    match (x0, x1) {
        (Ok(begin), Ok(end)) => Segment::try_new(begin, end, corr, id).map_err(|x| vec![x]),
        (a, b) => Err([a.err(), b.err()].into_iter().flatten().collect()),
    }
}

fn lookup_opt_segment(
    kws: &StdKeywords,
    bk: &StdKey,
    ek: &StdKey,
    corr: OffsetCorrection,
    id: SegmentId,
) -> Result<Option<Segment>, Vec<String>> {
    let x0 = get_opt(kws, bk);
    let x1 = get_opt(kws, ek);
    match (x0, x1) {
        (Ok(mb), Ok(me)) => {
            if let (Some(begin), Some(end)) = (mb, me) {
                Segment::try_new(begin, end, corr, id)
                    .map_err(|x| vec![x])
                    .map(Some)
            } else {
                Ok(None)
            }
        }
        (a, b) => Err([a.err(), b.err()].into_iter().flatten().collect()),
    }
}

// TODO unclear if these next two functions should throw errors or warnings
// on failure
fn lookup_data_offsets(
    kws: &StdKeywords,
    conf: &DataReadConfig,
    version: Version,
    default: &Segment,
) -> PureSuccess<Segment> {
    match version {
        Version::FCS2_0 => Ok(*default),
        _ => lookup_req_segment(
            kws,
            &Begindata::std(),
            &Enddata::std(),
            conf.data,
            SegmentId::Data,
        ),
    }
    .map_or_else(
        |es| {
            // TODO toggle this?
            let mut def = PureErrorBuf::from_many(es, PureErrorLevel::Warning);
            let msg =
                "could not use DATA offsets in TEXT, defaulting to HEADER offsets".to_string();
            def.push_warning(msg);
            PureSuccess {
                data: *default,
                deferred: def,
            }
        },
        PureSuccess::from,
    )
}

fn lookup_analysis_offsets(
    kws: &StdKeywords,
    conf: &DataReadConfig,
    version: Version,
    default: &Segment,
) -> PureSuccess<Segment> {
    let default_succ = |msgs| {
        // TODO toggle this?
        let mut def = PureErrorBuf::from_many(msgs, PureErrorLevel::Warning);
        let msg =
            "could not use ANALYSIS offsets in TEXT, defaulting to HEADER offsets".to_string();
        def.push_warning(msg);
        PureSuccess {
            data: *default,
            deferred: def,
        }
    };
    match version {
        Version::FCS2_0 => Ok(Some(*default)),
        Version::FCS3_0 | Version::FCS3_1 => lookup_req_segment(
            kws,
            &Beginanalysis::std(),
            &Endanalysis::std(),
            conf.analysis,
            SegmentId::Analysis,
        )
        .map(Some),
        Version::FCS3_2 => lookup_opt_segment(
            kws,
            &Beginanalysis::std(),
            &Endanalysis::std(),
            conf.analysis,
            SegmentId::Analysis,
        ),
    }
    .map_or_else(default_succ, |mab_seg| {
        mab_seg.map_or_else(|| default_succ(vec![]), PureSuccess::from)
    })
}

fn lookup_stext_offsets(
    kws: &StdKeywords,
    version: Version,
    conf: &RawTextReadConfig,
) -> PureMaybe<Segment> {
    // TODO add another msg explaining that the supp text won't be read if
    // offsets not found
    match version {
        Version::FCS2_0 => PureSuccess::from(None),
        Version::FCS3_0 | Version::FCS3_1 => {
            let res = lookup_req_segment(
                kws,
                &Beginstext::std(),
                &Endstext::std(),
                conf.stext,
                SegmentId::SupplementalText,
            );
            let level = if conf.enforce_stext {
                PureErrorLevel::Error
            } else {
                PureErrorLevel::Warning
            };
            PureMaybe::from_result_strs(res, level)
        }
        Version::FCS3_2 => lookup_opt_segment(
            kws,
            &Beginstext::std(),
            &Endstext::std(),
            conf.stext,
            SegmentId::SupplementalText,
        )
        .map_or_else(
            |es| PureSuccess {
                data: None,
                deferred: PureErrorBuf::from_many(es, PureErrorLevel::Warning),
            },
            PureSuccess::from,
        ),
    }
}

fn add_keywords(
    kws: &mut RawKeywords,
    pairs: RawPairs,
    conf: &RawTextReadConfig,
) -> PureSuccess<()> {
    let mut succ = PureSuccess::from(());
    for (k, v) in pairs.into_iter() {
        match kws.entry(k) {
            Entry::Occupied(e) => {
                let msg = format!(
                    "Skipping already-inserted key from supplemental TEXT: {}",
                    e.key()
                );
                succ.push_msg_leveled(msg, conf.enforce_unique);
            }
            Entry::Vacant(e) => {
                e.insert(v);
            }
        }
    }
    succ
}

fn lookup_nextdata(kws: &StdKeywords, enforce: bool) -> PureMaybe<u32> {
    if enforce {
        PureMaybe::from_result_1(get_req(kws, &Nextdata::std()), PureErrorLevel::Error)
    } else {
        PureMaybe::from_result_1(get_opt(kws, &Nextdata::std()), PureErrorLevel::Warning)
            .map(|x| x.flatten())
    }
}

#[derive(Default, Clone, Serialize)]
pub struct AllKeywords {
    pub std: StdKeywords,
    pub nonstd: NonStdKeywords,
}

impl AllKeywords {
    fn insert(&mut self, k: &[u8], v: String) -> Option<String> {
        // first char is '$'
        if k[0] == 36 {
            self.std.insert(StdKey(k[1..].to_vec()), v)
        } else {
            None
            // ASSUME all chars are ascii
            // self.nonstd
            //     .insert(NonStdKey::from_unchecked(str::from_utf8(k).unwrap()), v)
        }
    }
}

fn h_read_raw_text_from_header<R: Read + Seek>(
    h: &mut BufReader<R>,
    header: &Header,
    conf: &RawTextReadConfig,
) -> ImpureResult<RawTEXT> {
    let mut buf = vec![];
    header.text.read(h, &mut buf)?;

    let start = Instant::now();
    let res = verify_delim(&buf, conf).try_map(|delimiter| {
        let split_succ = split_raw_text(&buf, AllKeywords::default(), delimiter, conf).map(|kws| {
            repair_offsets(kws, conf)
            // hash_raw_pairs(pairs, conf)
        });
        let stext_succ = split_succ.try_map(|kws| {
            lookup_stext_offsets(&kws.std, header.version, conf).try_map(|s| {
                let succ = if let Some(seg) = s {
                    buf.clear();
                    seg.read(h, &mut buf)?;
                    split_raw_text(&buf, kws, delimiter, conf)
                    // .and_then(|pairs| add_keywords(&mut kws, pairs, conf))
                } else {
                    PureSuccess::from(kws)
                };
                Ok(succ.map(|k| (k, s)))
            })
        })?;
        Ok(stext_succ.and_then(|(mut kws, supp_text_seg)| {
            repair_keywords(&mut kws.std, conf);
            // TODO this will throw an error if not present, but we may not care
            // so toggle b/t error and warning
            let enforce_nextdata = true;
            lookup_nextdata(&kws.std, enforce_nextdata).map(|nextdata| RawTEXT {
                version: header.version,
                parse: ParseParameters {
                    prim_text: header.text,
                    supp_text: supp_text_seg,
                    data: header.data,
                    analysis: header.analysis,
                    nextdata,
                    delimiter,
                },
                keywords: kws,
            })
        }))
    });
    println!("std: {:?}", start.elapsed());
    res
}

fn split_remainder(xs: StdKeywords) -> (StdKeywords, StdKeywords) {
    xs.into_iter()
        .map(|(k, v)| {
            if k == Tot::std()
                || k == Begindata::std()
                || k == Enddata::std()
                || k == Beginanalysis::std()
                || k == Endanalysis::std()
            {
                Ok((k, v))
            } else {
                Err((k, v))
            }
        })
        .partition_result()
}

// fn comp_to_spillover(comp: Compensation, ns: &[Shortname]) -> Option<Spillover> {
//     // Matrix should be square, so if inverse fails that means that somehow it
//     // isn't full rank
//     comp.matrix.try_inverse().map(|matrix| Spillover {
//         measurements: ns.to_vec(),
//         matrix,
//     })
// }

// // TODO doesn't this need to be transposed also?
// fn spillover_to_comp(spillover: Spillover, ns: &[Shortname]) -> Option<Compensation> {
//     // Start by making a new square matrix for all measurements, since the older
//     // $COMP keyword couldn't specify measurements and thus covered all of them.
//     // Then assign the spillover matrix to the bigger full matrix, using the
//     // index of the measurement names. This will be a spillover matrix defined
//     // for all measurements. Anything absent from the original will have 0 in
//     // it's row/column except for the diagonal. Finally, invert this result to
//     // get the compensation matrix.
//     let n = ns.len();
//     let mut full_matrix = DMatrix::<f32>::identity(n, n);
//     // ASSUME spillover measurements are a subset of names supplied to function
//     let positions: Vec<_> = spillover
//         .measurements
//         .into_iter()
//         .enumerate()
//         .flat_map(|(i, m)| ns.iter().position(|x| *x == m).map(|x| (i, x)))
//         .collect();
//     for r in positions.iter() {
//         for c in positions.iter() {
//             full_matrix[(r.1, c.1)] = spillover.matrix[(r.0, c.0)]
//         }
//     }
//     // Matrix should be square, so if inverse fails that means that somehow it
//     // isn't full rank
//     full_matrix
//         .try_inverse()
//         .map(|matrix| Compensation { matrix })
// }
