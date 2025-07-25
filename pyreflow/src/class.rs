use fireflow_core::api::*;
use fireflow_core::config::*;
use fireflow_core::core::*;
use fireflow_core::error::*;
use fireflow_core::header::*;
use fireflow_core::segment::*;
use fireflow_core::text::byteord::ByteOrd2_0;
use fireflow_core::text::datetimes::ReversedDatetimes;
use fireflow_core::text::keywords::*;
use fireflow_core::text::named_vec::{
    Element, ElementIndexError, KeyLengthError, NamedVec, RawInput,
};
use fireflow_core::text::optional::*;
use fireflow_core::text::ranged_float::*;
use fireflow_core::text::scale::*;
use fireflow_core::text::timestamps::ReversedTimestamps;
use fireflow_core::text::unstainedcenters::UnstainedCenters;
use fireflow_core::validated::ascii_range::Chars;
use fireflow_core::validated::dataframe::*;
use fireflow_core::validated::datepattern::DatePattern;
use fireflow_core::validated::keys::*;
use fireflow_core::validated::shortname::*;

use super::layout::{self, PyLayout3_2, PyNonMixedLayout, PyOrderedLayout};
use super::macros::py_wrap;

use bigdecimal::BigDecimal;
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
use derive_more::{Display, From, Into};
use nonempty::NonEmpty;
use numpy::{PyArray2, PyReadonlyArray2, ToPyArray};
use polars::datatypes::NumericNative;
use polars::prelude::*;
use polars_arrow::array::PrimitiveArray;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyIndexError, PyValueError, PyWarning};
use pyo3::prelude::*;
use pyo3::types::{IntoPyDict, PyDict, PyFloat, PyString, PyTuple};
use pyo3::IntoPyObjectExt;
use pyo3_polars::{PyDataFrame, PySeries};
use std::collections::HashMap;
use std::convert::Infallible;
use std::ffi::CString;
use std::fmt;
use std::num::NonZeroU8;
use std::path;

#[pymodule]
fn pyreflow(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("PyreflowException", py.get_type::<PyreflowException>())?;
    m.add("PyreflowWarning", py.get_type::<PyreflowWarning>())?;

    m.add_class::<PyCoreTEXT2_0>()?;
    m.add_class::<PyCoreTEXT3_0>()?;
    m.add_class::<PyCoreTEXT3_1>()?;
    m.add_class::<PyCoreTEXT3_2>()?;

    m.add_class::<PyCoreDataset2_0>()?;
    m.add_class::<PyCoreDataset3_0>()?;
    m.add_class::<PyCoreDataset3_1>()?;
    m.add_class::<PyCoreDataset3_2>()?;

    m.add_class::<PyOptical2_0>()?;
    m.add_class::<PyOptical3_0>()?;
    m.add_class::<PyOptical3_1>()?;
    m.add_class::<PyOptical3_2>()?;

    m.add_class::<PyTemporal2_0>()?;
    m.add_class::<PyTemporal3_0>()?;
    m.add_class::<PyTemporal3_1>()?;
    m.add_class::<PyTemporal3_2>()?;

    m.add_class::<layout::PyAsciiFixedLayout>()?;
    m.add_class::<layout::PyAsciiDelimLayout>()?;
    m.add_class::<layout::PyOrderedUint08Layout>()?;
    m.add_class::<layout::PyOrderedUint16Layout>()?;
    m.add_class::<layout::PyOrderedUint24Layout>()?;
    m.add_class::<layout::PyOrderedUint32Layout>()?;
    m.add_class::<layout::PyOrderedUint40Layout>()?;
    m.add_class::<layout::PyOrderedUint48Layout>()?;
    m.add_class::<layout::PyOrderedUint56Layout>()?;
    m.add_class::<layout::PyOrderedUint64Layout>()?;
    m.add_class::<layout::PyOrderedF32Layout>()?;
    m.add_class::<layout::PyOrderedF64Layout>()?;
    m.add_class::<layout::PyEndianF32Layout>()?;
    m.add_class::<layout::PyEndianF64Layout>()?;
    m.add_class::<layout::PyEndianUintLayout>()?;
    m.add_class::<layout::PyMixedLayout>()?;

    m.add_function(wrap_pyfunction!(py_fcs_read_header, m)?)?;
    m.add_function(wrap_pyfunction!(py_fcs_read_raw_text, m)?)?;
    m.add_function(wrap_pyfunction!(py_fcs_read_std_text, m)?)?;
    m.add_function(wrap_pyfunction!(py_fcs_read_std_dataset, m)?)?;
    m.add_function(wrap_pyfunction!(py_fcs_read_raw_dataset, m)?)
}

#[allow(clippy::too_many_arguments)]
#[pyfunction]
#[pyo3(
    name = "fcs_read_header",
    signature = (
        p,
        version_override=None,
        prim_text_correction=(0,0),
        data_correction=(0,0),
        analysis_correction=(0,0),
        other_corrections=vec![],
        other_width=None,
        max_other=None,
        allow_negative=false,
        squish_offsets=false,
        truncate_offsets=false,
    )
)]
fn py_fcs_read_header(
    p: path::PathBuf,
    version_override: Option<PyVersion>,
    prim_text_correction: (i32, i32),
    data_correction: (i32, i32),
    analysis_correction: (i32, i32),
    other_corrections: Vec<(i32, i32)>,
    other_width: Option<PyChars>,
    max_other: Option<usize>,
    allow_negative: bool,
    squish_offsets: bool,
    truncate_offsets: bool,
) -> PyResult<PyHeader> {
    let conf = header_config(
        version_override,
        prim_text_correction,
        data_correction,
        analysis_correction,
        other_corrections,
        other_width,
        max_other,
        allow_negative,
        squish_offsets,
        truncate_offsets,
    );
    fcs_read_header(&p, &conf)
        .map_err(handle_failure_nowarn)
        .map(|x| x.inner().into())
}

#[allow(clippy::too_many_arguments)]
#[pyfunction]
#[pyo3(name = "fcs_read_raw_text")]
fn py_fcs_read_raw_text(
    p: path::PathBuf,

    version_override: Option<PyVersion>,
    prim_text_correction: (i32, i32),
    data_correction: (i32, i32),
    analysis_correction: (i32, i32),
    other_corrections: Vec<(i32, i32)>,
    other_width: Option<PyChars>,
    max_other: Option<usize>,
    allow_negative: bool,
    squish_offsets: bool,
    truncate_offsets: bool,

    text_data_correction: (i32, i32),
    text_analysis_correction: (i32, i32),
    supp_text_correction: (i32, i32),
    use_literal_delims: bool,
    allow_non_ascii_delim: bool,
    ignore_supp_text: bool,
    ignore_text_data_offsets: bool,
    ignore_text_analysis_offsets: bool,
    allow_header_text_offset_mismatch: bool,
    allow_missing_required_offsets: bool,
    allow_duplicated_stext: bool,
    allow_missing_final_delim: bool,
    allow_nonunique: bool,
    allow_odd: bool,
    allow_delim_at_boundary: bool,
    allow_empty: bool,
    allow_non_utf8: bool,
    allow_non_ascii_keywords: bool,
    allow_missing_stext: bool,
    allow_stext_own_delim: bool,
    allow_missing_nextdata: bool,
    trim_value_whitespace: bool,
    date_pattern: Option<PyDatePattern>,
    promote_to_standard: PyKeyPatterns,
    demote_from_standard: PyKeyPatterns,
    ignore_standard_keys: PyKeyPatterns,
    rename_standard_keys: PyKeyPairs,
    replace_standard_key_values: PyKeyValues,
    append_standard_keywords: PyKeyValues,
    warnings_are_errors: bool,
) -> PyResult<(PyVersion, PyStdKeywords, PyNonStdKeywords, PyParseData)> {
    let header = header_config(
        version_override,
        prim_text_correction,
        data_correction,
        analysis_correction,
        other_corrections,
        other_width,
        max_other,
        allow_negative,
        squish_offsets,
        truncate_offsets,
    );

    let conf = raw_config(
        header,
        text_data_correction,
        text_analysis_correction,
        supp_text_correction,
        use_literal_delims,
        allow_non_ascii_delim,
        ignore_supp_text,
        ignore_text_data_offsets,
        ignore_text_analysis_offsets,
        allow_header_text_offset_mismatch,
        allow_missing_required_offsets,
        allow_duplicated_stext,
        allow_missing_final_delim,
        allow_nonunique,
        allow_odd,
        allow_delim_at_boundary,
        allow_empty,
        allow_non_utf8,
        allow_non_ascii_keywords,
        allow_missing_stext,
        allow_stext_own_delim,
        allow_missing_nextdata,
        trim_value_whitespace,
        date_pattern,
        promote_to_standard,
        demote_from_standard,
        ignore_standard_keys,
        rename_standard_keys,
        replace_standard_key_values,
        append_standard_keywords,
        warnings_are_errors,
    );

    let raw: RawTEXTOutput =
        fcs_read_raw_text(&p, &conf).map_or_else(|e| Err(handle_failure(e)), handle_warnings)?;
    Ok((
        raw.version.into(),
        raw.keywords.std.into(),
        raw.keywords.nonstd.into(),
        raw.parse.into(),
    ))
}

#[allow(clippy::too_many_arguments)]
#[pyfunction]
#[pyo3(
    name = "fcs_read_std_text",
    signature = (
        p,

        version_override=None,
        prim_text_correction=(0,0),
        data_correction=(0,0),
        analysis_correction=(0,0),
        other_corrections=vec![],
        other_width=None,
        max_other=None,
        allow_negative=false,
        squish_offsets=false,
        truncate_offsets=false,

        supp_text_correction=(0,0),
        use_literal_delims=false,
        allow_non_ascii_delim=false,
        ignore_supp_text=false,
        ignore_text_data_offsets=false,
        ignore_text_analysis_offsets=false,
        allow_header_text_offset_mismatch=false,
        allow_missing_required_offsets=false,
        allow_duplicated_stext=false,
        allow_missing_final_delim=false,
        allow_nonunique=false,
        allow_odd=false,
        allow_delim_at_boundary=false,
        allow_empty=false,
        allow_non_utf8=false,
        allow_non_ascii_keywords=false,
        allow_missing_stext=false,
        allow_stext_own_delim=false,
        allow_missing_nextdata=false,
        trim_value_whitespace=false,
        date_pattern=None,
        promote_to_standard=PyKeyPatterns::default(),
        demote_from_standard=PyKeyPatterns::default(),
        ignore_standard_keys=PyKeyPatterns::default(),
        rename_standard_keys=PyKeyPairs::default(),
        replace_standard_key_values=PyKeyValues::default(),
        append_standard_keywords=PyKeyValues::default(),
        warnings_are_errors=false,

        disallow_deprecated=false,
        time_ensure=false,
        allow_pseudostandard=false,
        fix_log_scale_offsets=false,
        shortname_prefix=None,
        text_data_correction=(0,0),
        text_analysis_correction=(0,0),
        disallow_range_truncation=false,
        nonstandard_measurement_pattern=None,
        time_pattern=None,
        integer_widths_from_byteord=false,
        integer_byteord_override=PyByteOrd::default(),
    )
)]
fn py_fcs_read_std_text(
    p: path::PathBuf,

    version_override: Option<PyVersion>,
    prim_text_correction: (i32, i32),
    data_correction: (i32, i32),
    analysis_correction: (i32, i32),
    other_corrections: Vec<(i32, i32)>,
    other_width: Option<PyChars>,
    max_other: Option<usize>,
    allow_negative: bool,
    squish_offsets: bool,
    truncate_offsets: bool,

    supp_text_correction: (i32, i32),
    use_literal_delims: bool,
    allow_non_ascii_delim: bool,
    ignore_supp_text: bool,
    ignore_text_data_offsets: bool,
    ignore_text_analysis_offsets: bool,
    allow_header_text_offset_mismatch: bool,
    allow_missing_required_offsets: bool,
    allow_duplicated_stext: bool,
    allow_missing_final_delim: bool,
    allow_nonunique: bool,
    allow_odd: bool,
    allow_delim_at_boundary: bool,
    allow_empty: bool,
    allow_non_utf8: bool,
    allow_non_ascii_keywords: bool,
    allow_missing_stext: bool,
    allow_stext_own_delim: bool,
    allow_missing_nextdata: bool,
    trim_value_whitespace: bool,
    date_pattern: Option<PyDatePattern>,
    promote_to_standard: PyKeyPatterns,
    demote_from_standard: PyKeyPatterns,
    ignore_standard_keys: PyKeyPatterns,
    rename_standard_keys: PyKeyPairs,
    replace_standard_key_values: PyKeyValues,
    append_standard_keywords: PyKeyValues,
    warnings_are_errors: bool,

    disallow_deprecated: bool,
    time_ensure: bool,
    allow_pseudostandard: bool,
    fix_log_scale_offsets: bool,
    shortname_prefix: Option<PyShortnamePrefix>,
    text_data_correction: (i32, i32),
    text_analysis_correction: (i32, i32),
    disallow_range_truncation: bool,
    nonstandard_measurement_pattern: Option<PyNonStdMeasPattern>,
    time_pattern: Option<PyTimePattern>,
    integer_widths_from_byteord: bool,
    integer_byteord_override: PyByteOrd,
) -> PyResult<(PyAnyCoreTEXT, PyParseData, PyStdKeywords)> {
    let header = header_config(
        version_override,
        prim_text_correction,
        data_correction,
        analysis_correction,
        other_corrections,
        other_width,
        max_other,
        allow_negative,
        squish_offsets,
        truncate_offsets,
    );

    let raw = raw_config(
        header,
        text_data_correction,
        text_analysis_correction,
        supp_text_correction,
        use_literal_delims,
        allow_non_ascii_delim,
        ignore_supp_text,
        ignore_text_data_offsets,
        ignore_text_analysis_offsets,
        allow_header_text_offset_mismatch,
        allow_missing_required_offsets,
        allow_duplicated_stext,
        allow_missing_final_delim,
        allow_nonunique,
        allow_odd,
        allow_delim_at_boundary,
        allow_empty,
        allow_non_utf8,
        allow_non_ascii_keywords,
        allow_missing_stext,
        allow_stext_own_delim,
        allow_missing_nextdata,
        trim_value_whitespace,
        date_pattern,
        promote_to_standard,
        demote_from_standard,
        ignore_standard_keys,
        rename_standard_keys,
        replace_standard_key_values,
        append_standard_keywords,
        warnings_are_errors,
    );

    let conf = std_config(
        raw,
        disallow_deprecated,
        time_ensure,
        allow_pseudostandard,
        fix_log_scale_offsets,
        shortname_prefix,
        disallow_range_truncation,
        nonstandard_measurement_pattern,
        time_pattern,
        integer_widths_from_byteord,
        integer_byteord_override,
    );

    let out: StdTEXTOutput =
        fcs_read_std_text(&p, &conf).map_or_else(|e| Err(handle_failure(e)), handle_warnings)?;

    Ok((
        out.standardized.clone().into(),
        out.parse.into(),
        out.pseudostandard.clone().into(),
    ))
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
#[pyfunction]
#[pyo3(
    name = "fcs_read_raw_dataset",
    signature = (
        p,

        version_override=None,
        prim_text_correction=(0,0),
        data_correction=(0,0),
        analysis_correction=(0,0),
        other_corrections=vec![],
        other_width=None,
        max_other=None,
        allow_negative=false,
        squish_offsets=false,
        truncate_offsets=false,

        supp_text_correction=(0,0),
        use_literal_delims=false,
        allow_non_ascii_delim=false,
        ignore_supp_text=false,
        ignore_text_data_offsets=false,
        ignore_text_analysis_offsets=false,
        allow_header_text_offset_mismatch=false,
        allow_missing_required_offsets=false,
        allow_duplicated_stext=false,
        allow_missing_final_delim=false,
        allow_nonunique=false,
        allow_odd=false,
        allow_delim_at_boundary=false,
        allow_empty=false,
        allow_non_utf8=false,
        allow_non_ascii_keywords=false,
        allow_missing_stext=false,
        allow_stext_own_delim=false,
        allow_missing_nextdata=false,
        trim_value_whitespace=false,
        date_pattern=None,
        promote_to_standard=PyKeyPatterns::default(),
        demote_from_standard=PyKeyPatterns::default(),
        ignore_standard_keys=PyKeyPatterns::default(),
        rename_standard_keys=PyKeyPairs::default(),
        replace_standard_key_values=PyKeyValues::default(),
        append_standard_keywords=PyKeyValues::default(),
        warnings_are_errors=false,

        disallow_deprecated=false,
        time_ensure=false,
        allow_pseudostandard=false,
        fix_log_scale_offsets=false,
        shortname_prefix=None,
        text_data_correction=(0,0),
        text_analysis_correction=(0,0),
        disallow_range_truncation=false,
        nonstandard_measurement_pattern=None,
        time_pattern=None,
        integer_widths_from_byteord=false,
        integer_byteord_override=PyByteOrd::default(),

        allow_uneven_event_width=false,
        allow_tot_mismatch=false,
        allow_data_par_mismatch=false,
    )
)]
fn py_fcs_read_raw_dataset(
    p: path::PathBuf,

    version_override: Option<PyVersion>,
    prim_text_correction: (i32, i32),
    data_correction: (i32, i32),
    analysis_correction: (i32, i32),
    other_corrections: Vec<(i32, i32)>,
    other_width: Option<PyChars>,
    max_other: Option<usize>,
    allow_negative: bool,
    squish_offsets: bool,
    truncate_offsets: bool,

    supp_text_correction: (i32, i32),
    use_literal_delims: bool,
    allow_non_ascii_delim: bool,
    ignore_supp_text: bool,
    ignore_text_data_offsets: bool,
    ignore_text_analysis_offsets: bool,
    allow_header_text_offset_mismatch: bool,
    allow_missing_required_offsets: bool,
    allow_duplicated_stext: bool,
    allow_missing_final_delim: bool,
    allow_nonunique: bool,
    allow_odd: bool,
    allow_delim_at_boundary: bool,
    allow_empty: bool,
    allow_non_utf8: bool,
    allow_non_ascii_keywords: bool,
    allow_missing_stext: bool,
    allow_stext_own_delim: bool,
    allow_missing_nextdata: bool,
    trim_value_whitespace: bool,
    date_pattern: Option<PyDatePattern>,
    promote_to_standard: PyKeyPatterns,
    demote_from_standard: PyKeyPatterns,
    ignore_standard_keys: PyKeyPatterns,
    rename_standard_keys: PyKeyPairs,
    replace_standard_key_values: PyKeyValues,
    append_standard_keywords: PyKeyValues,
    warnings_are_errors: bool,

    disallow_deprecated: bool,
    time_ensure: bool,
    allow_pseudostandard: bool,
    fix_log_scale_offsets: bool,
    shortname_prefix: Option<PyShortnamePrefix>,
    text_data_correction: (i32, i32),
    text_analysis_correction: (i32, i32),
    disallow_range_truncation: bool,
    nonstandard_measurement_pattern: Option<PyNonStdMeasPattern>,
    time_pattern: Option<PyTimePattern>,
    integer_widths_from_byteord: bool,
    integer_byteord_override: PyByteOrd,

    allow_uneven_event_width: bool,
    allow_tot_mismatch: bool,
    allow_data_par_mismatch: bool,
) -> PyResult<(
    PyVersion,
    PyStdKeywords,
    PyNonStdKeywords,
    PyParseData,
    PyDataFrame,
    Vec<u8>,
    Vec<Vec<u8>>,
)> {
    let header = header_config(
        version_override,
        prim_text_correction,
        data_correction,
        analysis_correction,
        other_corrections,
        other_width,
        max_other,
        allow_negative,
        squish_offsets,
        truncate_offsets,
    );

    let raw = raw_config(
        header,
        text_data_correction,
        text_analysis_correction,
        supp_text_correction,
        use_literal_delims,
        allow_non_ascii_delim,
        ignore_supp_text,
        ignore_text_data_offsets,
        ignore_text_analysis_offsets,
        allow_header_text_offset_mismatch,
        allow_missing_required_offsets,
        allow_duplicated_stext,
        allow_missing_final_delim,
        allow_nonunique,
        allow_odd,
        allow_delim_at_boundary,
        allow_empty,
        allow_non_utf8,
        allow_non_ascii_keywords,
        allow_missing_stext,
        allow_stext_own_delim,
        allow_missing_nextdata,
        trim_value_whitespace,
        date_pattern,
        promote_to_standard,
        demote_from_standard,
        ignore_standard_keys,
        rename_standard_keys,
        replace_standard_key_values,
        append_standard_keywords,
        warnings_are_errors,
    );

    let standard = std_config(
        raw,
        disallow_deprecated,
        time_ensure,
        allow_pseudostandard,
        fix_log_scale_offsets,
        shortname_prefix,
        disallow_range_truncation,
        nonstandard_measurement_pattern,
        time_pattern,
        integer_widths_from_byteord,
        integer_byteord_override,
    );

    let conf = data_config(
        standard,
        allow_uneven_event_width,
        allow_tot_mismatch,
        allow_data_par_mismatch,
    );

    let out: RawDatasetOutput =
        fcs_read_raw_dataset(&p, &conf).map_or_else(|e| Err(handle_failure(e)), handle_warnings)?;

    Ok((
        out.text.version.into(),
        out.text.keywords.std.into(),
        out.text.keywords.nonstd.into(),
        out.text.parse.into(),
        PyFCSDataFrame::from(out.dataset.data).into(),
        out.dataset.analysis.0,
        out.dataset.others.0.into_iter().map(|x| x.0).collect(),
    ))
}

#[allow(clippy::too_many_arguments)]
#[pyfunction]
#[pyo3(
    name = "fcs_read_std_dataset",
    signature = (
        p,

        version_override=None,
        prim_text_correction=(0,0),
        data_correction=(0,0),
        analysis_correction=(0,0),
        other_corrections=vec![],
        other_width=None,
        max_other=None,
        allow_negative=false,
        squish_offsets=false,
        truncate_offsets=false,

        supp_text_correction=(0,0),
        use_literal_delims=false,
        allow_non_ascii_delim=false,
        ignore_supp_text=false,
        ignore_text_data_offsets=false,
        ignore_text_analysis_offsets=false,
        allow_header_text_offset_mismatch=false,
        allow_missing_required_offsets=false,
        allow_duplicated_stext=false,
        allow_missing_final_delim=false,
        allow_nonunique=false,
        allow_odd=false,
        allow_delim_at_boundary=false,
        allow_empty=false,
        allow_non_utf8=false,
        allow_non_ascii_keywords=false,
        allow_missing_stext=false,
        allow_stext_own_delim=false,
        allow_missing_nextdata=false,
        trim_value_whitespace=false,
        date_pattern=None,
        promote_to_standard=PyKeyPatterns::default(),
        demote_from_standard=PyKeyPatterns::default(),
        ignore_standard_keys=PyKeyPatterns::default(),
        rename_standard_keys=PyKeyPairs::default(),
        replace_standard_key_values=PyKeyValues::default(),
        append_standard_keywords=PyKeyValues::default(),
        warnings_are_errors=false,

        disallow_deprecated=false,
        time_ensure=false,
        allow_pseudostandard=false,
        fix_log_scale_offsets=false,
        shortname_prefix=None,
        text_data_correction=(0,0),
        text_analysis_correction=(0,0),
        disallow_range_truncation=false,
        nonstandard_measurement_pattern=None,
        time_pattern=None,
        integer_widths_from_byteord=false,
        integer_byteord_override=PyByteOrd::default(),

        allow_uneven_event_width=false,
        allow_tot_mismatch=false,
        allow_data_par_mismatch=false,
    )
)]
fn py_fcs_read_std_dataset(
    p: path::PathBuf,

    version_override: Option<PyVersion>,
    prim_text_correction: (i32, i32),
    data_correction: (i32, i32),
    analysis_correction: (i32, i32),
    other_corrections: Vec<(i32, i32)>,
    other_width: Option<PyChars>,
    max_other: Option<usize>,
    allow_negative: bool,
    squish_offsets: bool,
    truncate_offsets: bool,

    supp_text_correction: (i32, i32),
    use_literal_delims: bool,
    allow_non_ascii_delim: bool,
    ignore_supp_text: bool,
    ignore_text_data_offsets: bool,
    ignore_text_analysis_offsets: bool,
    allow_header_text_offset_mismatch: bool,
    allow_missing_required_offsets: bool,
    allow_duplicated_stext: bool,
    allow_missing_final_delim: bool,
    allow_nonunique: bool,
    allow_odd: bool,
    allow_delim_at_boundary: bool,
    allow_empty: bool,
    allow_non_utf8: bool,
    allow_non_ascii_keywords: bool,
    allow_missing_stext: bool,
    allow_stext_own_delim: bool,
    allow_missing_nextdata: bool,
    trim_value_whitespace: bool,
    date_pattern: Option<PyDatePattern>,
    promote_to_standard: PyKeyPatterns,
    demote_from_standard: PyKeyPatterns,
    ignore_standard_keys: PyKeyPatterns,
    rename_standard_keys: PyKeyPairs,
    replace_standard_key_values: PyKeyValues,
    append_standard_keywords: PyKeyValues,
    warnings_are_errors: bool,

    disallow_deprecated: bool,
    time_ensure: bool,
    allow_pseudostandard: bool,
    fix_log_scale_offsets: bool,
    shortname_prefix: Option<PyShortnamePrefix>,
    text_data_correction: (i32, i32),
    text_analysis_correction: (i32, i32),
    disallow_range_truncation: bool,
    nonstandard_measurement_pattern: Option<PyNonStdMeasPattern>,
    time_pattern: Option<PyTimePattern>,
    integer_widths_from_byteord: bool,
    integer_byteord_override: PyByteOrd,

    allow_uneven_event_width: bool,
    allow_tot_mismatch: bool,
    allow_data_par_mismatch: bool,
) -> PyResult<(PyAnyCoreDataset, PyParseData, PyStdKeywords)> {
    let header = header_config(
        version_override,
        prim_text_correction,
        data_correction,
        analysis_correction,
        other_corrections,
        other_width,
        max_other,
        allow_negative,
        squish_offsets,
        truncate_offsets,
    );

    let raw = raw_config(
        header,
        text_data_correction,
        text_analysis_correction,
        supp_text_correction,
        use_literal_delims,
        allow_non_ascii_delim,
        ignore_supp_text,
        ignore_text_data_offsets,
        ignore_text_analysis_offsets,
        allow_header_text_offset_mismatch,
        allow_missing_required_offsets,
        allow_duplicated_stext,
        allow_missing_final_delim,
        allow_nonunique,
        allow_odd,
        allow_delim_at_boundary,
        allow_empty,
        allow_non_utf8,
        allow_non_ascii_keywords,
        allow_missing_stext,
        allow_stext_own_delim,
        allow_missing_nextdata,
        trim_value_whitespace,
        date_pattern,
        promote_to_standard,
        demote_from_standard,
        ignore_standard_keys,
        rename_standard_keys,
        replace_standard_key_values,
        append_standard_keywords,
        warnings_are_errors,
    );

    let standard = std_config(
        raw,
        disallow_deprecated,
        time_ensure,
        allow_pseudostandard,
        fix_log_scale_offsets,
        shortname_prefix,
        disallow_range_truncation,
        nonstandard_measurement_pattern,
        time_pattern,
        integer_widths_from_byteord,
        integer_byteord_override,
    );

    let conf = data_config(
        standard,
        allow_uneven_event_width,
        allow_tot_mismatch,
        allow_data_par_mismatch,
    );

    let out: StdDatasetOutput =
        fcs_read_std_dataset(&p, &conf).map_or_else(|e| Err(handle_failure(e)), handle_warnings)?;

    Ok((
        out.dataset.standardized.core.clone().into(),
        out.parse.into(),
        out.dataset.pseudostandard.clone().into(),
    ))
}

#[allow(clippy::too_many_arguments)]
fn header_config(
    version_override: Option<PyVersion>,
    prim_text_correction: (i32, i32),
    data_correction: (i32, i32),
    analysis_correction: (i32, i32),
    other_corrections: Vec<(i32, i32)>,
    other_width: Option<PyChars>,
    max_other: Option<usize>,
    allow_negative: bool,
    squish_offsets: bool,
    truncate_offsets: bool,
) -> HeaderConfig {
    let os = other_corrections.into_iter().map(|x| x.into()).collect();
    HeaderConfig {
        version_override: version_override.map(|x| x.0),
        text_correction: OffsetCorrection::from(prim_text_correction),
        data_correction: OffsetCorrection::from(data_correction),
        analysis_correction: OffsetCorrection::from(analysis_correction),
        other_corrections: os,
        other_width: other_width.map(|x| x.0.into()).unwrap_or_default(),
        max_other,
        allow_negative,
        squish_offsets,
        truncate_offsets,
    }
}

#[allow(clippy::too_many_arguments)]
fn raw_config(
    header: HeaderConfig,
    text_data_correction: (i32, i32),
    text_analysis_correction: (i32, i32),
    supp_text_correction: (i32, i32),
    use_literal_delims: bool,
    allow_non_ascii_delim: bool,
    ignore_supp_text: bool,
    ignore_text_data_offsets: bool,
    ignore_text_analysis_offsets: bool,
    allow_header_text_offset_mismatch: bool,
    allow_missing_required_offsets: bool,
    allow_duplicated_stext: bool,
    allow_missing_final_delim: bool,
    allow_nonunique: bool,
    allow_odd: bool,
    allow_delim_at_boundary: bool,
    allow_empty: bool,
    allow_non_utf8: bool,
    allow_non_ascii_keywords: bool,
    allow_missing_stext: bool,
    allow_stext_own_delim: bool,
    allow_missing_nextdata: bool,
    trim_value_whitespace: bool,
    date_pattern: Option<PyDatePattern>,
    promote_to_standard: PyKeyPatterns,
    demote_from_standard: PyKeyPatterns,
    ignore_standard_keys: PyKeyPatterns,
    rename_standard_keys: PyKeyPairs,
    replace_standard_key_values: PyKeyValues,
    append_standard_keywords: PyKeyValues,
    warnings_are_errors: bool,
) -> RawTextReadConfig {
    RawTextReadConfig {
        header,
        data: OffsetCorrection::from(text_data_correction),
        analysis: OffsetCorrection::from(text_analysis_correction),
        supp_text_correction: OffsetCorrection::from(supp_text_correction),
        use_literal_delims,
        ignore_supp_text,
        ignore_text_data_offsets,
        ignore_text_analysis_offsets,
        allow_header_text_offset_mismatch,
        allow_missing_required_offsets,
        allow_duplicated_stext,
        allow_non_ascii_delim,
        allow_missing_final_delim,
        allow_nonunique,
        allow_odd,
        allow_delim_at_boundary,
        allow_empty,
        allow_non_utf8,
        allow_non_ascii_keywords,
        allow_missing_stext,
        allow_stext_own_delim,
        allow_missing_nextdata,
        trim_value_whitespace,
        date_pattern: date_pattern.map(|x| x.0),
        promote_to_standard: promote_to_standard.0,
        demote_from_standard: demote_from_standard.0,
        ignore_standard_keys: ignore_standard_keys.0,
        rename_standard_keys: rename_standard_keys.0,
        replace_standard_key_values: replace_standard_key_values.0,
        append_standard_keywords: append_standard_keywords.0,
        warnings_are_errors,
    }
}

#[allow(clippy::too_many_arguments)]
fn std_config(
    raw: RawTextReadConfig,
    disallow_deprecated: bool,
    time_ensure: bool,
    allow_pseudostandard: bool,
    fix_log_scale_offsets: bool,
    shortname_prefix: Option<PyShortnamePrefix>,
    disallow_range_truncation: bool,
    nonstandard_measurement_pattern: Option<PyNonStdMeasPattern>,
    time_pattern: Option<PyTimePattern>,
    integer_widths_from_byteord: bool,
    integer_byteord_override: PyByteOrd,
) -> StdTextReadConfig {
    StdTextReadConfig {
        raw,
        shortname_prefix: shortname_prefix.map(|x| x.0).unwrap_or_default(),
        time: TimeConfig {
            pattern: time_pattern.map(|x| x.0),
            allow_missing: time_ensure,
            // allow_nonlinear_scale: time_ensure_linear,
            // allow_nontime_keywords: time_ensure_nogain,
        },
        allow_pseudostandard,
        fix_log_scale_offsets,
        disallow_deprecated,
        disallow_range_truncation,
        nonstandard_measurement_pattern: nonstandard_measurement_pattern.map(|x| x.0),
        integer_widths_from_byteord,
        integer_byteord_override: integer_byteord_override.0,
    }
}

fn data_config(
    standard: StdTextReadConfig,
    allow_uneven_event_width: bool,
    allow_tot_mismatch: bool,
    allow_data_par_mismatch: bool,
) -> DataReadConfig {
    DataReadConfig {
        standard,
        reader: ReaderConfig {
            allow_uneven_event_width,
            allow_tot_mismatch,
            allow_data_par_mismatch,
        },
    }
}

// core* objects
py_wrap!(PyCoreTEXT2_0, CoreTEXT2_0, "CoreTEXT2_0");
py_wrap!(PyCoreTEXT3_0, CoreTEXT3_0, "CoreTEXT3_0");
py_wrap!(PyCoreTEXT3_1, CoreTEXT3_1, "CoreTEXT3_1");
py_wrap!(PyCoreTEXT3_2, CoreTEXT3_2, "CoreTEXT3_2");

py_wrap!(PyCoreDataset2_0, CoreDataset2_0, "CoreDataset2_0");
py_wrap!(PyCoreDataset3_0, CoreDataset3_0, "CoreDataset3_0");
py_wrap!(PyCoreDataset3_1, CoreDataset3_1, "CoreDataset3_1");
py_wrap!(PyCoreDataset3_2, CoreDataset3_2, "CoreDataset3_2");

#[derive(IntoPyObject, From)]
enum PyAnyCoreTEXT {
    #[from(CoreTEXT2_0)]
    FCS2_0(PyCoreTEXT2_0),
    #[from(CoreTEXT3_0)]
    FCS3_0(PyCoreTEXT3_0),
    #[from(CoreTEXT3_1)]
    FCS3_1(PyCoreTEXT3_1),
    #[from(CoreTEXT3_2)]
    FCS3_2(PyCoreTEXT3_2),
}

impl From<AnyCoreTEXT> for PyAnyCoreTEXT {
    fn from(value: AnyCoreTEXT) -> PyAnyCoreTEXT {
        match value {
            AnyCoreTEXT::FCS2_0(x) => (*x).into(),
            AnyCoreTEXT::FCS3_0(x) => (*x).into(),
            AnyCoreTEXT::FCS3_1(x) => (*x).into(),
            AnyCoreTEXT::FCS3_2(x) => (*x).into(),
        }
    }
}

#[derive(IntoPyObject, From)]
enum PyAnyCoreDataset {
    #[from(CoreDataset2_0)]
    FCS2_0(PyCoreDataset2_0),
    #[from(CoreDataset3_0)]
    FCS3_0(PyCoreDataset3_0),
    #[from(CoreDataset3_1)]
    FCS3_1(PyCoreDataset3_1),
    #[from(CoreDataset3_2)]
    FCS3_2(PyCoreDataset3_2),
}

impl From<AnyCoreDataset> for PyAnyCoreDataset {
    fn from(value: AnyCoreDataset) -> PyAnyCoreDataset {
        match value {
            AnyCoreDataset::FCS2_0(x) => (*x).into(),
            AnyCoreDataset::FCS3_0(x) => (*x).into(),
            AnyCoreDataset::FCS3_1(x) => (*x).into(),
            AnyCoreDataset::FCS3_2(x) => (*x).into(),
        }
    }
}

py_wrap!(PyOptical2_0, Optical2_0, "Optical2_0");
py_wrap!(PyOptical3_0, Optical3_0, "Optical3_0");
py_wrap!(PyOptical3_1, Optical3_1, "Optical3_1");
py_wrap!(PyOptical3_2, Optical3_2, "Optical3_2");

py_wrap!(PyTemporal2_0, Temporal2_0, "Temporal2_0");
py_wrap!(PyTemporal3_0, Temporal3_0, "Temporal3_0");
py_wrap!(PyTemporal3_1, Temporal3_1, "Temporal3_1");
py_wrap!(PyTemporal3_2, Temporal3_2, "Temporal3_2");

macro_rules! get_set_metaroot_opt {
    ($get:ident, $set:ident, $inner:ident, $outer:ident, $($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn $get(&self) -> Option<$outer> {
                    self.0.get_metaroot_opt::<$inner>().map(|x| x.clone().into())
                }

                #[setter]
                fn $set(&mut self, s: Option<$outer>) {
                    self.0.set_metaroot::<Option<$inner>>(s.map(|x| x.into()))
                }
            }
        )*
    };
}

macro_rules! get_set_all_meas {
    ($get:ident, $set:ident, $outer:ident, $inner:ident, $($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn $get(&self) -> Vec<(usize, Option<$outer>)> {
                    self.0.get_meas_opt::<$inner>()
                        .map(|(i, x)| (
                            i.into(),
                            x.map(|y| y.clone().into())
                        ))
                        .collect()
                }

                #[setter]
                fn $set(&mut self, xs: Vec<Option<$outer>>) -> Result<(), PyKeyLengthError> {
                    let ys = xs.into_iter().map(|x| x.map($inner::from)).collect();
                    self.0.set_meas(ys)?;
                    Ok(())
                }
            }
        )*
    };
}

macro_rules! get_set_all_optical {
    ($get:ident, $set:ident, $outer:ident, $inner:ident, $($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn $get(&self) -> Vec<(usize, Option<$outer>)> {
                    self.0.get_optical_opt::<$inner>()
                        .map(|(i, x)| (
                            i.into(),
                            x.map(|y| y.clone().into())
                        ))
                        .collect()
                }

                #[setter]
                fn $set(&mut self, xs: Vec<Option<$outer>>) -> Result<(), PyKeyLengthError> {
                    let ys = xs.into_iter().map(|x| x.map($inner::from)).collect();
                    self.0.set_optical(ys)?;
                    Ok(())
                }
            }
        )*
    };
}

macro_rules! convert_methods {
    ($pytype:ident, $([$fn:ident, $to:ident]),+) => {
        #[pymethods]
        impl $pytype {
            $(
                fn $fn(&self, lossless: bool) -> PyResult<$to> {
                    let new = self.0.clone().try_convert(lossless);
                    new.py_def_terminate(ConvertFailure).map(|x| x.into())
                }
            )*
        }
    };
}

convert_methods!(
    PyCoreTEXT2_0,
    [version_3_0, PyCoreTEXT3_0],
    [version_3_1, PyCoreTEXT3_1],
    [version_3_2, PyCoreTEXT3_2]
);

convert_methods!(
    PyCoreTEXT3_0,
    [version_2_0, PyCoreTEXT2_0],
    [version_3_1, PyCoreTEXT3_1],
    [version_3_2, PyCoreTEXT3_2]
);

convert_methods!(
    PyCoreTEXT3_1,
    [version_2_0, PyCoreTEXT2_0],
    [version_3_0, PyCoreTEXT3_0],
    [version_3_2, PyCoreTEXT3_2]
);

convert_methods!(
    PyCoreTEXT3_2,
    [version_2_0, PyCoreTEXT2_0],
    [version_3_0, PyCoreTEXT3_0],
    [version_3_1, PyCoreTEXT3_1]
);

convert_methods!(
    PyCoreDataset2_0,
    [version_3_0, PyCoreDataset3_0],
    [version_3_1, PyCoreDataset3_1],
    [version_3_2, PyCoreDataset3_2]
);

convert_methods!(
    PyCoreDataset3_0,
    [version_2_0, PyCoreDataset2_0],
    [version_3_1, PyCoreDataset3_1],
    [version_3_2, PyCoreDataset3_2]
);

convert_methods!(
    PyCoreDataset3_1,
    [version_2_0, PyCoreDataset2_0],
    [version_3_0, PyCoreDataset3_0],
    [version_3_2, PyCoreDataset3_2]
);

convert_methods!(
    PyCoreDataset3_2,
    [version_2_0, PyCoreDataset2_0],
    [version_3_0, PyCoreDataset3_0],
    [version_3_1, PyCoreDataset3_1]
);

#[pymethods]
impl PyCoreTEXT2_0 {
    #[new]
    fn new(mode: PyMode) -> PyResult<Self> {
        Ok(CoreTEXT2_0::new(mode.into()).into())
    }
}

#[pymethods]
impl PyCoreTEXT3_0 {
    #[new]
    fn new(mode: PyMode) -> PyResult<Self> {
        Ok(CoreTEXT3_0::new(mode.into()).into())
    }

    #[getter]
    fn get_unicode(&self) -> Option<PyUnicode> {
        self.0
            .metaroot
            .specific
            .unicode
            .as_ref_opt()
            .map(|x| x.clone().into())
    }

    #[setter]
    fn set_unicode(&mut self, x: Option<PyUnicode>) {
        self.0.metaroot.specific.unicode = x.map(|y| y.into()).into();
    }
}

#[pymethods]
impl PyCoreTEXT3_1 {
    #[new]
    fn new(mode: PyMode) -> Self {
        CoreTEXT3_1::new(mode.into()).into()
    }
}

#[pymethods]
impl PyCoreTEXT3_2 {
    #[new]
    fn new(cyt: String) -> Self {
        CoreTEXT3_2::new(cyt).into()
    }

    #[getter]
    fn get_begindatetime(&self) -> Option<DateTime<FixedOffset>> {
        self.0.get_begindatetime()
    }

    #[setter]
    fn set_begindatetime(
        &mut self,
        x: Option<DateTime<FixedOffset>>,
    ) -> Result<(), PyReversedDatetimes> {
        self.0.set_begindatetime(x)?;
        Ok(())
    }

    #[getter]
    fn get_enddatetime(&self) -> Option<DateTime<FixedOffset>> {
        self.0.get_enddatetime()
    }

    #[setter]
    fn set_enddatetime(
        &mut self,
        x: Option<DateTime<FixedOffset>>,
    ) -> Result<(), PyReversedDatetimes> {
        self.0.set_enddatetime(x)?;
        Ok(())
    }

    #[getter]
    fn get_cyt(&self) -> String {
        self.0.metaroot.specific.cyt.0.clone()
    }

    #[setter]
    fn set_cyt(&mut self, x: String) {
        self.0.metaroot.specific.cyt = x.into()
    }

    #[getter]
    fn get_unstainedinfo(&self) -> Option<String> {
        self.0
            .metaroot
            .specific
            .unstained
            .unstainedinfo
            .0
            .as_ref()
            .map(|x| x.clone().into())
    }

    #[setter]
    fn set_unstainedinfo(&mut self, x: Option<String>) {
        self.0.metaroot.specific.unstained.unstainedinfo = x.map(|x| x.into()).into()
    }

    #[getter]
    fn get_unstained_centers(&self) -> Option<HashMap<String, f32>> {
        self.0.get_metaroot_opt::<UnstainedCenters>().map(|y| {
            <HashMap<Shortname, f32>>::from(y.clone())
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect()
        })
    }

    fn insert_unstained_center(&mut self, k: PyShortname, v: f32) -> PyResult<Option<f32>> {
        self.0
            .insert_unstained_center(k.0, v)
            .map_err(|e| PyreflowException::new_err(e.to_string()))
    }

    fn remove_unstained_center(&mut self, k: PyShortname) -> Option<f32> {
        self.0.remove_unstained_center(&k.0)
    }

    fn clear_unstained_centers(&mut self) {
        self.0.clear_unstained_centers()
    }
}

// Get/set methods for all versions
macro_rules! common_methods {
    ($pytype:ident, $($rest:ident),*) => {
        common_methods!($pytype);
        common_methods!($($rest),+);

    };

    ($pytype:ident) => {
        get_set_metaroot_opt!(get_abrt, set_abrt, Abrt, u32, $pytype);
        get_set_metaroot_opt!(get_cells, set_cells, Cells, String, $pytype);
        get_set_metaroot_opt!(get_com, set_com, Com, String, $pytype);
        get_set_metaroot_opt!(get_exp, set_exp, Exp, String, $pytype);
        get_set_metaroot_opt!(get_fil, set_fil, Fil, String, $pytype);
        get_set_metaroot_opt!(get_inst, set_inst, Inst, String, $pytype);
        get_set_metaroot_opt!(get_lost, set_lost, Lost, u32, $pytype);
        get_set_metaroot_opt!(get_op, set_op, Op, String, $pytype);
        get_set_metaroot_opt!(get_proj, set_proj, Proj, String, $pytype);
        get_set_metaroot_opt!(get_smno, set_smno, Smno, String, $pytype);
        get_set_metaroot_opt!(get_src, set_src, Src, String, $pytype);
        get_set_metaroot_opt!(get_sys, set_sys, Sys, String, $pytype);

        // common measurement keywords
        get_set_all_optical!(get_filters, set_filters, String, Filter, $pytype);
        get_set_all_optical!(get_powers, set_powers, PyNonNegFloat, Power, $pytype);

        get_set_all_optical!(
            get_percents_emitted,
            set_percents_emitted,
            String,
            PercentEmitted,
            $pytype
        );

        get_set_all_optical!(
            get_detector_types,
            set_detector_types,
            String,
            DetectorType,
            $pytype
        );

        get_set_all_optical!(
            get_detector_voltages,
            set_detector_voltages,
            PyNonNegFloat,
            DetectorVoltage,
            $pytype
        );


        #[pymethods]
        impl $pytype {
            fn insert_nonstandard(&mut self, key: PyNonStdKey, v: String) -> Option<String> {
                self.0.metaroot.nonstandard_keywords.insert(key.0, v)
            }

            fn remove_nonstandard(&mut self, key: PyNonStdKey) -> Option<String> {
                self.0.metaroot.nonstandard_keywords.remove(&key.0)
            }

            fn get_nonstandard(&mut self, key: PyNonStdKey) -> Option<String> {
                self.0.metaroot.nonstandard_keywords.get(&key.0).cloned()
            }

            // TODO add way to remove nonstandard
            #[pyo3(signature = (want_req=None, want_meta=None))]
            fn raw_keywords<'py>(
                &self,
                py: Python<'py>,
                want_req: Option<bool>,
                want_meta: Option<bool>,
            ) -> PyResult<Bound<'py, PyDict>> {
                self.0.raw_keywords(want_req, want_meta).clone().into_py_dict(py)
            }

            #[getter]
            fn par(&self) -> usize {
                self.0.par().0
            }

            fn insert_meas_nonstandard(
                &mut self,
                keyvals: Vec<(PyNonStdKey, String)>,
            ) -> PyResult<Vec<Option<String>>> {
                let xs = keyvals.into_iter().map(|(k, v)| (k.0, v)).collect();
                self.0
                    .insert_meas_nonstandard(xs)
                    .map_err(|e| PyreflowException::new_err(e.to_string()))

            }

            fn remove_meas_nonstandard(
                &mut self,
                keys: Vec<PyNonStdKey>
            ) -> PyResult<Vec<Option<String>>> {
                let xs = keys.iter().map(|k| &k.0).collect();
                self.0
                    .remove_meas_nonstandard(xs)
                    .map_err(|e| PyreflowException::new_err(e.to_string()))
            }

            fn get_meas_nonstandard(
                &mut self,
                keys: Vec<PyNonStdKey>
            ) -> PyResult<Option<Vec<Option<String>>>> {
                let xs: Vec<_> = keys.into_iter().map(|k| k.0).collect();
                let res = self.0
                    .get_meas_nonstandard(&xs[..])
                    .map(|rs| rs.into_iter().map(|r| r.cloned()).collect());
                Ok(res)
            }

            #[getter]
            fn get_btim(&self) -> Option<NaiveTime> {
                self.0.btim_naive()
            }

            #[setter]
            fn set_btim(&mut self, x: Option<NaiveTime>) -> Result<(), PyReversedTimestamps> {
                self.0.set_btim_naive(x)?;
                Ok(())
            }

            #[getter]
            fn get_etim(&self) -> Option<NaiveTime> {
                self.0.etim_naive()
            }

            #[setter]
            fn set_etim(&mut self, x: Option<NaiveTime>) -> Result<(), PyReversedTimestamps> {
                self.0.set_etim_naive(x)?;
                Ok(())
            }

            #[getter]
            fn get_date(&self) -> Option<NaiveDate> {
                self.0.date_naive()
            }

            #[setter]
            fn set_date(&mut self, x: Option<NaiveDate>) -> Result<(), PyReversedTimestamps> {
                self.0.set_date_naive(x)?;
                Ok(())
            }

            #[getter]
            fn trigger_name(&self) -> Option<String> {
                self.0.trigger_name().map(|x| x.to_string())
            }

            #[getter]
            fn trigger_threshold(&self) -> Option<u32> {
                self.0.trigger_threshold()
            }

            #[setter]
            fn set_trigger_name(&mut self, name: PyShortname) -> bool {
                self.0.set_trigger_name(name.0)
            }

            #[setter]
            fn set_trigger_threshold(&mut self, x: u32) -> bool {
                self.0.set_trigger_threshold(x)
            }

            fn clear_trigger(&mut self) -> bool {
                self.0.clear_trigger()
            }

            #[getter]
            fn get_longnames(&self) -> Vec<Option<String>> {
                self.0
                    .longnames()
                    .into_iter()
                    .map(|x| x.map(|y| y.0.to_string()))
                    .collect()
            }

            #[setter]
            fn set_longnames(&mut self, ns: Vec<Option<String>>) -> PyResult<()> {
                self.0
                    .set_longnames(ns)
                    .map_err(|e| PyreflowException::new_err(e.to_string()))
            }

            #[getter]
            fn shortnames_maybe(&self) -> Vec<Option<String>> {
                self.0
                    .shortnames_maybe()
                    .into_iter()
                    .map(|x| x.map(|y| y.to_string()))
                    .collect()
            }

            #[getter]
            fn all_shortnames(&self) -> Vec<String> {
                self.0
                    .all_shortnames()
                    .into_iter()
                    .map(|x| x.to_string())
                    .collect()
            }

            #[setter]
            fn set_all_shortnames(&mut self, names: Vec<PyShortname>) -> PyResult<()> {
                // TODO this shouldn't be necessary
                let ns = names.into_iter().map(|s| s.0).collect();
                self.0
                    .set_all_shortnames(ns)
                    // TODO this is a setkeyserror, could be more generalized
                    .map_err(|e| PyreflowException::new_err(e.to_string()))
                    .void()
            }
        }
    };
}

common_methods!(
    PyCoreTEXT2_0,
    PyCoreTEXT3_0,
    PyCoreTEXT3_1,
    PyCoreTEXT3_2,
    PyCoreDataset2_0,
    PyCoreDataset3_0,
    PyCoreDataset3_1,
    PyCoreDataset3_2
);

macro_rules! temporal_get_set_2_0 {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                fn set_temporal(
                    &mut self,
                    name: PyShortname,
                    force: bool
                ) -> PyResult<bool> {
                    self.0
                        .set_temporal(&name.0, (), force)
                        .py_def_terminate(SetTemporalFailure)
                }

                fn set_temporal_at(
                    &mut self,
                    index: usize,
                    force: bool
                ) -> PyResult<bool> {
                    self.0
                        .set_temporal_at(index.into(), (), force)
                        .py_def_terminate(SetTemporalFailure)
                }

                fn unset_temporal(&mut self, force: bool) -> PyResult<bool> {
                    let out = self.0.unset_temporal(force).map(|x| x.is_some());
                    Ok(out).py_def_terminate(SetTemporalFailure)
                }
            }
        )*
    }
}

temporal_get_set_2_0!(PyCoreTEXT2_0, PyCoreDataset2_0);

macro_rules! temporal_get_set_3_0 {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                fn set_temporal(
                    &mut self,
                    name: PyShortname,
                    timestep: PyPositiveFloat,
                    force: bool
                ) -> PyResult<bool> {
                    self.0
                        .set_temporal(&name.0, timestep.into(), force)
                        .py_def_terminate(SetTemporalFailure)
                }

                fn set_temporal_at(
                    &mut self,
                    index: usize,
                    timestep: PyPositiveFloat,
                    force: bool
                ) -> PyResult<bool> {
                    self.0
                        .set_temporal_at(index.into(), timestep.into(), force)
                        .py_def_terminate(SetTemporalFailure)
                }

                fn unset_temporal(&mut self, force: bool) -> PyResult<Option<f32>> {
                    let out = self.0.unset_temporal(force).map(|x| x.map(|y| y.0.into()));
                    Ok(out).py_def_terminate(SetTemporalFailure)
                }
            }
        )*
    }
}

temporal_get_set_3_0!(
    PyCoreTEXT3_0,
    PyCoreTEXT3_1,
    PyCoreTEXT3_2,
    PyCoreDataset3_0,
    PyCoreDataset3_1,
    PyCoreDataset3_2
);

macro_rules! common_meas_get_set {
    ($pytype:ident, $o:ident, $t:ident) => {
        #[pymethods]
        impl $pytype {
            fn remove_measurement_by_name(
                &mut self,
                name: PyShortname,
            ) -> Option<(usize, PyElement<$t, $o>)> {
                self.0
                    .remove_measurement_by_name(&name.0)
                    .map(|(i, x)| (i.into(), x.inner_into().into()))
            }

            fn measurement_at(&self, i: usize) -> Result<PyElement<$t, $o>, PyElementIndexError> {
                let ms: &NamedVec<_, _, _, _> = self.0.as_ref();
                let m = ms.get(i.into()).map_err(PyElementIndexError)?;
                Ok(m.bimap(|x| x.1.clone(), |x| x.1.clone())
                    .inner_into()
                    .into())
            }

            fn replace_optical_at(
                &mut self,
                i: usize,
                m: $o,
            ) -> Result<PyElement<$t, $o>, PyElementIndexError> {
                let ret = self
                    .0
                    .replace_optical_at(i.into(), m.into())
                    .map_err(PyElementIndexError)?;
                Ok(ret.inner_into().into())
            }

            fn replace_optical_named(
                &mut self,
                name: PyShortname,
                m: $o,
            ) -> Option<PyElement<$t, $o>> {
                self.0
                    .replace_optical_named(&name.0, m.into())
                    .map(|r| r.inner_into().into())
            }

            fn rename_temporal(&mut self, name: PyShortname) -> Option<String> {
                self.0.rename_temporal(name.0).map(|n| n.to_string())
            }

            fn replace_temporal_at(
                &mut self,
                i: usize,
                m: $t,
                force: bool,
            ) -> PyResult<PyElement<$t, $o>> {
                let ret = self
                    .0
                    .replace_temporal_at(i.into(), m.into(), force)
                    .py_def_terminate(SetTemporalFailure)?;
                Ok(ret.inner_into().into())
            }

            fn replace_temporal_named(
                &mut self,
                name: PyShortname,
                m: $t,
                force: bool,
            ) -> PyResult<Option<PyElement<$t, $o>>> {
                let ret = self
                    .0
                    .replace_temporal_named(&name.0, m.into(), force)
                    .py_def_terminate(SetTemporalFailure)?;
                Ok(ret.map(|r| r.inner_into().into()))
            }

            #[getter]
            fn measurements(&self) -> Vec<PyElement<$t, $o>> {
                // This might seem inefficient since we are cloning
                // everything, but if we want to map a python lambda
                // function over the measurements we would need to to do
                // this anyways, so simply returnig a copied list doesn't
                // lose anything and keeps this API simpler.
                let ms: &NamedVec<_, _, _, _> = self.0.as_ref();
                ms.iter()
                    .map(|(_, e)| e.bimap(|t| t.value.clone(), |o| o.value.clone()))
                    .map(|v| v.inner_into().into())
                    .collect()
            }
        }
    };
}

common_meas_get_set!(PyCoreTEXT2_0, PyOptical2_0, PyTemporal2_0);
common_meas_get_set!(PyCoreTEXT3_0, PyOptical3_0, PyTemporal3_0);
common_meas_get_set!(PyCoreTEXT3_1, PyOptical3_1, PyTemporal3_1);
common_meas_get_set!(PyCoreTEXT3_2, PyOptical3_2, PyTemporal3_2);
common_meas_get_set!(PyCoreDataset2_0, PyOptical2_0, PyTemporal2_0);
common_meas_get_set!(PyCoreDataset3_0, PyOptical3_0, PyTemporal3_0);
common_meas_get_set!(PyCoreDataset3_1, PyOptical3_1, PyTemporal3_1);
common_meas_get_set!(PyCoreDataset3_2, PyOptical3_2, PyTemporal3_2);

macro_rules! common_coretext_meas_get_set {
    ($pytype:ident, $timetype:ident) => {
        #[pymethods]
        impl $pytype {
            fn push_temporal(
                &mut self,
                name: PyShortname,
                t: $timetype,
                r: BigDecimal,
                notrunc: bool,
            ) -> PyResult<()> {
                self.0
                    .push_temporal(name.0, t.into(), Range(r), notrunc)
                    .py_def_terminate(PushTemporalFailure)
            }

            fn insert_temporal(
                &mut self,
                i: usize,
                name: PyShortname,
                t: $timetype,
                r: BigDecimal,
                notrunc: bool,
            ) -> PyResult<()> {
                self.0
                    .insert_temporal(i.into(), name.0, t.into(), Range(r), notrunc)
                    .py_def_terminate(InsertTemporalFailure)
            }

            fn unset_measurements(&mut self) -> PyResult<()> {
                self.0
                    .unset_measurements()
                    .map_err(|e| PyreflowException::new_err(e.to_string()))
            }
        }
    };
}

common_coretext_meas_get_set!(PyCoreTEXT2_0, PyTemporal2_0);
common_coretext_meas_get_set!(PyCoreTEXT3_0, PyTemporal3_0);
common_coretext_meas_get_set!(PyCoreTEXT3_1, PyTemporal3_1);
common_coretext_meas_get_set!(PyCoreTEXT3_2, PyTemporal3_2);

macro_rules! coredata_meas_get_set {
    ($pytype:ident, $timetype:ident) => {
        #[pymethods]
        impl $pytype {
            fn push_temporal(
                &mut self,
                name: PyShortname,
                t: $timetype,
                col: PyFCSColumn,
                r: BigDecimal,
                notrunc: bool,
            ) -> PyResult<()> {
                self.0
                    .push_temporal(name.0, t.into(), col.0, Range(r), notrunc)
                    .py_def_terminate(PushTemporalFailure)
            }

            fn insert_time_channel(
                &mut self,
                i: usize,
                name: PyShortname,
                t: $timetype,
                col: PyFCSColumn,
                r: BigDecimal,
                notrunc: bool,
            ) -> PyResult<()> {
                self.0
                    .insert_temporal(i.into(), name.0, t.into(), col.0, Range(r), notrunc)
                    .py_def_terminate(InsertTemporalFailure)
            }

            fn unset_data(&mut self) -> PyResult<()> {
                self.0
                    .unset_data()
                    .map_err(|e| PyreflowException::new_err(e.to_string()))
            }

            #[getter]
            fn data(&self) -> PyDataFrame {
                let ns = self.0.all_shortnames();
                let df: &FCSDataFrame = self.0.as_ref();
                let columns = df
                    .iter_columns()
                    .zip(ns)
                    .map(|(c, n)| {
                        // ASSUME this will not fail because the we know the types and
                        // we don't have a validity array
                        Series::from_arrow(n.as_ref().into(), c.as_array())
                            .unwrap()
                            .into()
                    })
                    .collect();
                // ASSUME this will not fail because all columns should have unique
                // names and the same length
                PyDataFrame(DataFrame::new(columns).unwrap())
            }

            #[getter]
            fn analysis(&self) -> Vec<u8> {
                self.0.analysis.0.clone()
            }

            #[setter]
            fn set_analysis(&mut self, xs: Vec<u8>) {
                self.0.analysis = xs.into();
            }

            #[getter]
            fn others(&self) -> Vec<Vec<u8>> {
                self.0.others.0.clone().into_iter().map(|x| x.0).collect()
            }

            #[setter]
            fn set_others(&mut self, xs: Vec<Vec<u8>>) {
                self.0.others = Others(xs.into_iter().map(Other).collect());
            }
        }
    };
}

coredata_meas_get_set!(PyCoreDataset2_0, PyTemporal2_0);
coredata_meas_get_set!(PyCoreDataset3_0, PyTemporal3_0);
coredata_meas_get_set!(PyCoreDataset3_1, PyTemporal3_1);
coredata_meas_get_set!(PyCoreDataset3_2, PyTemporal3_2);

macro_rules! coretext2_0_meas_methods {
    ($pytype:ident, $o:ident, $t:ident) => {
        #[pymethods]
        impl $pytype {
            fn remove_measurement_by_index(
                &mut self,
                index: usize,
            ) -> Result<(Option<PyShortname>, PyElement<$t, $o>), PyElementIndexError> {
                let r = self
                    .0
                    .remove_measurement_by_index(index.into())
                    .map_err(PyElementIndexError)?;
                let (n, v) = Element::unzip::<MaybeFamily>(r);
                Ok((n.0.map(|x| x.into()), v.inner_into().into()))
            }

            #[pyo3(signature = (m, r, notrunc=false, name=None))]
            fn push_measurement(
                &mut self,
                m: $o,
                r: BigDecimal,
                notrunc: bool,
                name: Option<PyShortname>,
            ) -> PyResult<PyShortname> {
                self.0
                    .push_optical(name.map(|n| n.0).into(), m.into(), r.into(), notrunc)
                    .py_def_terminate(InsertOpticalFailure)
                    .map(|x| x.into())
            }

            #[pyo3(signature = (i, m, r, notrunc=false, name=None))]
            fn insert_optical(
                &mut self,
                i: usize,
                m: $o,
                r: BigDecimal,
                notrunc: bool,
                name: Option<PyShortname>,
            ) -> PyResult<PyShortname> {
                let n = name.map(|n| n.0).into();
                self.0
                    .insert_optical(i.into(), n, m.into(), Range(r), notrunc)
                    .py_def_terminate(InsertOpticalFailure)
                    .map(|x| x.into())
            }
        }
    };
}

coretext2_0_meas_methods!(PyCoreTEXT2_0, PyOptical2_0, PyTemporal2_0);
coretext2_0_meas_methods!(PyCoreTEXT3_0, PyOptical3_0, PyTemporal3_0);

macro_rules! coretext3_1_meas_methods {
    ($pytype:ident, $o:ident, $t:ident) => {
        #[pymethods]
        impl $pytype {
            fn remove_measurement_by_index(
                &mut self,
                index: usize,
            ) -> Result<(PyShortname, PyElement<$t, $o>), PyElementIndexError> {
                let r = self
                    .0
                    .remove_measurement_by_index(index.into())
                    .map_err(PyElementIndexError)?;
                let (n, v) = Element::unzip::<AlwaysFamily>(r);
                Ok((n.0.into(), v.inner_into().into()))
            }

            fn push_optical(
                &mut self,
                m: $o,
                name: PyShortname,
                r: BigDecimal,
                notrunc: bool,
            ) -> PyResult<()> {
                self.0
                    .push_optical(AlwaysValue(name.0), m.into(), Range(r), notrunc)
                    .py_def_terminate(PushOpticalFailure)
                    .void()
            }

            fn insert_optical(
                &mut self,
                i: usize,
                m: $o,
                name: PyShortname,
                r: BigDecimal,
                notrunc: bool,
            ) -> PyResult<()> {
                self.0
                    .insert_optical(i.into(), AlwaysValue(name.0), m.into(), Range(r), notrunc)
                    .py_def_terminate(InsertOpticalFailure)
                    .void()
            }
        }
    };
}

coretext3_1_meas_methods!(PyCoreTEXT3_1, PyOptical3_1, PyTemporal3_1);
coretext3_1_meas_methods!(PyCoreTEXT3_2, PyOptical3_2, PyTemporal3_2);

macro_rules! set_measurements_ordered {
    ($pytype:ident, $t:ident, $o:ident) => {
        #[pymethods]
        impl $pytype {
            fn set_measurements(
                &mut self,
                xs: PyRawMaybeInput<$t, $o>,
                prefix: PyShortnamePrefix,
            ) -> PyResult<()> {
                self.0
                    .set_measurements(xs.0.inner_into(), prefix.0)
                    .py_mult_terminate(SetMeasurementsFailure)
                    .void()
            }

            fn set_measurements_and_layout(
                &mut self,
                xs: PyRawMaybeInput<$t, $o>,
                layout: PyOrderedLayout,
                prefix: PyShortnamePrefix,
            ) -> PyResult<()> {
                self.0
                    .set_measurements_and_layout(xs.0.inner_into(), layout.into(), prefix.0)
                    .py_mult_terminate(SetMeasurementsFailure)
                    .void()
            }

            #[getter]
            fn get_layout(&self) -> Option<PyOrderedLayout> {
                let x: &Option<_> = self.0.as_ref();
                x.as_ref().map(|y| y.clone().into())
            }

            fn set_layout(&mut self, layout: PyOrderedLayout) -> PyResult<()> {
                self.0
                    .set_layout(layout.into())
                    .py_mult_terminate(SetLayoutFailure)
            }
        }
    };
}

set_measurements_ordered!(PyCoreTEXT2_0, PyTemporal2_0, PyOptical2_0);
set_measurements_ordered!(PyCoreTEXT3_0, PyTemporal3_0, PyOptical3_0);
set_measurements_ordered!(PyCoreDataset2_0, PyTemporal2_0, PyOptical2_0);
set_measurements_ordered!(PyCoreDataset3_0, PyTemporal3_0, PyOptical3_0);

macro_rules! set_measurements_endian {
    ($pytype:ident, $t:ident, $o:ident, $l:ident) => {
        #[pymethods]
        impl $pytype {
            pub fn set_measurements(&mut self, xs: PyRawAlwaysInput<$t, $o>) -> PyResult<()> {
                self.0
                    .set_measurements_noprefix(xs.0.inner_into())
                    .py_mult_terminate(SetMeasurementsFailure)
                    .void()
            }

            fn set_measurements_and_layout(
                &mut self,
                xs: PyRawAlwaysInput<$t, $o>,
                layout: $l,
            ) -> PyResult<()> {
                self.0
                    .set_measurements_and_layout_noprefix(xs.0.inner_into(), layout.into())
                    .py_mult_terminate(SetMeasurementsFailure)
                    .void()
            }

            #[getter]
            fn get_layout(&self) -> Option<$l> {
                let x: &Option<_> = self.0.as_ref();
                x.as_ref().map(|y| y.clone().into())
            }

            fn set_layout(&mut self, layout: $l) -> PyResult<()> {
                self.0
                    .set_layout(layout.into())
                    .py_mult_terminate(SetLayoutFailure)
            }
        }
    };
}

set_measurements_endian!(PyCoreTEXT3_1, PyTemporal3_1, PyOptical3_1, PyNonMixedLayout);
set_measurements_endian!(PyCoreTEXT3_2, PyTemporal3_2, PyOptical3_2, PyLayout3_2);
set_measurements_endian!(
    PyCoreDataset3_1,
    PyTemporal3_1,
    PyOptical3_1,
    PyNonMixedLayout
);
set_measurements_endian!(PyCoreDataset3_2, PyTemporal3_2, PyOptical3_2, PyLayout3_2);

macro_rules! coredata2_0_meas_methods {
    ($pytype:ident, $t:ident, $o:ident) => {
        #[pymethods]
        impl $pytype {
            fn set_measurements_and_data(
                &mut self,
                xs: PyRawMaybeInput<$t, $o>,
                cols: PyFCSColumns,
                prefix: PyShortnamePrefix,
            ) -> PyResult<()> {
                self.0
                    .set_measurements_and_data(xs.0.inner_into(), cols.0, prefix.0)
                    .py_mult_terminate(SetMeasurementsFailure)
                    .void()
            }
        }
    };
}

coredata2_0_meas_methods!(PyCoreDataset2_0, PyTemporal2_0, PyOptical2_0);
coredata2_0_meas_methods!(PyCoreDataset3_0, PyTemporal3_0, PyOptical3_0);

macro_rules! coredata3_1_meas_methods {
    ($pytype:ident, $t:ident, $o:ident) => {
        #[pymethods]
        impl $pytype {
            fn set_measurements_and_data(
                &mut self,
                xs: PyRawAlwaysInput<$t, $o>,
                cols: PyFCSColumns,
            ) -> PyResult<()> {
                self.0
                    .set_measurements_and_data_noprefix(xs.0.inner_into(), cols.0)
                    .py_mult_terminate(SetMeasurementsFailure)
                    .void()
            }
        }
    };
}

coredata3_1_meas_methods!(PyCoreDataset3_1, PyTemporal3_1, PyOptical3_1);
coredata3_1_meas_methods!(PyCoreDataset3_2, PyTemporal3_2, PyOptical3_2);

// Get/set methods for setting $PnN (2.0-3.0)
macro_rules! shortnames_methods {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                fn set_measurement_shortnames_maybe(
                    &mut self,
                    names: Vec<Option<PyShortname>>,
                ) -> PyResult<()> {
                    // TODO do this better
                    let ns = names.into_iter().map(|n| n.map(|x| x.0)).collect();
                    self.0
                        .set_measurement_shortnames_maybe(ns)
                        .map_err(|e| PyreflowException::new_err(e.to_string()))
                        .void()
                }
            }
        )*
    };
}

shortnames_methods!(
    PyCoreTEXT2_0,
    PyCoreTEXT3_0,
    PyCoreDataset2_0,
    PyCoreDataset3_0
);

// Get/set methods for $PnE (2.0)
macro_rules! scales_methods {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn get_all_scales(&self) -> Vec<Option<PyScale>> {
                    self.0.get_all_scales().map(|x| x.map(|y| y.into())).collect()
                }

                #[getter]
                fn get_scales(&self) -> Vec<(usize, Option<PyScale>)> {
                    self.0
                        .get_optical_opt::<Scale>()
                        .map(|(i, s)| (i.into(), s.map(|&x| x.into())))
                        .collect()
                }

                #[setter]
                fn set_scales(&mut self, xs: Vec<Option<PyScale>>) -> PyResult<()> {
                    let ys = xs.into_iter().map(|x| x.map(|y| y.into())).collect();
                    self.0
                        .set_scales(ys)
                        .py_mult_terminate(SetMeasurementsFailure)
                        .void()
                }
            }
        )*
    };
}

scales_methods!(PyCoreTEXT2_0, PyCoreDataset2_0);

// Get/set methods for $PnE (3.0-3.2)
macro_rules! transforms_methods {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn get_all_transforms(&self) -> Vec<PyScaleTransform> {
                    self.0.get_all_transforms().map(|x| x.into()).collect()
                }

                #[getter]
                fn get_transforms(&self) -> Vec<(usize, PyScaleTransform)> {
                    self.0
                        .get_optical::<ScaleTransform>()
                        .map(|(i, &s)| (i.into(), s.into()))
                        .collect()
                }

                #[setter]
                fn set_transforms(&mut self, xs: Vec<PyScaleTransform>) -> PyResult<()> {
                    let ys = xs.into_iter().map(|x| x.into()).collect();
                    self.0
                        .set_transforms(ys)
                        .py_mult_terminate(SetMeasurementsFailure)
                        .void()
                }
            }
        )*
    };
}

transforms_methods!(
    PyCoreTEXT3_0,
    PyCoreTEXT3_1,
    PyCoreTEXT3_2,
    PyCoreDataset3_0,
    PyCoreDataset3_1,
    PyCoreDataset3_2
);

// Get/set methods for $TIMESTEP (3.0-3.2)
macro_rules! timestep_methods {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn get_timestep(&self) -> Option<f32> {
                    self.0.timestep().map(|&x| x.into())
                }

                #[setter]
                fn set_timestep(&mut self, ts: PyPositiveFloat) -> bool {
                    self.0.set_timestep(ts.into())
                }
            }
        )*
    };
}

timestep_methods!(
    PyCoreTEXT3_0,
    PyCoreTEXT3_1,
    PyCoreTEXT3_2,
    PyCoreDataset3_0,
    PyCoreDataset3_1,
    PyCoreDataset3_2
);

// Get/set methods for scaler $PnL (2.0-3.0)
macro_rules! wavelength_methods {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn get_wavelengths(&self) -> Vec<(usize, Option<f32>)> {
                    self.0.get_optical_opt::<Wavelength>()
                        .map(|(i, x)| (i.into(), x.map(|y| y.0.into())))
                        .collect()
                }

                #[setter]
                fn set_wavelengths(&mut self, xs: Vec<Option<PyPositiveFloat>>) -> PyResult<()> {
                    let ys = xs
                        .into_iter()
                        .map(|x| x.map(|y| Wavelength::from(y.0)))
                        .collect();
                    self.0
                        .set_optical(ys)
                        .map_err(|e| PyreflowException::new_err(e.to_string()))
                }
            }
        )*
    };
}

wavelength_methods!(
    PyCoreTEXT2_0,
    PyCoreTEXT3_0,
    PyCoreDataset2_0,
    PyCoreDataset3_0
);

// Get/set methods for vector $PnL (3.1-3.2)
macro_rules! wavelengths_methods {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn get_wavelengths(&self) -> Vec<(usize, Vec<f32>)> {
                    self.0.get_optical_opt::<Wavelengths>()
                        .map(|(i, x)| {
                            (
                                i.into(),
                                x.map(|y| y.clone().into()).unwrap_or_default(),
                            )
                        })
                        .collect()
                }

                #[setter]
                fn set_wavelengths(&mut self, xs: Vec<Vec<PyPositiveFloat>>) -> PyResult<()> {
                    // TODO cleanme
                    let ys = xs
                        .into_iter()
                        .map(|ys| NonEmpty::from_vec(ys).map(|zs| zs.map(|z| z.0)))
                        .map(|ys| ys.map(Wavelengths::from))
                        .collect();
                    self.0
                        .set_optical(ys)
                        .map_err(|e| PyreflowException::new_err(e.to_string()))
                }
            }
        )*
    };
}

wavelengths_methods!(
    PyCoreTEXT3_1,
    PyCoreTEXT3_2,
    PyCoreDataset3_1,
    PyCoreDataset3_2
);

// Get/set methods for $LAST_MODIFIER/$LAST_MODIFIED/$ORIGINALITY (3.1-3.2)
macro_rules! modification_methods {
    ($($pytype:ident),+) => {
        get_set_metaroot_opt!(
            get_originality,
            set_originality,
            Originality,
            PyOriginality,
            $($pytype),*
        );

        get_set_metaroot_opt!(
            get_last_modified,
            set_last_modified,
            ModifiedDateTime,
            NaiveDateTime,
            $($pytype),*
        );

        get_set_metaroot_opt!(
            get_last_modifier,
            set_last_modifier,
            LastModifier,
            String,
            $($pytype),*
        );
    };
}

modification_methods!(
    PyCoreTEXT3_1,
    PyCoreTEXT3_2,
    PyCoreDataset3_1,
    PyCoreDataset3_2
);

// Get/set methods for $CARRIERID/$CARRIERTYPE/$LOCATIONID (3.2)
macro_rules! carrier_methods {
    ($($pytype:ident),*) => {
        get_set_metaroot_opt!(get_carriertype, set_carriertype, Carriertype, String, $($pytype),*);
        get_set_metaroot_opt!(get_carrierid,   set_carrierid,   Carrierid,   String, $($pytype),*);
        get_set_metaroot_opt!(get_locationid,  set_locationid,  Locationid,  String, $($pytype),*);
    };
}

carrier_methods!(PyCoreTEXT3_2, PyCoreDataset3_2);

// Get/set methods for $PLATEID/$WELLID/$PLATENAME (3.1-3.2)
macro_rules! plate_methods {
    ($($pytype:ident),*) => {
        get_set_metaroot_opt!(get_wellid,    set_wellid,    Wellid,    String, $($pytype),*);
        get_set_metaroot_opt!(get_plateid,   set_plateid,   Plateid,   String, $($pytype),*);
        get_set_metaroot_opt!(get_platename, set_platename, Platename, String, $($pytype),*);
    };
}

plate_methods!(
    PyCoreTEXT3_1,
    PyCoreTEXT3_2,
    PyCoreDataset3_1,
    PyCoreDataset3_2
);

// get/set methods for $COMP (2.0-3.0)
macro_rules! comp_methods {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn get_compensation<'a>(&self, py: Python<'a>) -> Option<Bound<'a, PyArray2<f32>>> {
                    self.0.compensation().map(|x| x.to_pyarray(py))
                }


                fn set_compensation(
                    &mut self,
                    a: PyReadonlyArray2<f32>,
                ) -> Result<(), PyErr> {
                    let m = a.as_matrix().into_owned();
                    self.0
                        .set_compensation(m)
                        .map_err(|e| PyreflowException::new_err(e.to_string()))
                }

                fn unset_compensation(&mut self) {
                    self.0.unset_compensation()
                }
            }
        )*
    };
}

comp_methods!(
    PyCoreTEXT2_0,
    PyCoreTEXT3_0,
    PyCoreDataset2_0,
    PyCoreDataset3_0
);

// Get/set methods for $SPILLOVER (3.1-3.2)
macro_rules! spillover_methods {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn get_spillover_matrix<'a>(&self, py: Python<'a>) -> Option<Bound<'a, PyArray2<f32>>> {
                    self.0.spillover_matrix().map(|x| x.to_pyarray(py))
                }

                #[getter]
                fn get_spillover_names(&self) -> Vec<String> {
                    self.0
                        .spillover_names()
                        .map(|x| x.iter().map(|y| y.clone().into()).collect())
                        .unwrap_or_default()
                }

                fn set_spillover(
                    &mut self,
                    names: Vec<PyShortname>,
                    a: PyReadonlyArray2<f32>,
                ) -> Result<(), PyErr> {
                    let ns = names.into_iter().map(|n| n.0).collect();
                    let m = a.as_matrix().into_owned();
                    self.0
                        .set_spillover(ns, m)
                        // TODO handle error better
                        .map_err(|e| PyreflowException::new_err(e.to_string()))
                }

                fn unset_spillover(&mut self) {
                    self.0.unset_spillover()
                }
            }
        )*
    };
}

spillover_methods!(
    PyCoreTEXT3_1,
    PyCoreTEXT3_2,
    PyCoreDataset3_1,
    PyCoreDataset3_2
);

get_set_metaroot_opt!(
    get_vol,
    set_vol,
    Vol,
    PyNonNegFloat,
    PyCoreTEXT3_1,
    PyCoreTEXT3_2,
    PyCoreDataset3_1,
    PyCoreDataset3_2
);

// Get/set methods for (optional) $CYT (2.0-3.1)
//
// 3.2 is required which is why it is not included here
get_set_metaroot_opt!(
    get_cyt,
    set_cyt,
    Cyt,
    String,
    PyCoreTEXT2_0,
    PyCoreTEXT3_0,
    PyCoreTEXT3_1,
    PyCoreDataset2_0,
    PyCoreDataset3_0,
    PyCoreDataset3_1
);

// Get/set methods for $FLOWRATE (3.2)
get_set_metaroot_opt!(
    get_flowrate,
    set_flowrate,
    Flowrate,
    String,
    PyCoreTEXT3_2,
    PyCoreDataset3_2
);

// Get/set methods for $CYTSN (3.0-3.2)
get_set_metaroot_opt!(
    get_cytsn,
    set_cytsn,
    Cytsn,
    String,
    PyCoreTEXT3_0,
    PyCoreTEXT3_1,
    PyCoreTEXT3_2,
    PyCoreDataset3_0,
    PyCoreDataset3_1,
    PyCoreDataset3_2
);

// Get/set methods for $PnD (3.1+)
//
// This is valid for the time channel so don't set on just optical
get_set_all_meas!(
    get_displays,
    set_displays,
    PyDisplay,
    Display,
    PyCoreTEXT3_1,
    PyCoreDataset3_1,
    PyCoreTEXT3_2,
    PyCoreDataset3_2
);

// Get/set methods for $PnDET (3.2)
get_set_all_optical!(
    get_detector_names,
    set_detector_names,
    String,
    DetectorName,
    PyCoreTEXT3_2,
    PyCoreDataset3_2
);

// Get/set methods for $PnCALIBRATION (3.1)
get_set_all_optical!(
    get_calibrations,
    set_calibrations,
    PyCalibration3_1,
    Calibration3_1,
    PyCoreTEXT3_1,
    PyCoreDataset3_1
);

// Get/set methods for $PnCALIBRATION (3.2)
get_set_all_optical!(
    get_calibrations,
    set_calibrations,
    PyCalibration3_2,
    Calibration3_2,
    PyCoreTEXT3_2,
    PyCoreDataset3_2
);

// Get/set methods for $PnTAG (3.2)
get_set_all_optical!(
    get_tags,
    set_tags,
    String,
    Tag,
    PyCoreTEXT3_2,
    PyCoreDataset3_2
);

// Get/set methods for $PnTYPE (3.2)
get_set_all_optical!(
    get_measurement_types,
    set_measurement_types,
    PyOpticalType,
    OpticalType,
    PyCoreTEXT3_2,
    PyCoreDataset3_2
);

// Get/set methods for $PnFEATURE (3.2)
get_set_all_optical!(
    get_features,
    set_features,
    PyFeature,
    Feature,
    PyCoreTEXT3_2,
    PyCoreDataset3_2
);

// Get/set methods for $PnANALYTE (3.2)
get_set_all_optical!(
    get_analytes,
    set_analytes,
    String,
    Analyte,
    PyCoreTEXT3_2,
    PyCoreDataset3_2
);

// Add method to convert CoreTEXT* to CoreDataset* by adding DATA, ANALYSIS, and
// OTHER(s) (all versions)
macro_rules! to_dataset_method {
    ($from:ident, $to:ident) => {
        #[pymethods]
        impl $from {
            fn to_dataset(
                &self,
                cols: PyFCSColumns,
                analysis: Vec<u8>,
                others: Vec<Vec<u8>>,
            ) -> PyResult<$to> {
                self.0
                    .clone()
                    .into_coredataset(
                        cols.0,
                        analysis.into(),
                        Others(others.into_iter().map(|x| x.into()).collect()),
                    )
                    .map_err(|e| PyreflowException::new_err(e.to_string()))
                    .map(|df| df.into())
            }
        }
    };
}

to_dataset_method!(PyCoreTEXT2_0, PyCoreDataset2_0);
to_dataset_method!(PyCoreTEXT3_0, PyCoreDataset3_0);
to_dataset_method!(PyCoreTEXT3_1, PyCoreDataset3_1);
to_dataset_method!(PyCoreTEXT3_2, PyCoreDataset3_2);

// TODO there might a more natural way to emit all these warnings when
// converting from a rust type to a python type, that way I don't need to call
// this repeatedly
fn handle_warnings<X, W>(t: Terminal<X, W>) -> PyResult<X>
where
    W: fmt::Display,
{
    let (x, warn_res) = t.resolve(emit_warnings);
    warn_res?;
    Ok(x)
}

fn emit_warnings<W>(ws: Vec<W>) -> PyResult<()>
where
    W: fmt::Display,
{
    Python::with_gil(|py| -> PyResult<()> {
        let wt = py.get_type::<PyreflowWarning>();
        for w in ws {
            let s = CString::new(w.to_string())?;
            PyErr::warn(py, &wt, &s, 0)?;
        }
        Ok(())
    })
}

// TODO use warnings_are_errors flag
// TODO python has a way of handling multiple exceptions (ExceptionGroup)
// starting in 3.11
fn handle_failure<W, E, T>(f: TerminalFailure<W, E, T>) -> PyErr
where
    E: fmt::Display,
    T: fmt::Display,
    W: fmt::Display,
{
    let (warn_res, e) = f.resolve(emit_warnings, emit_failure);
    if let Err(w) = warn_res {
        w
    } else {
        e
    }
}

fn handle_failure_nowarn<E, T>(f: TerminalFailure<(), E, T>) -> PyErr
where
    E: fmt::Display,
    T: fmt::Display,
{
    f.resolve(|_| (), emit_failure).1
}

fn emit_failure<E, T>(es: NonEmpty<E>, r: T) -> PyErr
where
    E: fmt::Display,
    T: fmt::Display,
{
    let s = {
        let xs: Vec<_> = [format!("Toplevel Error: {r}")]
            .into_iter()
            .chain(es.into_iter().map(|x| x.to_string()))
            .collect();
        xs[..].join("\n").to_string()
    };
    PyreflowException::new_err(s)
}

create_exception!(
    pyreflow,
    PyreflowException,
    PyException,
    "Exception created by internal pyreflow."
);

create_exception!(
    pyreflow,
    PyreflowWarning,
    PyWarning,
    "Warning created by internal pyreflow."
);

#[pymethods]
impl PyOptical2_0 {
    #[new]
    fn new() -> Self {
        Optical2_0::default().into()
    }

    #[getter]
    fn get_scale(&self) -> Option<PyScale> {
        self.0.specific.scale.0.as_ref().map(|&x| x.into())
    }

    #[setter]
    fn set_scale(&mut self, x: Option<PyScale>) {
        self.0.specific.scale = x.map(|y| y.into()).into()
    }
}

#[pymethods]
impl PyOptical3_0 {
    #[new]
    fn new(scale: PyScale) -> Self {
        Optical3_0::new(scale.into()).into()
    }
}

#[pymethods]
impl PyOptical3_1 {
    #[new]
    fn new(scale: PyScale) -> Self {
        Optical3_1::new(scale.into()).into()
    }
}

#[pymethods]
impl PyOptical3_2 {
    #[new]
    fn new(scale: PyScale) -> Self {
        Optical3_2::new(scale.into()).into()
    }
}

#[pymethods]
impl PyTemporal2_0 {
    #[new]
    fn new() -> Self {
        Temporal2_0::default().into()
    }
}

#[pymethods]
impl PyTemporal3_0 {
    #[new]
    fn new(timestep: PyPositiveFloat) -> Self {
        Temporal3_0::new(timestep.into()).into()
    }
}

#[pymethods]
impl PyTemporal3_1 {
    #[new]
    fn new(timestep: PyPositiveFloat) -> Self {
        Temporal3_1::new(timestep.into()).into()
    }
}

#[pymethods]
impl PyTemporal3_2 {
    #[new]
    fn new(timestep: PyPositiveFloat) -> Self {
        Temporal3_2::new(timestep.into()).into()
    }

    #[getter]
    fn get_measurement_type(&self) -> bool {
        self.0.specific.measurement_type.0.is_some()
    }

    #[setter]
    fn set_measurement_type(&mut self, x: bool) {
        self.0.specific.measurement_type = if x { Some(TemporalType) } else { None }.into();
    }
}

macro_rules! shared_meas_get_set {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                // #[getter]
                // fn width(&self) -> Option<u8> {
                //     self.0.common.width.into()
                // }

                // #[setter]
                // fn set_width(&mut self, x: Option<u8>) {
                //     self.0.common.width = x.into();
                // }

                // #[getter]
                // fn range<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
                //     float_or_int_to_any(self.0.common.range.0, py)
                // }

                // #[setter]
                // fn set_range(&mut self, x: Bound<'_, PyAny>) -> PyResult<()> {
                //     self.0.common.range = any_to_range(x)?;
                //     Ok(())
                // }

                #[getter]
                fn longname(&self) -> Option<String> {
                    self.0.common.longname.as_ref_opt().map(|x| x.clone().into())
                }

                #[setter]
                fn set_longname(&mut self, x: Option<String>) {
                    self.0.common.longname = x.map(|y| y.into()).into();
                }

                #[getter]
                fn nonstandard_keywords(&self) -> HashMap<String, String> {
                    self.0
                        .common
                        .nonstandard_keywords
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.clone()))
                        .collect()
                }

                #[setter]
                fn set_nonstandard_keywords(&mut self, xs: HashMap<String, String>) -> PyResult<()> {
                    let mut ys = HashMap::new();
                    for (k, v) in xs {
                        let kk = k
                            .parse::<NonStdKey>()
                            .map_err(|e| PyreflowException::new_err(e.to_string()))?;
                        ys.insert(kk, v);
                    }
                    self.0.common.nonstandard_keywords = ys;
                    Ok(())
                }

                fn nonstandard_insert(
                    &mut self,
                    key: PyNonStdKey,
                    value: String
                ) -> Option<String> {
                    self.0.common.nonstandard_keywords.insert(key.0, value)
                }

                fn nonstandard_get(&self, key: PyNonStdKey) -> Option<String> {
                    self.0.common.nonstandard_keywords.get(&key.0).map(|x| x.clone())
                }

                fn nonstandard_remove(&mut self, key: PyNonStdKey) -> Option<String> {
                    self.0.common.nonstandard_keywords.remove(&key.0)
                }
            }
        )*
    };
}

shared_meas_get_set!(
    PyOptical2_0,
    PyOptical3_0,
    PyOptical3_1,
    PyOptical3_2,
    PyTemporal2_0,
    PyTemporal3_0,
    PyTemporal3_1,
    PyTemporal3_2
);

macro_rules! get_set_meas {
    ($get:ident, $set:ident, $outer:ident, $inner:ident, $($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn $get(&self) -> Option<$outer> {
                    let x: &Option<$inner> = self.0.as_ref();
                    x.as_ref().map(|y| y.clone().into())
                }

                #[setter]
                fn $set(&mut self, x: Option<$outer>) {
                    *self.0.as_mut() = x.map(|y| $inner::from(y))
                }
            }
        )*

    };
}

macro_rules! optical_common {
    ($($pytype:ident),*) => {
        get_set_meas!(
            get_filter,
            set_filter,
            String,
            Filter,
            $($pytype),*
        );

        get_set_meas!(
            get_detector_type,
            set_detector_type,
            String,
            DetectorType,
            $($pytype),*
        );

        get_set_meas!(
            get_percent_emitted,
            set_percent_emitted,
            String,
            PercentEmitted,
            $($pytype),*
        );

        get_set_meas!(
            get_detector_voltage,
            set_detector_voltage,
            PyNonNegFloat,
            DetectorVoltage,
            $($pytype),*
        );

        get_set_meas!(
            get_power,
            set_power,
            PyNonNegFloat,
            Power,
            $($pytype),*
        );
    };
}

optical_common!(PyOptical2_0, PyOptical3_0, PyOptical3_1, PyOptical3_2);

// $PnE (2.0)
macro_rules! get_set_meas_scale {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
            }
        )*
    };
}

get_set_meas_scale!(PyOptical2_0);

// $PnE (3.0-3.2)
macro_rules! get_set_meas_transform {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn get_transform(&self) -> PyScaleTransform {
                    self.0.specific.scale.into()
                }

                #[setter]
                fn set_transform(&mut self, x: PyScaleTransform) {
                    self.0.specific.scale = x.into();
                }
            }
        )*
    };
}

get_set_meas_transform!(PyOptical3_0, PyOptical3_1, PyOptical3_2);

// $PnL (2.0/3.0)
get_set_meas!(
    get_wavelength,
    set_wavelength,
    PyPositiveFloat,
    Wavelength,
    PyOptical2_0,
    PyOptical3_0
);

// #PnL (3.1-3.2)
macro_rules! meas_get_set_wavelengths {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn get_wavelengths(&self) -> Vec<f32> {
                    let ws: &Option<Wavelengths> = self.0.as_ref();
                    ws.as_ref().map(|xs: &Wavelengths| xs.clone().into()).unwrap_or_default()
                }

                #[setter]
                fn set_wavelengths(&mut self, xs: Vec<PyPositiveFloat>) {
                    let ws = if let Some(ys) = NonEmpty::from_vec(xs) {
                        let ws = Wavelengths::from(ys.map(|y| y.0));
                        Some(ws)
                    } else {
                        None.into()
                    };
                    *self.0.as_mut() = ws;
                }
            }
        )*
    };
}

meas_get_set_wavelengths!(PyOptical3_1, PyOptical3_2);

// #TIMESTEP (3.0-3.2)
macro_rules! meas_get_set_timestep {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn get_timestep(&self) -> f32 {
                    self.0.specific.timestep.0.into()
                }

                #[setter]
                fn set_timestep(&mut self, x: PyPositiveFloat) {
                    self.0.specific.timestep = x.into()
                }
            }
        )*
    };
}

meas_get_set_timestep!(PyTemporal3_0, PyTemporal3_1, PyTemporal3_2);

// $PnCalibration (3.1)
get_set_meas!(
    get_calibration,
    set_calibration,
    PyCalibration3_1,
    Calibration3_1,
    PyOptical3_1
);

// $PnD (3.1-3.2)
get_set_meas!(
    get_display,
    set_display,
    PyDisplay,
    Display,
    PyOptical3_1,
    PyOptical3_2,
    PyTemporal3_1,
    PyTemporal3_2
);

// $PnDATATYPE (3.2)
// get_set_copied!(
//     PyOptical3_2,
//     PyTemporal3_2,
//     [specific],
//     get_datatype,
//     set_datatype,
//     datatype,
//     PyNumType
// );

// $PnDET (3.2)
get_set_meas!(get_det, set_det, String, DetectorName, PyOptical3_2);

// $PnTAG (3.2)
get_set_meas!(get_tag, set_tag, String, Tag, PyOptical3_2);

// $PnTYPE (3.2)
get_set_meas!(
    get_measurement_type,
    set_measurement_type,
    PyOpticalType,
    OpticalType,
    PyOptical3_2
);

// $PnFEATURE (3.2)
get_set_meas!(get_feature, set_feature, PyFeature, Feature, PyOptical3_2);

// $PnANALYTE (3.2)
get_set_meas!(get_analyte, set_analyte, String, Analyte, PyOptical3_2);

// $PnCalibration (3.2)
get_set_meas!(
    get_calibration,
    set_calibration,
    PyCalibration3_2,
    Calibration3_2,
    PyOptical3_2
);

// TODO deref for stuff like this?
/// A python value for a segment.
///
/// This is represented as a tuple like `(u64, u64)` where the two numbers
/// are exactly as they appear in the FCS file.
#[derive(From)]
struct PySegment(Segment<u64>);

impl<'py> IntoPyObject<'py> for PySegment {
    type Target = PyTuple;
    type Output = Bound<'py, <(u64, u64) as IntoPyObject<'py>>::Target>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        self.0.try_coords().unwrap_or((0, 0)).into_pyobject(py)
    }
}

/// A python value for the values in the HEADER as a dictionary.
#[derive(IntoPyObject)]
struct PyHeader {
    version: PyVersion,
    text: PySegment,
    data: PySegment,
    analysis: PySegment,
    other: Vec<PySegment>,
}

impl From<Header> for PyHeader {
    fn from(value: Header) -> Self {
        let s = value.segments;
        Self {
            version: value.version.into(),
            text: s.text.inner.as_u64().into(),
            data: s.data.inner.as_u64().into(),
            analysis: s.analysis.inner.as_u64().into(),
            other: s
                .other
                .into_iter()
                .map(|x| x.inner.as_u64().into())
                .collect(),
        }
    }
}

/// A python value for various values in HEADER and TEXT as a dictionary.
#[derive(IntoPyObject)]
struct PyParseData {
    prim_text: PySegment,
    supp_text: Option<PySegment>,
    data: PySegment,
    analysis: PySegment,
    other: Vec<PySegment>,
    nextdata: Option<u32>,
    delimiter: u8,
    non_ascii_keywords: Vec<(String, String)>,
    byte_pairs: Vec<(Vec<u8>, Vec<u8>)>,
}

impl From<RawTEXTParseData> for PyParseData {
    fn from(value: RawTEXTParseData) -> Self {
        let h = value.header_segments;
        Self {
            prim_text: h.text.inner.as_u64().into(),
            supp_text: value.supp_text.map(|s| s.inner.as_u64().into()),
            data: h.data.inner.as_u64().into(),
            analysis: h.analysis.inner.as_u64().into(),
            other: h
                .other
                .into_iter()
                .map(|x| x.inner.as_u64().into())
                .collect(),
            nextdata: value.nextdata,
            delimiter: value.delimiter,
            non_ascii_keywords: value.non_ascii,
            byte_pairs: value.byte_pairs,
        }
    }
}

/// A python value for returned (pseudo)standard keywords.
#[derive(From)]
struct PyStdKeywords(StdKeywords);

impl<'py> IntoPyObject<'py> for PyStdKeywords {
    type Target = PyDict;
    type Output = Bound<'py, Self::Target>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        self.0
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .into_py_dict(py)
    }
}

/// A python value for returned (pseudo)standard keywords.
#[derive(From)]
struct PyNonStdKeywords(NonStdKeywords);

impl<'py> IntoPyObject<'py> for PyNonStdKeywords {
    type Target = PyDict;
    type Output = Bound<'py, Self::Target>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        self.0
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .into_py_dict(py)
    }
}

/// A python value for a vector of input columns from a polars dataframe.
#[derive(From)]
struct PyFCSDataFrame(FCSDataFrame);

impl From<PyFCSDataFrame> for PyDataFrame {
    fn from(value: PyFCSDataFrame) -> Self {
        let columns = value
            .0
            .iter_columns()
            .enumerate()
            .map(|(i, c)| {
                Series::from_arrow(PlSmallStr::from(format!("X{i}")), c.as_array())
                    .unwrap()
                    .into()
            })
            .collect();
        // ASSUME this will not fail because all columns should have unique
        // names and the same length
        PyDataFrame(DataFrame::new(columns).unwrap())
    }
}

/// A python value for $PnE (2.0).
///
/// This is either a unit tuple or a 2-tuple of positive floats.
#[derive(Into, From)]
struct PyScale(Scale);

impl<'py> FromPyObject<'py> for PyScale {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        if ob.is_instance_of::<PyTuple>() && ob.len()? == 0 {
            Ok(Self(Scale::Linear))
        } else {
            let (decades, offset): (f32, f32) = ob.extract()?;
            let log = Scale::try_new_log(decades, offset).map_err(PyLogRangeError)?;
            Ok(Self(log))
        }
    }
}

impl<'py> IntoPyObject<'py> for PyScale {
    type Target = PyAny;
    type Output = Bound<'py, Self::Target>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        match self.0 {
            Scale::Linear => Ok(PyTuple::empty(py).into_any()),
            Scale::Log(l) => (f32::from(l.decades), f32::from(l.offset)).into_bound_py_any(py),
        }
    }
}

/// A python value for $PnE/$PnG (2.0).
///
/// This is either a 1-tuple representing gain ($PnG) or a 2-tuple representing
/// a log transform with decades and offset respectively ($PnE). All numbers are
/// positive floats.
#[derive(Into, From)]
struct PyScaleTransform(ScaleTransform);

impl<'py> FromPyObject<'py> for PyScaleTransform {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        if let Ok(gain) = ob.extract::<PyPositiveFloat>() {
            Ok(ScaleTransform::Lin(gain.0).into())
        } else if let Ok(log) = ob.extract::<(f32, f32)>()?.try_into() {
            Ok(ScaleTransform::Log(log).into())
        } else {
            // TODO make this into a general "argument value error"
            Err(PyValueError::new_err(
                "scale transform must be a positive \
                     float or a 2-tuple of positive floats",
            ))
        }
    }
}

impl<'py> IntoPyObject<'py> for PyScaleTransform {
    type Target = PyAny;
    type Output = Bound<'py, Self::Target>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        match self.0 {
            ScaleTransform::Lin(gain) => f32::from(gain).into_bound_py_any(py),
            ScaleTransform::Log(l) => {
                (f32::from(l.decades), f32::from(l.offset)).into_bound_py_any(py)
            }
        }
    }
}

/// A python value for $UNICODE (3.0).
///
/// This is a tuple like `(u32, Vec<String>)` representing the unicode page
/// and keywords respectively.
#[derive(Into, From)]
struct PyUnicode(Unicode);

impl<'py> FromPyObject<'py> for PyUnicode {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let (page, kws): (u32, Vec<String>) = ob.extract()?;
        Ok(Self(Unicode { page, kws }))
    }
}

impl<'py> IntoPyObject<'py> for PyUnicode {
    type Target = PyTuple;
    type Output = Bound<'py, <(u32, Vec<String>) as IntoPyObject<'py>>::Target>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        (self.0.page, self.0.kws).into_pyobject(py)
    }
}

/// A python value for $PnD (3.1+)
///
/// This is a tuple like `(bool, f32, f32)` where the first boolean is `false`
/// for linear and `true` for log display. The two floats are the values for
/// either display setting (lower/upper or decades/offset).
#[derive(Into, From)]
struct PyDisplay(Display);

impl<'py> FromPyObject<'py> for PyDisplay {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let (is_log, x0, x1): (bool, f32, f32) = ob.extract()?;
        let ret = if is_log {
            Display::Log {
                offset: x0,
                decades: x1,
            }
        } else {
            Display::Lin {
                lower: x0,
                upper: x1,
            }
        };
        Ok(ret.into())
    }
}

impl<'py> IntoPyObject<'py> for PyDisplay {
    type Target = PyTuple;
    type Output = Bound<'py, <(bool, f32, f32) as IntoPyObject<'py>>::Target>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        let ret = match self.0 {
            Display::Lin { lower, upper } => (false, lower, upper),
            Display::Log { offset, decades } => (true, offset, decades),
        };
        ret.into_pyobject(py)
    }
}

/// A python value for $PnCALIBRATION (3.1).
///
/// This is a tuple like `(f32, String)` where the first float is positive and
/// represents the slope and the string represents the unit.
#[derive(Into, From)]
struct PyCalibration3_1(Calibration3_1);

impl<'py> FromPyObject<'py> for PyCalibration3_1 {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let (slope, unit): (PyPositiveFloat, String) = ob.extract()?;
        Ok(Self(Calibration3_1 {
            slope: slope.0,
            unit,
        }))
    }
}

impl<'py> IntoPyObject<'py> for PyCalibration3_1 {
    type Target = PyTuple;
    type Output = Bound<'py, <(PyPositiveFloat, String) as IntoPyObject<'py>>::Target>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        (PyPositiveFloat(self.0.slope), self.0.unit).into_pyobject(py)
    }
}

/// A python value for $PnCALIBRATION (3.2).
///
/// This is a tuple like `(f32, f32, String)` where the first float is positive
/// and represents the slope, the second float represents the intercept, and the
/// string represents the unit.
#[derive(Into, From)]
struct PyCalibration3_2(Calibration3_2);

impl<'py> FromPyObject<'py> for PyCalibration3_2 {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let (slope, offset, unit): (PyPositiveFloat, f32, String) = ob.extract()?;
        Ok(Self(Calibration3_2 {
            slope: slope.0,
            offset,
            unit,
        }))
    }
}

impl<'py> IntoPyObject<'py> for PyCalibration3_2 {
    type Target = PyTuple;
    type Output = Bound<'py, <(PyPositiveFloat, f32, String) as IntoPyObject<'py>>::Target>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        (PyPositiveFloat(self.0.slope), self.0.offset, self.0.unit).into_pyobject(py)
    }
}

/// A python value for $PnN (all versions).
#[derive(Into, From)]
struct PyShortname(Shortname);

impl<'py> FromPyObject<'py> for PyShortname {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let s: String = ob.extract()?;
        let n = s.parse().map_err(PyShortnameError)?;
        Ok(PyShortname(n))
    }
}

impl<'py> IntoPyObject<'py> for PyShortname {
    type Target = PyString;
    type Output = Bound<'py, Self::Target>;
    type Error = Infallible;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        self.0.to_string().into_pyobject(py)
    }
}

/// A python value for a non-standard keyword.
struct PyNonStdKey(NonStdKey);

impl<'py> FromPyObject<'py> for PyNonStdKey {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let s: String = ob.extract()?;
        let n = s.parse().map_err(PyKeyStringError)?;
        Ok(PyNonStdKey(n))
    }
}

/// A python value for a vector of input columns from a polars dataframe.
struct PyFCSColumns(Vec<AnyFCSColumn>);

impl<'py> FromPyObject<'py> for PyFCSColumns {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let mut df: PyDataFrame = ob.extract()?;
        df.0.rechunk_mut();
        let ret =
            df.0.take_columns()
                .into_iter()
                .map(|col| series_to_fcs(col.take_materialized_series()))
                .collect::<Result<Vec<_>, _>>()
                // TODO make better error
                .map_err(PyreflowException::new_err)?;
        Ok(Self(ret))
    }
}

/// A python value for a single input column from a polars series.
struct PyFCSColumn(AnyFCSColumn);

impl<'py> FromPyObject<'py> for PyFCSColumn {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let ser: PySeries = ob.extract()?;
        let ret = series_to_fcs(ser.0)
            // TODO make better error
            .map_err(PyreflowException::new_err)?;
        Ok(Self(ret))
    }
}

/// A python value for a vector of temporal and optical measurements (2.0/3.0)
struct PyRawMaybeInput<T, O>(RawInput<MaybeFamily, T, O>);

impl<'py, T, O> FromPyObject<'py> for PyRawMaybeInput<T, O>
where
    T: FromPyObject<'py>,
    O: FromPyObject<'py>,
{
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let xs: Vec<(Bound<'py, PyAny>, Bound<'py, PyAny>)> = ob.extract()?;
        xs.into_iter()
            .map(|(name, meas)| {
                if let Ok(t) = meas.extract::<T>() {
                    let n: PyShortname = name.extract()?;
                    Ok(Element::Center((n.0, t)))
                } else if let Ok(o) = meas.extract::<O>() {
                    let n: Option<PyShortname> = name.extract()?;
                    Ok(Element::NonCenter((n.map(|m| m.0).into(), o)))
                } else {
                    Err(PyValueError::new_err("could not parse measurement"))
                }
            })
            .collect::<Result<_, _>>()
            .map(|ret| Self(RawInput(ret)))
    }
}

/// A python value for a vector of temporal and optical measurements (3.1/3.2)
struct PyRawAlwaysInput<T, O>(RawInput<AlwaysFamily, T, O>);

impl<'py, T, O> FromPyObject<'py> for PyRawAlwaysInput<T, O>
where
    T: FromPyObject<'py>,
    O: FromPyObject<'py>,
{
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let xs: Vec<(PyShortname, Bound<'py, PyAny>)> = ob.extract()?;
        xs.into_iter()
            .map(|(name, meas)| {
                if let Ok(t) = meas.extract::<T>() {
                    Ok(Element::Center((name.0, t)))
                } else if let Ok(o) = meas.extract::<O>() {
                    Ok(Element::NonCenter((name.0.into(), o)))
                } else {
                    // TODO fix this lame error message
                    Err(PyValueError::new_err("could not parse measurement"))
                }
            })
            .collect::<Result<_, _>>()
            .map(|ret| Self(RawInput(ret)))
    }
}

/// A python value for a temporal or optical measurement
#[derive(From, Into)]
struct PyElement<T, O>(Element<T, O>);

impl<'py, T, O> FromPyObject<'py> for PyElement<T, O>
where
    T: FromPyObject<'py>,
    O: FromPyObject<'py>,
{
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        // TODO misleading error
        if let Ok(t) = ob.extract::<T>() {
            Ok(Self(Element::Center(t)))
        } else {
            let o = ob.extract::<O>()?;
            Ok(Self(Element::NonCenter(o)))
        }
    }
}

impl<'py, T, O> IntoPyObject<'py> for PyElement<T, O>
where
    T: IntoPyObject<'py>,
    O: IntoPyObject<'py>,
{
    type Target = PyAny;
    type Output = Bound<'py, Self::Target>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        match self.0 {
            Element::Center(x) => x.into_bound_py_any(py),
            Element::NonCenter(x) => x.into_bound_py_any(py),
        }
    }
}

/// A python value for any configuration parameter requiring [`Chars`].
struct PyChars(Chars);

impl<'py> FromPyObject<'py> for PyChars {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let x: u8 = ob.extract()?;
        Chars::try_from(x)
            .map_err(|e| PyValueError::new_err(e.to_string()))
            .map(Self)
    }
}

/// A python value for the shortname prefix config parameter.
struct PyShortnamePrefix(ShortnamePrefix);

impl<'py> FromPyObject<'py> for PyShortnamePrefix {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let s: String = ob.extract()?;
        let n = s.parse().map_err(PyShortnameError)?;
        Ok(Self(n))
    }
}

/// A python value for the non-standard meas pattern config parameter.
struct PyNonStdMeasPattern(NonStdMeasPattern);

impl<'py> FromPyObject<'py> for PyNonStdMeasPattern {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let s: String = ob.extract()?;
        let n = s
            .parse::<NonStdMeasPattern>()
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(Self(n))
    }
}

/// A python value for the byteord override config parameter
#[derive(Default)]
struct PyByteOrd(Option<ByteOrd2_0>);

impl<'py> FromPyObject<'py> for PyByteOrd {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let xs: Vec<NonZeroU8> = ob.extract()?;
        let ret = if xs.is_empty() {
            None
        } else {
            Some(ByteOrd2_0::try_from(&xs[..]).map_err(|e| PyValueError::new_err(e.to_string()))?)
        };
        Ok(Self(ret))
    }
}

/// A python value for the time pattern config parameter.
struct PyTimePattern(TimePattern);

impl<'py> FromPyObject<'py> for PyTimePattern {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let s: String = ob.extract()?;
        let n = s
            .parse::<TimePattern>()
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(Self(n))
    }
}

/// A python value for the date pattern config parameter.
struct PyDatePattern(DatePattern);

impl<'py> FromPyObject<'py> for PyDatePattern {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let s: String = ob.extract()?;
        let n = s
            .parse::<DatePattern>()
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(Self(n))
    }
}

/// A python value for config parameters that take [`KeyString`] pairs.
#[derive(Default)]
struct PyKeyPairs(HashMap<KeyString, KeyString>);

impl<'py> FromPyObject<'py> for PyKeyPairs {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let xs: HashMap<String, String> = ob.extract()?;
        let ret = xs
            .into_iter()
            .map(|(x, y)| {
                x.parse::<KeyString>()
                    .and_then(|a| y.parse::<KeyString>().map(|b| (a, b)))
            })
            .collect::<Result<HashMap<_, _>, _>>()
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(Self(ret))
    }
}

/// A python value for config parameters that take keystrings with strings.
#[derive(Default)]
struct PyKeyValues(HashMap<KeyString, String>);

impl<'py> FromPyObject<'py> for PyKeyValues {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let xs: HashMap<String, String> = ob.extract()?;
        let ret = xs
            .into_iter()
            .map(|(k, v)| k.parse::<KeyString>().map(|x| (x, v)))
            .collect::<Result<HashMap<_, _>, _>>()
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(Self(ret))
    }
}

/// A python value for the key patterns configuration option.
#[derive(Default)]
struct PyKeyPatterns(KeyPatterns);

impl<'py> FromPyObject<'py> for PyKeyPatterns {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let (lits, pats): (Vec<String>, Vec<String>) = ob.extract()?;
        let mut ret = KeyPatterns::try_from_literals(lits)
            .map_err(|e| PyreflowException::new_err(e.to_string()))?;
        let ps = KeyPatterns::try_from_patterns(pats)
            .map_err(|e| PyreflowException::new_err(e.to_string()))?;
        ret.extend(ps);
        Ok(Self(ret))
    }
}

/// A python value for a positive float as used in many keyword values.
#[derive(Into, From)]
#[into(Timestep, Wavelength)]
#[from(Timestep, Wavelength)]
struct PyPositiveFloat(PositiveFloat);

impl<'py> FromPyObject<'py> for PyPositiveFloat {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let x: f32 = ob.extract()?;
        let y = x.try_into().map_err(PyRangedFloatError)?;
        Ok(PyPositiveFloat(y))
    }
}

impl<'py> IntoPyObject<'py> for PyPositiveFloat {
    type Target = PyFloat;
    type Output = Bound<'py, <f32 as IntoPyObject<'py>>::Target>;
    type Error = Infallible;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        f32::from(self.0).into_pyobject(py)
    }
}

/// A python value for a non-negative float as used in many keyword values.
#[derive(Into, From)]
#[into(DetectorVoltage, Power, Vol)]
#[from(DetectorVoltage, Power, Vol)]
struct PyNonNegFloat(NonNegFloat);

impl<'py> FromPyObject<'py> for PyNonNegFloat {
    fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
        let x: f32 = ob.extract()?;
        let y = x.try_into().map_err(PyRangedFloatError)?;
        Ok(PyNonNegFloat(y))
    }
}

impl<'py> IntoPyObject<'py> for PyNonNegFloat {
    type Target = PyFloat;
    type Output = Bound<'py, <f32 as IntoPyObject<'py>>::Target>;
    type Error = Infallible;

    fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
        f32::from(self.0).into_pyobject(py)
    }
}

macro_rules! impl_pystring {
    ($outer:ident, $inner:ident) => {
        #[derive(Into, From)]
        pub(crate) struct $outer(pub(crate) $inner);

        impl<'py> FromPyObject<'py> for $outer {
            fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
                let s: String = ob.extract()?;
                s.parse()
                    .map(Self)
                    .map_err(|e| PyValueError::new_err(e.to_string()))
            }
        }

        impl<'py> IntoPyObject<'py> for $outer {
            type Target = PyString;
            type Output = Bound<'py, Self::Target>;
            type Error = Infallible;

            fn into_pyobject(self, py: Python<'py>) -> Result<Self::Output, Self::Error> {
                self.0.to_string().into_pyobject(py)
            }
        }
    };
}

impl_pystring!(PyVersion, Version);
impl_pystring!(PyOriginality, Originality);
impl_pystring!(PyAlphaNumType, AlphaNumType);
impl_pystring!(PyNumType, NumType);
impl_pystring!(PyFeature, Feature);
impl_pystring!(PyMode, Mode);
impl_pystring!(PyOpticalType, OpticalType);

fn series_to_fcs(ser: Series) -> Result<AnyFCSColumn, String> {
    fn column_to_buf<T>(ser: Series) -> Result<AnyFCSColumn, String>
    where
        T: NumericNative,
        AnyFCSColumn: From<FCSColumn<T>>,
    {
        if ser.null_count() > 0 {
            // TODO make this not a string
            Err(format!("column {} has null values", ser.name()))
        } else {
            let buf = ser.into_chunks()[0]
                .as_any()
                .downcast_ref::<PrimitiveArray<T>>()
                .unwrap()
                .values()
                .clone();
            Ok(FCSColumn(buf).into())
        }
    }

    match ser.dtype() {
        DataType::UInt8 => column_to_buf::<u8>(ser),
        DataType::UInt16 => column_to_buf::<u16>(ser),
        DataType::UInt32 => column_to_buf::<u32>(ser),
        DataType::UInt64 => column_to_buf::<u64>(ser),
        DataType::Float32 => column_to_buf::<f32>(ser),
        DataType::Float64 => column_to_buf::<f64>(ser),
        t => Err(format!("invalid datatype: {t}")),
    }
}

#[derive(Display, From)]
struct PyShortnameError(ShortnameError);

impl From<PyShortnameError> for PyErr {
    fn from(value: PyShortnameError) -> Self {
        PyreflowException::new_err(value.to_string())
    }
}

#[derive(Display, From)]
struct PyRangedFloatError(RangedFloatError);

impl From<PyRangedFloatError> for PyErr {
    fn from(value: PyRangedFloatError) -> Self {
        PyreflowException::new_err(value.to_string())
    }
}

#[derive(Display, From)]
struct PyKeyStringError(KeyStringError);

impl From<PyKeyStringError> for PyErr {
    fn from(value: PyKeyStringError) -> Self {
        PyreflowException::new_err(value.to_string())
    }
}

#[derive(Display, From)]
struct PyKeyLengthError(KeyLengthError);

impl From<PyKeyLengthError> for PyErr {
    fn from(value: PyKeyLengthError) -> Self {
        PyreflowException::new_err(value.to_string())
    }
}

#[derive(Display, From)]
struct PyReversedTimestamps(ReversedTimestamps);

impl From<PyReversedTimestamps> for PyErr {
    fn from(value: PyReversedTimestamps) -> Self {
        PyreflowException::new_err(value.to_string())
    }
}

#[derive(Display, From)]
struct PyReversedDatetimes(ReversedDatetimes);

impl From<PyReversedDatetimes> for PyErr {
    fn from(value: PyReversedDatetimes) -> Self {
        PyreflowException::new_err(value.to_string())
    }
}

#[derive(Display, From)]
struct PyLogRangeError(LogRangeError);

impl From<PyLogRangeError> for PyErr {
    fn from(value: PyLogRangeError) -> Self {
        PyreflowException::new_err(value.to_string())
    }
}

#[derive(Display, From)]
struct PyElementIndexError(ElementIndexError);

impl From<PyElementIndexError> for PyErr {
    fn from(value: PyElementIndexError) -> Self {
        PyIndexError::new_err(value.to_string())
    }
}

trait PyMultResultExt {
    type V;
    type E;

    fn py_mult_terminate<T: fmt::Display>(self, reason: T) -> PyResult<Self::V>;
}

impl<V, E: fmt::Display> PyMultResultExt for MultiResult<V, E> {
    type V = V;
    type E = E;

    fn py_mult_terminate<T: fmt::Display>(self, reason: T) -> PyResult<Self::V> {
        self.mult_to_deferred::<E, ()>()
            .py_def_terminate_nowarn(reason)
    }
}

trait PyDefResultExt {
    type V;
    type W;
    type E;

    fn py_def_terminate<T: fmt::Display>(self, reason: T) -> PyResult<Self::V>;
}

impl<V, W: fmt::Display, E: fmt::Display> PyDefResultExt for DeferredResult<V, W, E> {
    type V = V;
    type W = W;
    type E = E;

    fn py_def_terminate<T: fmt::Display>(self, reason: T) -> PyResult<Self::V> {
        self.def_terminate(reason)
            .map_or_else(|e| Err(handle_failure(e)), handle_warnings)
    }
}

trait PyDefNoWarnResultExt {
    type V;
    type E;

    fn py_def_terminate_nowarn<T: fmt::Display>(self, reason: T) -> PyResult<Self::V>;
}

impl<V, E: fmt::Display> PyDefNoWarnResultExt for DeferredResult<V, (), E> {
    type V = V;
    type E = E;

    fn py_def_terminate_nowarn<T: fmt::Display>(self, reason: T) -> PyResult<Self::V> {
        self.def_terminate(reason)
            .map_err(handle_failure_nowarn)
            .map(|x| x.inner())
    }
}

macro_rules! def_failure {
    ($failname:ident, $msg:expr) => {
        struct $failname;

        impl fmt::Display for $failname {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
                write!(f, $msg)
            }
        }
    };
}

def_failure!(ConvertFailure, "could not change FCS version");

def_failure!(
    SetTemporalFailure,
    "could not convert to/from temporal measurement"
);

def_failure!(SetLayoutFailure, "could not set data layout");

def_failure!(PushTemporalFailure, "could not push temporal measurement");

def_failure!(InsertTemporalFailure, "could not push temporal measurement");

def_failure!(PushOpticalFailure, "could not push optical measurement");

def_failure!(InsertOpticalFailure, "could not push optical measurement");

def_failure!(SetMeasurementsFailure, "could not set measurements/layout");
