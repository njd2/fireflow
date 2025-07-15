use fireflow_core::api::*;
use fireflow_core::config::*;
use fireflow_core::core::*;
use fireflow_core::error::*;
use fireflow_core::header::*;
use fireflow_core::segment::*;
use fireflow_core::text::byteord::ByteOrd2_0;
use fireflow_core::text::keywords::*;
use fireflow_core::text::named_vec::Element;
use fireflow_core::text::optional::*;
use fireflow_core::text::ranged_float::*;
use fireflow_core::text::scale::*;
use fireflow_core::validated::dataframe::*;
use fireflow_core::validated::datepattern::DatePattern;
use fireflow_core::validated::keys::*;
use fireflow_core::validated::other_width::*;
use fireflow_core::validated::shortname::*;

use super::layout::{
    PyAlphaNumType, PyDataLayout2_0, PyDataLayout3_0, PyDataLayout3_1, PyDataLayout3_2,
};
use super::macros::{py_disp, py_enum, py_eq, py_ord, py_parse, py_wrap};

use bigdecimal::BigDecimal;
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
use nonempty::NonEmpty;
use numpy::{PyArray2, PyReadonlyArray2, ToPyArray};
use polars::prelude::*;
use polars_arrow::array::PrimitiveArray;
use pyo3::class::basic::CompareOp;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyWarning};
use pyo3::prelude::*;
use pyo3::types::{IntoPyDict, PyDict, PyType};
use pyo3::IntoPyObjectExt;
use pyo3_polars::{PyDataFrame, PySeries};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::ffi::CString;
use std::fmt;
use std::path;

#[pymodule]
fn pyreflow(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("PyreflowException", py.get_type::<PyreflowException>())?;
    m.add("PyreflowWarning", py.get_type::<PyreflowWarning>())?;

    m.add_class::<PyVersion>()?;
    m.add_class::<PySegment>()?;
    m.add_class::<PyHeader>()?;

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

    m.add_class::<PyDataLayout2_0>()?;
    m.add_class::<PyDataLayout3_0>()?;
    m.add_class::<PyDataLayout3_1>()?;
    m.add_class::<PyDataLayout3_2>()?;

    m.add_class::<PyCalibration3_1>()?;
    m.add_class::<PyCalibration3_2>()?;
    m.add_class::<PyFeature>()?;
    m.add_class::<PyMode>()?;
    m.add_class::<PyOriginality>()?;
    m.add_class::<PyAlphaNumType>()?;
    m.add_class::<PyNumType>()?;
    m.add_class::<PyOpticalType>()?;
    m.add_class::<PyScale>()?;
    m.add_class::<PyDisplay>()?;
    m.add_class::<PyUnicode>()?;

    m.add_function(wrap_pyfunction!(py_fcs_read_header, m)?)?;
    m.add_function(wrap_pyfunction!(py_fcs_read_raw_text, m)?)?;
    m.add_function(wrap_pyfunction!(py_fcs_read_std_text, m)?)?;
    m.add_function(wrap_pyfunction!(py_fcs_read_std_dataset, m)?)
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
    other_width: Option<u8>,
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
    )?;
    fcs_read_header(&p, &conf)
        .map_err(handle_failure_nowarn)
        .map(|x| x.inner().into())
}

#[allow(clippy::too_many_arguments)]
#[pyfunction]
#[pyo3(
    name = "fcs_read_raw_text",
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
        promote_keys_to_standard=vec![],
        promote_patterns_to_standard=vec![],
        demote_keys_from_standard=vec![],
        demote_patterns_from_standard=vec![],
        ignore_literal_standard_keys=vec![],
        ignore_pattern_standard_keys=vec![],
        rename_standard_keys=vec![],
        replace_standard_key_values=vec![],
        append_standard_keywords=vec![],
        warnings_are_errors=false,
    )
)]
fn py_fcs_read_raw_text(
    p: path::PathBuf,

    py: Python<'_>,

    version_override: Option<PyVersion>,
    prim_text_correction: (i32, i32),
    data_correction: (i32, i32),
    analysis_correction: (i32, i32),
    other_corrections: Vec<(i32, i32)>,
    other_width: Option<u8>,
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
    date_pattern: Option<String>,
    promote_keys_to_standard: Vec<String>,
    promote_patterns_to_standard: Vec<String>,
    demote_keys_from_standard: Vec<String>,
    demote_patterns_from_standard: Vec<String>,
    ignore_literal_standard_keys: Vec<String>,
    ignore_pattern_standard_keys: Vec<String>,
    rename_standard_keys: Vec<(String, String)>,
    replace_standard_key_values: Vec<(String, String)>,
    append_standard_keywords: Vec<(String, String)>,
    warnings_are_errors: bool,
) -> PyResult<(PyVersion, Bound<'_, PyDict>, Bound<'_, PyDict>, PyParseData)> {
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
    )?;

    let conf = raw_config(
        header,
        supp_text_correction,
        use_literal_delims,
        allow_non_ascii_delim,
        ignore_supp_text,
        ignore_text_data_offsets,
        ignore_text_analysis_offsets,
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
        promote_keys_to_standard,
        promote_patterns_to_standard,
        demote_keys_from_standard,
        demote_patterns_from_standard,
        ignore_literal_standard_keys,
        ignore_pattern_standard_keys,
        rename_standard_keys,
        replace_standard_key_values,
        append_standard_keywords,
        warnings_are_errors,
    )?;

    let raw: RawTEXTOutput =
        fcs_read_raw_text(&p, &conf).map_or_else(|e| Err(handle_failure(e)), handle_warnings)?;
    let std = raw
        .keywords
        .std
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .into_py_dict(py)?;
    let nonstd = raw
        .keywords
        .nonstd
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .into_py_dict(py)?;
    Ok((raw.version.into(), std, nonstd, raw.parse.into()))
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
        promote_keys_to_standard=vec![],
        promote_patterns_to_standard=vec![],
        demote_keys_from_standard=vec![],
        demote_patterns_from_standard=vec![],
        ignore_literal_standard_keys=vec![],
        ignore_pattern_standard_keys=vec![],
        rename_standard_keys=vec![],
        replace_standard_key_values=vec![],
        append_standard_keywords=vec![],
        warnings_are_errors=false,

        disallow_deprecated=false,
        time_ensure=false,
        allow_pseudostandard=false,
        fix_log_scale_offsets=false,
        shortname_prefix=None,
        allow_header_text_offset_mismatch=false,
        allow_missing_required_offsets=false,
        text_data_correction=(0,0),
        text_analysis_correction=(0,0),
        disallow_range_truncation=false,
        nonstandard_measurement_pattern=None,
        time_pattern=None,
        integer_widths_from_byteord=false,
        integer_byteord_override=vec![],
    )
)]
fn py_fcs_read_std_text(
    py: Python<'_>,

    p: path::PathBuf,

    version_override: Option<PyVersion>,
    prim_text_correction: (i32, i32),
    data_correction: (i32, i32),
    analysis_correction: (i32, i32),
    other_corrections: Vec<(i32, i32)>,
    other_width: Option<u8>,
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
    date_pattern: Option<String>,
    promote_keys_to_standard: Vec<String>,
    promote_patterns_to_standard: Vec<String>,
    demote_keys_from_standard: Vec<String>,
    demote_patterns_from_standard: Vec<String>,
    ignore_literal_standard_keys: Vec<String>,
    ignore_pattern_standard_keys: Vec<String>,
    rename_standard_keys: Vec<(String, String)>,
    replace_standard_key_values: Vec<(String, String)>,
    append_standard_keywords: Vec<(String, String)>,
    warnings_are_errors: bool,

    disallow_deprecated: bool,
    time_ensure: bool,
    allow_pseudostandard: bool,
    fix_log_scale_offsets: bool,
    shortname_prefix: Option<String>,
    allow_header_text_offset_mismatch: bool,
    allow_missing_required_offsets: bool,
    text_data_correction: (i32, i32),
    text_analysis_correction: (i32, i32),
    disallow_range_truncation: bool,
    nonstandard_measurement_pattern: Option<String>,
    time_pattern: Option<String>,
    integer_widths_from_byteord: bool,
    integer_byteord_override: Vec<u8>,
) -> PyResult<(Bound<'_, PyAny>, PyParseData, Bound<'_, PyDict>)> {
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
    )?;

    let raw = raw_config(
        header,
        supp_text_correction,
        use_literal_delims,
        allow_non_ascii_delim,
        ignore_supp_text,
        ignore_text_data_offsets,
        ignore_text_analysis_offsets,
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
        promote_keys_to_standard,
        promote_patterns_to_standard,
        demote_keys_from_standard,
        demote_patterns_from_standard,
        ignore_literal_standard_keys,
        ignore_pattern_standard_keys,
        rename_standard_keys,
        replace_standard_key_values,
        append_standard_keywords,
        warnings_are_errors,
    )?;

    let conf = std_config(
        raw,
        disallow_deprecated,
        time_ensure,
        allow_pseudostandard,
        fix_log_scale_offsets,
        shortname_prefix,
        allow_header_text_offset_mismatch,
        allow_missing_required_offsets,
        text_data_correction,
        text_analysis_correction,
        disallow_range_truncation,
        nonstandard_measurement_pattern,
        time_pattern,
        integer_widths_from_byteord,
        integer_byteord_override,
    )?;

    let out: StdTEXTOutput =
        fcs_read_std_text(&p, &conf).map_or_else(|e| Err(handle_failure(e)), handle_warnings)?;

    let text = match &out.standardized {
        // TODO this copies all data from the "union type" into a new
        // version-specific type. This might not be a big deal, but these
        // types might be rather large with lots of strings.
        AnyCoreTEXT::FCS2_0(x) => PyCoreTEXT2_0::from((**x).clone()).into_bound_py_any(py),
        AnyCoreTEXT::FCS3_0(x) => PyCoreTEXT3_0::from((**x).clone()).into_bound_py_any(py),
        AnyCoreTEXT::FCS3_1(x) => PyCoreTEXT3_1::from((**x).clone()).into_bound_py_any(py),
        AnyCoreTEXT::FCS3_2(x) => PyCoreTEXT3_2::from((**x).clone()).into_bound_py_any(py),
    }?;

    Ok((
        text,
        out.parse.into(),
        out.pseudostandard
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .into_py_dict(py)?,
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
        promote_keys_to_standard=vec![],
        promote_patterns_to_standard=vec![],
        demote_keys_from_standard=vec![],
        demote_patterns_from_standard=vec![],
        ignore_literal_standard_keys=vec![],
        ignore_pattern_standard_keys=vec![],
        rename_standard_keys=vec![],
        replace_standard_key_values=vec![],
        append_standard_keywords=vec![],
        warnings_are_errors=false,

        disallow_deprecated=false,
        time_ensure=false,
        allow_pseudostandard=false,
        fix_log_scale_offsets=false,
        shortname_prefix=None,
        allow_header_text_offset_mismatch=false,
        allow_missing_required_offsets=false,
        text_data_correction=(0,0),
        text_analysis_correction=(0,0),
        disallow_range_truncation=false,
        nonstandard_measurement_pattern=None,
        time_pattern=None,
        integer_widths_from_byteord=false,
        integer_byteord_override=vec![],

        allow_uneven_event_width=false,
        allow_tot_mismatch=false,
    )
)]
fn py_fcs_read_std_dataset(
    py: Python<'_>,

    p: path::PathBuf,

    version_override: Option<PyVersion>,
    prim_text_correction: (i32, i32),
    data_correction: (i32, i32),
    analysis_correction: (i32, i32),
    other_corrections: Vec<(i32, i32)>,
    other_width: Option<u8>,
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
    date_pattern: Option<String>,
    promote_keys_to_standard: Vec<String>,
    promote_patterns_to_standard: Vec<String>,
    demote_keys_from_standard: Vec<String>,
    demote_patterns_from_standard: Vec<String>,
    ignore_literal_standard_keys: Vec<String>,
    ignore_pattern_standard_keys: Vec<String>,
    rename_standard_keys: Vec<(String, String)>,
    replace_standard_key_values: Vec<(String, String)>,
    append_standard_keywords: Vec<(String, String)>,
    warnings_are_errors: bool,

    disallow_deprecated: bool,
    time_ensure: bool,
    allow_pseudostandard: bool,
    fix_log_scale_offsets: bool,
    shortname_prefix: Option<String>,
    allow_header_text_offset_mismatch: bool,
    allow_missing_required_offsets: bool,
    text_data_correction: (i32, i32),
    text_analysis_correction: (i32, i32),
    disallow_range_truncation: bool,
    nonstandard_measurement_pattern: Option<String>,
    time_pattern: Option<String>,
    integer_widths_from_byteord: bool,
    integer_byteord_override: Vec<u8>,

    allow_uneven_event_width: bool,
    allow_tot_mismatch: bool,
) -> PyResult<(Bound<'_, PyAny>, PyParseData, Bound<'_, PyDict>)> {
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
    )?;

    let raw = raw_config(
        header,
        supp_text_correction,
        use_literal_delims,
        allow_non_ascii_delim,
        ignore_supp_text,
        ignore_text_data_offsets,
        ignore_text_analysis_offsets,
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
        promote_keys_to_standard,
        promote_patterns_to_standard,
        demote_keys_from_standard,
        demote_patterns_from_standard,
        ignore_literal_standard_keys,
        ignore_pattern_standard_keys,
        rename_standard_keys,
        replace_standard_key_values,
        append_standard_keywords,
        warnings_are_errors,
    )?;

    let standard = std_config(
        raw,
        disallow_deprecated,
        time_ensure,
        allow_pseudostandard,
        fix_log_scale_offsets,
        shortname_prefix,
        allow_header_text_offset_mismatch,
        allow_missing_required_offsets,
        text_data_correction,
        text_analysis_correction,
        disallow_range_truncation,
        nonstandard_measurement_pattern,
        time_pattern,
        integer_widths_from_byteord,
        integer_byteord_override,
    )?;

    let conf = data_config(standard, allow_uneven_event_width, allow_tot_mismatch);

    let out: StdDatasetOutput =
        fcs_read_std_dataset(&p, &conf).map_or_else(|e| Err(handle_failure(e)), handle_warnings)?;

    let dataset = match &out.dataset.standardized.core {
        // TODO this copies all data from the "union type" into a new
        // version-specific type. This might not be a big deal, but these
        // types might be rather large with lots of strings.
        AnyCoreDataset::FCS2_0(x) => PyCoreDataset2_0::from((**x).clone()).into_bound_py_any(py),
        AnyCoreDataset::FCS3_0(x) => PyCoreDataset3_0::from((**x).clone()).into_bound_py_any(py),
        AnyCoreDataset::FCS3_1(x) => PyCoreDataset3_1::from((**x).clone()).into_bound_py_any(py),
        AnyCoreDataset::FCS3_2(x) => PyCoreDataset3_2::from((**x).clone()).into_bound_py_any(py),
    }?;

    Ok((
        dataset,
        out.parse.into(),
        out.dataset
            .pseudostandard
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .into_py_dict(py)?,
    ))
}

#[allow(clippy::too_many_arguments)]
fn header_config(
    version_override: Option<PyVersion>,
    prim_text_correction: (i32, i32),
    data_correction: (i32, i32),
    analysis_correction: (i32, i32),
    other_corrections: Vec<(i32, i32)>,
    other_width: Option<u8>,
    max_other: Option<usize>,
    allow_negative: bool,
    squish_offsets: bool,
    truncate_offsets: bool,
) -> PyResult<HeaderConfig> {
    let os = other_corrections
        .into_iter()
        .map(OffsetCorrection::from)
        .collect();
    let ow = other_width
        .map(OtherWidth::try_from)
        .transpose()
        .map_err(|e| PyreflowException::new_err(e.to_string()))?
        .unwrap_or_default();
    let out = HeaderConfig {
        version_override: version_override.map(|x| x.0),
        text_correction: OffsetCorrection::from(prim_text_correction),
        data_correction: OffsetCorrection::from(data_correction),
        analysis_correction: OffsetCorrection::from(analysis_correction),
        other_corrections: os,
        other_width: ow,
        max_other,
        allow_negative,
        squish_offsets,
        truncate_offsets,
    };
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn raw_config(
    header: HeaderConfig,
    supp_text_correction: (i32, i32),
    use_literal_delims: bool,
    allow_non_ascii_delim: bool,
    ignore_supp_text: bool,
    ignore_text_data_offsets: bool,
    ignore_text_analysis_offsets: bool,
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
    date_pattern: Option<String>,
    promote_keys_to_standard: Vec<String>,
    promote_patterns_to_standard: Vec<String>,
    demote_keys_from_standard: Vec<String>,
    demote_patterns_from_standard: Vec<String>,
    ignore_literal_standard_keys: Vec<String>,
    ignore_pattern_standard_keys: Vec<String>,
    rename_standard_keys: Vec<(String, String)>,
    replace_standard_key_values: Vec<(String, String)>,
    append_standard_keywords: Vec<(String, String)>,
    warnings_are_errors: bool,
) -> PyResult<RawTextReadConfig> {
    let pss = strs_to_key_patterns(promote_keys_to_standard, promote_patterns_to_standard)?;
    let dss = strs_to_key_patterns(demote_keys_from_standard, demote_patterns_from_standard)?;
    let iss = strs_to_key_patterns(ignore_literal_standard_keys, ignore_pattern_standard_keys)?;

    let rss = rename_standard_keys
        .into_iter()
        .map(|(x, y)| {
            x.parse::<KeyString>()
                .and_then(|a| y.parse::<KeyString>().map(|b| (a, b)))
        })
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(|e| PyreflowException::new_err(e.to_string()))?;

    let rvs = replace_standard_key_values
        .into_iter()
        .map(|(k, v)| k.parse::<KeyString>().map(|x| (x, v)))
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(|e| PyreflowException::new_err(e.to_string()))?;

    let ass = append_standard_keywords
        .into_iter()
        .map(|(k, v)| k.parse::<KeyString>().map(|x| (x, v)))
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(|e| PyreflowException::new_err(e.to_string()))?;

    let out = RawTextReadConfig {
        header,
        supp_text_correction: OffsetCorrection::from(supp_text_correction),
        use_literal_delims,
        ignore_supp_text,
        ignore_text_data_offsets,
        ignore_text_analysis_offsets,
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
        date_pattern: date_pattern.map(str_to_date_pat).transpose()?,
        promote_to_standard: pss,
        demote_from_standard: dss,
        ignore_standard_keys: iss,
        rename_standard_keys: rss,
        replace_standard_key_values: rvs,
        append_standard_keywords: ass,
        warnings_are_errors,
    };
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn std_config(
    raw: RawTextReadConfig,
    disallow_deprecated: bool,
    time_ensure: bool,
    allow_pseudostandard: bool,
    fix_log_scale_offsets: bool,
    shortname_prefix: Option<String>,
    allow_header_text_offset_mismatch: bool,
    allow_missing_required_offsets: bool,
    text_data_correction: (i32, i32),
    text_analysis_correction: (i32, i32),
    disallow_range_truncation: bool,
    nonstandard_measurement_pattern: Option<String>,
    time_pattern: Option<String>,
    integer_widths_from_byteord: bool,
    integer_byteord_override: Vec<u8>,
) -> PyResult<StdTextReadConfig> {
    let sp = shortname_prefix.map(str_to_shortname_prefix).transpose()?;
    let nsmp = nonstandard_measurement_pattern
        .map(str_to_nonstd_meas_pat)
        .transpose()?;
    let tp = time_pattern.map(str_to_time_pat).transpose()?;
    let xs = &integer_byteord_override[..];
    let bo = if xs.is_empty() {
        None
    } else {
        Some(ByteOrd2_0::try_from(xs).map_err(|e| PyreflowException::new_err(e.to_string()))?)
    };

    let out = StdTextReadConfig {
        raw,
        shortname_prefix: sp.unwrap_or_default(),
        time: TimeConfig {
            pattern: tp,
            allow_missing: time_ensure,
            // allow_nonlinear_scale: time_ensure_linear,
            // allow_nontime_keywords: time_ensure_nogain,
        },
        allow_pseudostandard,
        fix_log_scale_offsets,
        disallow_deprecated,
        allow_header_text_offset_mismatch,
        allow_missing_required_offsets,
        data: OffsetCorrection::from(text_data_correction),
        analysis: OffsetCorrection::from(text_analysis_correction),
        disallow_range_truncation,
        nonstandard_measurement_pattern: nsmp,
        integer_widths_from_byteord,
        integer_byteord_override: bo,
    };
    Ok(out)
}

fn data_config(
    standard: StdTextReadConfig,
    allow_uneven_event_width: bool,
    allow_tot_mismatch: bool,
) -> DataReadConfig {
    DataReadConfig {
        standard,
        reader: ReaderConfig {
            allow_uneven_event_width,
            allow_tot_mismatch,
        },
    }
}

py_wrap!(PyVersion, Version, "Version");
py_ord!(PyVersion);
py_disp!(PyVersion);
py_enum!(
    PyVersion,
    Version,
    [FCS2_0, FCS2_0],
    [FCS3_0, FCS3_0],
    [FCS3_1, FCS3_1],
    [FCS3_2, FCS3_2]
);

py_wrap!(PySegment, Segment<u64>, "Segment");

#[pymethods]
impl PySegment {
    fn coords(&self) -> (u64, u64) {
        self.0.try_coords().unwrap_or((0, 0))
    }

    fn nbytes(&self) -> u64 {
        self.0.len()
    }

    fn __repr__(&self) -> String {
        format!("({})", self.0.fmt_pair())
    }
}

py_wrap!(PyHeader, Header, "Header");

#[pymethods]
impl PyHeader {
    #[getter]
    fn version(&self) -> PyVersion {
        self.0.version.into()
    }

    #[getter]
    fn text(&self) -> PySegment {
        self.0.segments.text.inner.as_u64().into()
    }

    #[getter]
    fn data(&self) -> PySegment {
        self.0.segments.data.inner.as_u64().into()
    }

    #[getter]
    fn analysis(&self) -> PySegment {
        self.0.segments.analysis.inner.as_u64().into()
    }

    #[getter]
    fn other(&self) -> Vec<PySegment> {
        self.0
            .segments
            .other
            .iter()
            .copied()
            .map(|x| x.inner.as_u64().into())
            .collect()
    }
}

py_wrap!(PyParseData, RawTEXTParseData, "ParseData");

#[pymethods]
impl PyParseData {
    #[getter]
    fn prim_text(&self) -> PySegment {
        self.0.header_segments.text.inner.as_u64().into()
    }

    #[getter]
    fn supp_text(&self) -> Option<PySegment> {
        self.0.supp_text.map(|x| x.inner.as_u64().into())
    }

    #[getter]
    fn data(&self) -> PySegment {
        self.0.header_segments.data.inner.as_u64().into()
    }

    #[getter]
    fn analysis(&self) -> PySegment {
        self.0.header_segments.analysis.inner.as_u64().into()
    }

    #[getter]
    fn nextdata(&self) -> Option<u32> {
        self.0.nextdata
    }

    #[getter]
    fn delimiter(&self) -> u8 {
        self.0.delimiter
    }

    #[getter]
    fn non_ascii_keywords(&self) -> Vec<(String, String)> {
        self.0.non_ascii.clone()
    }

    #[getter]
    fn byte_pairs(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.0.byte_pairs.clone()
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

py_wrap!(PyOptical2_0, Optical2_0, "Optical2_0");
py_wrap!(PyOptical3_0, Optical3_0, "Optical3_0");
py_wrap!(PyOptical3_1, Optical3_1, "Optical3_1");
py_wrap!(PyOptical3_2, Optical3_2, "Optical3_2");

py_wrap!(PyTemporal2_0, Temporal2_0, "Temporal2_0");
py_wrap!(PyTemporal3_0, Temporal3_0, "Temporal3_0");
py_wrap!(PyTemporal3_1, Temporal3_1, "Temporal3_1");
py_wrap!(PyTemporal3_2, Temporal3_2, "Temporal3_2");

// PnCALIBRATION (3.1)
py_wrap!(PyCalibration3_1, Calibration3_1, "Calibration3_1");
py_eq!(PyCalibration3_1);
py_disp!(PyCalibration3_1);
py_parse!(PyCalibration3_1);

#[pymethods]
impl PyCalibration3_1 {
    #[new]
    fn new(slope: f32, unit: String) -> PyResult<Self> {
        let ret = Calibration3_1 {
            slope: f32_to_positive_float(slope)?,
            unit,
        };
        Ok(ret.into())
    }

    #[getter]
    fn get_slope(&self) -> f32 {
        self.0.slope.into()
    }

    #[getter]
    fn get_unit(&self) -> String {
        self.0.unit.clone()
    }

    #[setter]
    fn set_slope(&mut self, slope: f32) -> PyResult<()> {
        self.0.slope = f32_to_positive_float(slope)?;
        Ok(())
    }

    #[setter]
    fn set_unit(&mut self, unit: String) {
        self.0.unit = unit;
    }
}

// PnCALIBRATION (3.2)
py_wrap!(PyCalibration3_2, Calibration3_2, "Calibration3_2");
py_eq!(PyCalibration3_2);
py_disp!(PyCalibration3_2);
py_parse!(PyCalibration3_2);

#[pymethods]
impl PyCalibration3_2 {
    #[new]
    fn new(slope: f32, offset: f32, unit: String) -> PyResult<Self> {
        let ret = Calibration3_2 {
            slope: f32_to_positive_float(slope)?,
            offset,
            unit,
        };
        Ok(ret.into())
    }

    #[getter]
    fn get_slope(&self) -> f32 {
        self.0.slope.into()
    }

    #[getter]
    fn get_offset(&self) -> f32 {
        self.0.offset
    }

    #[getter]
    fn get_unit(&self) -> String {
        self.0.unit.clone()
    }

    #[setter]
    fn set_slope(&mut self, slope: f32) -> PyResult<()> {
        self.0.slope = f32_to_positive_float(slope)?;
        Ok(())
    }

    #[setter]
    fn set_offset(&mut self, offset: f32) {
        self.0.offset = offset;
    }

    #[setter]
    fn set_unit(&mut self, unit: String) {
        self.0.unit = unit;
    }
}

// $PnFEATURE (3.2)
py_wrap!(PyFeature, Feature, "Feature");
py_eq!(PyFeature);
py_disp!(PyFeature);
py_parse!(PyFeature);
py_enum!(
    PyFeature,
    Feature,
    [Area, AREA],
    [Width, WIDTH],
    [Height, HEIGHT]
);

// $MODE (2.0-3.0)
py_wrap!(PyMode, Mode, "Mode");
py_eq!(PyMode);
py_disp!(PyMode);
py_parse!(PyMode);
py_enum!(
    PyMode,
    Mode,
    [List, LIST],
    [Uncorrelated, UNCORRELATED],
    [Correlated, CORRELATED]
);

// $ORIGINALITY (3.1-3.2)
py_wrap!(PyOriginality, Originality, "Originality");
py_eq!(PyOriginality);
py_disp!(PyOriginality);
py_parse!(PyOriginality);
py_enum!(
    PyOriginality,
    Originality,
    [Original, ORIGINAL],
    [NonDataModified, NON_DATA_MODIFIED],
    [Appended, APPENDED],
    [DataModified, DATA_MODIFIED]
);

// $PnDATATYPE (3.2)
py_wrap!(PyNumType, NumType, "NumType");
py_eq!(PyNumType);
py_disp!(PyNumType);
py_parse!(PyNumType);
py_enum!(
    PyNumType,
    NumType,
    [Integer, INTEGER],
    [Single, SINGLE],
    [Double, DOUBLE]
);

// $PnTYPE (3.2)
py_wrap!(PyOpticalType, OpticalType, "OpticalType");
py_eq!(PyOpticalType);
py_disp!(PyOpticalType);
py_parse!(PyOpticalType);
py_enum!(
    PyOpticalType,
    OpticalType,
    [ForwardScatter, FORWARD_SCATTER],
    [SideScatter, SIDE_SCATTER],
    [RawFluorescence, RAW_FLUORESCENCE],
    [UnmixedFluorescence, UNMIXED_FLUORESCENCE],
    [Mass, MASS],
    [ElectronicVolume, ELECTRONIC_VOLUME],
    [Classification, CLASSIFICATION],
    [Index, INDEX]
);

#[pymethods]
impl PyOpticalType {
    #[classmethod]
    fn other(_: &Bound<'_, PyType>, s: String) -> Self {
        OpticalType::Other(s).into()
    }
}

// $PnE (all versions)
py_wrap!(PyScale, Scale, "Scale");
py_eq!(PyScale);
py_disp!(PyScale);
py_parse!(PyScale);

#[pymethods]
impl PyScale {
    #[classattr]
    #[allow(non_snake_case)]
    fn LINEAR() -> Self {
        Scale::Linear.into()
    }

    #[classmethod]
    fn log(_: &Bound<'_, PyType>, decades: f32, offset: f32) -> PyResult<Self> {
        Scale::try_new_log(decades, offset)
            .map_err(|e| PyreflowException::new_err(e.to_string()))
            .map(|x| x.into())
    }

    fn is_linear(&self) -> bool {
        self.0 == Scale::Linear
    }
}

py_wrap!(PyDisplay, Display, "Display");
py_eq!(PyDisplay);
py_disp!(PyDisplay);
py_parse!(PyDisplay);

#[pymethods]
impl PyDisplay {
    #[classmethod]
    fn lin(_: &Bound<'_, PyType>, lower: f32, upper: f32) -> Self {
        Display::Lin { lower, upper }.into()
    }

    #[classmethod]
    fn log(_: &Bound<'_, PyType>, decades: f32, offset: f32) -> Self {
        Display::Log { offset, decades }.into()
    }

    fn is_linear(&self) -> bool {
        matches!(self.0, Display::Lin { lower: _, upper: _ })
    }
}

// $UNICODE (3.0)
py_wrap!(PyUnicode, Unicode, "Unicode");
py_eq!(PyUnicode);
py_disp!(PyUnicode);
py_parse!(PyUnicode);

#[pymethods]
impl PyUnicode {
    #[new]
    fn new(page: u32, kws: Vec<String>) -> Self {
        Unicode { page, kws }.into()
    }
}

macro_rules! get_set_cloned {
    ($pytype:ident, $($rest:ident,)+ [$($root:ident),*], $get:ident, $set:ident, $field:ident, $out:path) => {
        get_set_cloned!($pytype, [$($root),*], $get, $set, $field, $out);
        get_set_cloned!($($rest,)+ [$($root),*], $get, $set, $field, $out);
    };

    ($pytype:ident, [$($root:ident),*], $get:ident, $set:ident, $field:ident, $out:path) => {
        #[pymethods]
        impl $pytype {
            #[getter]
            fn $get(&self) -> Option<$out> {
                self.0.$($root.)*$field.as_ref_opt().map(|x| x.clone().into())
            }

            #[setter]
            fn $set(&mut self, s: Option<$out>) {
                self.0.$($root.)*$field = s.map(|x| x.into()).into()
            }
        }
    };
}

macro_rules! get_set_str {
    ($($pytype:ident,)* [$($root:ident),*], $get:ident, $set:ident, $field:ident) => {
        get_set_cloned!($($pytype,)* [$($root),*], $get, $set, $field, String);
    };
}

macro_rules! get_set_copied {
    ($pytype:ident, $($rest:ident,)+ [$($root:ident),*], $get:ident, $set:ident, $field:ident, $out:ty) => {
        get_set_copied!($pytype, [$($root),*], $get, $set, $field, $out);
        get_set_copied!($($rest,)+ [$($root),*], $get, $set, $field, $out);
    };

    ($pytype:ident, [$($root:ident),*], $get:ident, $set:ident, $field:ident, $out:ty) => {
        #[pymethods]
        impl $pytype {
            #[getter]
            fn $get(&self) -> Option<$out> {
                self.0.$($root.)*$field.0.map(|x| x.into())
            }

            #[setter]
            fn $set(&mut self, s: Option<$out>) {
                self.0.$($root.)*$field = s.map(|x| x.into()).into()
            }
        }
    };

    ($($pytype:ident),*; $get:ident, $set:ident, $out:ty) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn $get(&self) -> Option<$out> {
                    self.0.$get().map(|x| (*x).into())
                }

                #[setter]
                fn $set(&mut self, s: Option<$out>) {
                    self.0.$set(s.map(|x| x.into()).into())
                }
            }
        )*
    };
}

macro_rules! meas_get_set {
    ($get:ident, $set:ident, $t:path, $($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn $get(&self) -> Vec<(usize, Option<$t>)> {
                    self.0
                        .$get()
                        .into_iter()
                        .map(|(i, x)| (i.into(), x.map(|y| y.clone().into())))
                        .collect()
                }

                #[setter]
                fn $set(&mut self, xs: Vec<Option<$t>>) -> PyResult<()> {
                    self.0
                        .$set(xs.into_iter().map(|x| x.map(|y| y.into())).collect())
                        .map_err(|e| PyreflowException::new_err(e.to_string()))
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
                    new.def_map_value(|x| x.into())
                        .def_terminate(ConvertFailure)
                        .map_or_else(|e| Err(handle_failure(e)), handle_warnings)
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
    fn get_datetime_begin(&self) -> Option<DateTime<FixedOffset>> {
        self.0.metaroot.specific.datetimes.begin_naive()
    }

    #[setter]
    fn set_datetime_begin(&mut self, x: Option<DateTime<FixedOffset>>) -> PyResult<()> {
        self.0
            .metaroot
            .specific
            .datetimes
            .set_begin_naive(x)
            .map_err(|e| PyreflowException::new_err(e.to_string()))
    }

    #[getter]
    fn get_datetime_end(&self) -> Option<DateTime<FixedOffset>> {
        self.0.metaroot.specific.datetimes.end_naive()
    }

    #[setter]
    fn set_datetime_end(&mut self, x: Option<DateTime<FixedOffset>>) -> PyResult<()> {
        self.0
            .metaroot
            .specific
            .datetimes
            .set_end_naive(x)
            .map_err(|e| PyreflowException::new_err(e.to_string()))
    }

    // #[getter]
    // fn get_datatypes(&self) -> Vec<PyAlphaNumType> {
    //     self.0.datatypes().into_iter().map(|x| x.into()).collect()
    // }

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
        self.0.unstained_centers().map(|x| {
            <HashMap<Shortname, f32>>::from(x.clone())
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect()
        })
    }

    fn insert_unstained_center(&mut self, k: String, v: f32) -> PyResult<Option<f32>> {
        let n = str_to_shortname(k)?;
        self.0
            .insert_unstained_center(n, v)
            .map_err(|e| PyreflowException::new_err(e.to_string()))
    }

    fn remove_unstained_center(&mut self, k: String) -> PyResult<Option<f32>> {
        let n = str_to_shortname(k)?;
        Ok(self.0.remove_unstained_center(&n))
    }

    fn clear_unstained_centers(&mut self) {
        self.0.clear_unstained_centers()
    }

    // fn set_data_mixed(&mut self, cs: Vec<PyMixedColumnSetter>) -> PyResult<()> {
    //     self.0
    //         .set_data_mixed(cs.into_iter().map(|x| x.into()).collect())
    //         .map_err(|e| PyreflowException::new_err(e.to_string()))
    // }
}

// Get/set methods for all versions
macro_rules! common_methods {
    ($pytype:ident, $($rest:ident),*) => {
        common_methods!($pytype);
        common_methods!($($rest),+);

    };

    ($pytype:ident) => {
        // common measurement keywords
        meas_get_set!(filters,           set_filters,           String,        $pytype);
        meas_get_set!(detector_types,    set_detector_types,    String,        $pytype);
        meas_get_set!(percents_emitted,  set_percents_emitted,  String,        $pytype);

        get_set_copied!($pytype, [metaroot], get_abrt, set_abrt, abrt, u32);
        get_set_copied!($pytype, [metaroot], get_lost, set_lost, lost, u32);

        get_set_str!($pytype, [metaroot], get_cells, set_cells, cells);
        get_set_str!($pytype, [metaroot], get_com,   set_com,   com);
        get_set_str!($pytype, [metaroot], get_exp,   set_exp,   exp);
        get_set_str!($pytype, [metaroot], get_fil,   set_fil,   fil);
        get_set_str!($pytype, [metaroot], get_inst,  set_inst,  inst);
        get_set_str!($pytype, [metaroot], get_op,    set_op,    op);
        get_set_str!($pytype, [metaroot], get_proj,  set_proj,  proj);
        get_set_str!($pytype, [metaroot], get_smno,  set_smno,  smno);
        get_set_str!($pytype, [metaroot], get_src,   set_src,   src);
        get_set_str!($pytype, [metaroot], get_sys,   set_sys,   sys);

        #[pymethods]
        impl $pytype {
            fn insert_nonstandard(&mut self, key: String, v: String) -> PyResult<Option<String>> {
                let k = str_to_nonstd_key(key)?;
                Ok(self.0.metaroot.nonstandard_keywords.insert(k, v))
            }

            fn remove_nonstandard(&mut self, key: String) -> PyResult<Option<String>> {
                let k = str_to_nonstd_key(key)?;
                Ok(self.0.metaroot.nonstandard_keywords.remove(&k.into()))
            }

            fn get_nonstandard(&mut self, key: String) -> PyResult<Option<String>> {
                let k = str_to_nonstd_key(key)?;
                Ok(self.0.metaroot.nonstandard_keywords.get(&k.into()).cloned())
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
                keyvals: Vec<(String, String)>,
            ) -> PyResult<Vec<Option<String>>> {
                let mut xs = vec![];
                for (k, v) in keyvals {
                    xs.push((str_to_nonstd_key(k)?, v));
                }
                self.0
                    .insert_meas_nonstandard(xs)
                    .map_err(|e| PyreflowException::new_err(e.to_string()))

            }

            fn remove_meas_nonstandard(
                &mut self,
                keys: Vec<String>
            ) -> PyResult<Vec<Option<String>>> {
                let mut xs = vec![];
                for k in keys {
                    xs.push(str_to_nonstd_key(k)?);
                }
                self.0
                    .remove_meas_nonstandard(xs.iter().collect())
                    .map_err(|e| PyreflowException::new_err(e.to_string()))
            }

            fn get_meas_nonstandard(
                &mut self,
                keys: Vec<String>
            ) -> PyResult<Option<Vec<Option<String>>>> {
                let mut xs = vec![];
                for k in keys {
                    xs.push(str_to_nonstd_key(k)?);
                }
                let res = self.0
                    .get_meas_nonstandard(&xs[..])
                    .map(|rs| rs.into_iter().map(|r| r.cloned()).collect());
                Ok(res)
            }

            #[getter]
            fn get_btim(&self) -> Option<NaiveTime> {
                self.0.timestamps().btim_naive()
            }

            #[setter]
            fn set_btim(&mut self, x: Option<NaiveTime>) -> PyResult<()> {
                self.0
                    .timestamps_mut()
                    .set_btim_naive(x)
                    .map_err(|e| PyreflowException::new_err(e.to_string()))
            }

            #[getter]
            fn get_etim(&self) -> Option<NaiveTime> {
                self.0.timestamps().etim_naive()
            }

            #[setter]
            fn set_etim(&mut self, x: Option<NaiveTime>) -> PyResult<()> {
                self.0
                    .timestamps_mut()
                    .set_etim_naive(x)
                    .map_err(|e| PyreflowException::new_err(e.to_string()))
            }

            #[getter]
            fn get_date(&self) -> Option<NaiveDate> {
                self.0.timestamps().date_naive()
            }

            #[setter]
            fn set_date(&mut self, x: Option<NaiveDate>) -> PyResult<()> {
                self.0
                    .timestamps_mut()
                    .set_date_naive(x)
                    .map_err(|e| PyreflowException::new_err(e.to_string()))
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
            fn set_trigger_name(&mut self, name: String) -> PyResult<bool> {
                let n = str_to_shortname(name)?;
                Ok(self.0.set_trigger_name(n))
            }

            #[setter]
            fn set_trigger_threshold(&mut self, x: u32) -> bool {
                self.0.set_trigger_threshold(x)
            }

            fn clear_trigger(&mut self) -> bool {
                self.0.clear_trigger()
            }

            // #[getter]
            // fn get_ranges<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyAny>>> {
            //     let mut rs = vec![];
            //     for r in self.0.ranges() {
            //         rs.push(float_or_int_to_any(r.0, py)?);
            //     }
            //     Ok(rs)
            // }

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
            fn set_all_shortnames(&mut self, names: Vec<String>) -> PyResult<()> {
                let mut ns = vec![];
                for x in names {
                    ns.push(str_to_shortname(x)?);
                }
                self.0
                    .set_all_shortnames(ns)
                    .map_err(|e| PyreflowException::new_err(e.to_string()))
                    .map(|_| ())
            }

            // fn set_data_f32(&mut self, ranges: Vec<f32>) -> PyResult<()> {
            //     self.0
            //         .set_data_f32(ranges)
            //         .map_err(|e| PyreflowException::new_err(e.to_string()))
            // }

            // fn set_data_f64(&mut self, ranges: Vec<f64>) -> PyResult<()> {
            //     self.0
            //         .set_data_f64(ranges)
            //         .map_err(|e| PyreflowException::new_err(e.to_string()))
            // }

            // fn set_data_ascii(&mut self, rs: Vec<PyAsciiRangeSetter>) -> PyResult<()> {
            //     self.0
            //         .set_data_ascii(rs.into_iter().map(|x| x.into()).collect())
            //         .map_err(|e| PyreflowException::new_err(e.to_string()))
            // }

            // fn set_data_delimited(&mut self, ranges: Vec<u64>) -> PyResult<()> {
            //     self.0
            //         .set_data_delimited(ranges)
            //         .map_err(|e| PyreflowException::new_err(e.to_string()))
            // }

            #[getter]
            fn get_detector_voltages(&self) -> Vec<(usize, Option<f32>)> {
                self.0
                    .detector_voltages()
                    .into_iter()
                    .map(|(i, x)| (
                        i.into(),
                        x.as_ref().copied().map(|y| y.0.into())
                    ))
                    .collect()
            }

            #[setter]
            fn set_detector_voltages(&mut self, xs: Vec<Option<f32>>) -> PyResult<()> {
                let mut ys = vec![];
                for x in xs {
                    ys.push(
                        x.map(f32_to_nonneg_float)
                            .transpose()?
                            .map(DetectorVoltage)
                    );
                }
                self.0
                    .set_detector_voltages(ys)
                    .map_err(|e| PyreflowException::new_err(e.to_string()))
            }

            #[getter]
            fn get_powers(&self) -> Vec<(usize, Option<f32>)> {
                self.0
                    .powers()
                    .into_iter()
                    .map(|(i, x)| (
                        i.into(),
                        x.as_ref().copied().map(|y| y.0.into())
                    ))
                    .collect()
            }

            #[setter]
            fn set_powers(&mut self, xs: Vec<Option<f32>>) -> PyResult<()> {
                let mut ys = vec![];
                for x in xs {
                    ys.push(
                        x.map(f32_to_nonneg_float)
                            .transpose()?
                            .map(Power)
                    );
                }
                self.0
                    .set_powers(ys)
                    .map_err(|e| PyreflowException::new_err(e.to_string()))
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
                    name: String,
                    force: bool
                ) -> PyResult<bool> {
                    let n = str_to_shortname(name)?;
                    self.0
                        .set_temporal(&n, (), force)
                        .def_terminate(SetTemporalFailure)
                        .map_or_else(|e| Err(handle_failure(e)), handle_warnings)
                }

                fn set_temporal_at(
                    &mut self,
                    index: usize,
                    force: bool
                ) -> PyResult<bool> {
                    self.0
                        .set_temporal_at(index.into(), (), force)
                        .def_terminate(SetTemporalFailure)
                        .map_or_else(|e| Err(handle_failure(e)), handle_warnings)
                }

                fn unset_temporal(&mut self, force: bool) -> PyResult<bool> {
                    let out = self.0.unset_temporal(force).map(|x| x.is_some());
                    Ok(out)
                        .def_terminate(SetTemporalFailure)
                        .map_or_else(|e| Err(handle_failure(e)), handle_warnings)
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
                    name: String,
                    timestep: f32,
                    force: bool
                ) -> PyResult<bool> {
                    let n = str_to_shortname(name)?;
                    let ts = f32_to_positive_float(timestep).map(Timestep)?;
                    self.0
                        .set_temporal(&n, ts, force)
                        .def_terminate(SetTemporalFailure)
                        .map_or_else(|e| Err(handle_failure(e)), handle_warnings)
                }

                fn set_temporal_at(
                    &mut self,
                    index: usize,
                    timestep: f32,
                    force: bool
                ) -> PyResult<bool> {
                    let ts = f32_to_positive_float(timestep).map(Timestep)?;
                    self.0
                        .set_temporal_at(index.into(), ts, force)
                        .def_terminate(SetTemporalFailure)
                        .map_or_else(|e| Err(handle_failure(e)), handle_warnings)
                }

                fn unset_temporal(&mut self, force: bool) -> PyResult<Option<f32>> {
                    let out = self.0.unset_temporal(force).map(|x| x.map(|y| y.0.into()));
                    Ok(out)
                        .def_terminate(SetTemporalFailure)
                        .map_or_else(|e| Err(handle_failure(e)), handle_warnings)
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
    ($([$pytype:ident, $opttype:ident, $timetype:ident]),*) => {
        $(
            #[pymethods]
            impl $pytype {
                fn remove_measurement_by_name<'py>(
                    &mut self,
                    name: String,
                    py: Python<'py>,
                ) -> PyResult<Option<(usize, Bound<'py, PyAny>)>> {
                    let n = str_to_shortname(name)?;
                    self.0
                        .remove_measurement_by_name(&n)
                        .map(|(i, x)| {
                            x.both(
                                |l| $timetype::from(l).into_bound_py_any(py),
                                |r| $opttype::from(r).into_bound_py_any(py)
                            ).map(|x| (usize::from(i), x))
                        }).transpose()
                }

                fn measurement_at<'py>(
                    &self,
                    i: usize,
                    py: Python<'py>
                ) -> PyResult<Bound<'py, PyAny>> {
                    // TODO this is basically going to emit an "index out of
                    // bounds" error, which python already has
                    let m = self.0
                        .measurements_named_vec().get(i.into())
                        .map_err(|e| PyreflowException::new_err(e.to_string()))?;
                    m.both(
                        |(_, l)| $timetype::from(l.clone()).into_bound_py_any(py),
                        |(_, r)| $opttype::from(r.clone()).into_bound_py_any(py)
                    )
                }

                fn replace_optical_at<'py>(
                    &mut self,
                    i: usize,
                    m: $opttype,
                    py: Python<'py>
                ) -> PyResult<Bound<'py, PyAny>> {
                    let r = self.0
                        .replace_optical_at(i.into(), m.into())
                        .map_err(|e| PyreflowException::new_err(e.to_string()))?;
                    r.both(
                        |l| $timetype::from(l).into_bound_py_any(py),
                        |r| $opttype::from(r).into_bound_py_any(py),
                    )
                }

                fn replace_optical_named<'py>(
                    &mut self,
                    name: String,
                    m: $opttype,
                    py: Python<'py>
                ) -> PyResult<Option<Bound<'py, PyAny>>> {
                    let n = str_to_shortname(name)?;
                    let r = self.0.replace_optical_named(&n, m.into());
                    r.map(|x| x.both(
                        |l| $timetype::from(l).into_bound_py_any(py),
                        |r| $opttype::from(r).into_bound_py_any(py),
                    )).transpose()
                }

                fn rename_temporal(&mut self, name: String) -> PyResult<Option<String>> {
                    let n = str_to_shortname(name)?;
                    Ok(self.0.rename_temporal(n).map(|n| n.to_string()))
                }

                fn replace_temporal_at<'py>(
                    &mut self,
                    i: usize,
                    m: $timetype,
                    force: bool,
                    py: Python<'py>
                ) -> PyResult<Bound<'py, PyAny>> {
                    let r = self.0
                        .replace_temporal_at(i.into(), m.into(), force)
                        .def_terminate(SetTemporalFailure)
                        .map_or_else(|e| Err(handle_failure(e)), handle_warnings)?;
                    r.both(
                        |l| $timetype::from(l).into_bound_py_any(py),
                        |r| $opttype::from(r).into_bound_py_any(py),
                    )
                }

                fn replace_temporal_named<'py>(
                    &mut self,
                    name: String,
                    m: $timetype,
                    force: bool,
                    py: Python<'py>
                ) -> PyResult<Option<Bound<'py, PyAny>>> {
                    let n = str_to_shortname(name)?;
                    let r = self.0
                        .replace_temporal_named(&n, m.into(), force)
                        .def_terminate(SetTemporalFailure)
                        .map_or_else(|e| Err(handle_failure(e)), handle_warnings)?;
                    r.map(|x| x.both(
                        |l| $timetype::from(l).into_bound_py_any(py),
                        |r| $opttype::from(r).into_bound_py_any(py),
                    )).transpose()
                }

                #[getter]
                fn measurements<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyAny>>> {
                    // This might seem inefficient since we are cloning
                    // everything, but if we want to map a python lambda
                    // function over the measurements we would need to to do
                    // this anyways, so simply returnig a copied list doesn't
                    // lose anything and keeps this API simpler.
                    let mut ret = vec![];
                    for x in self.0
                        .measurements_named_vec()
                        .iter()
                        .map(|(_, x)| x.both(
                            |l| $timetype::from(l.value.clone()).into_bound_py_any(py),
                            |r| $opttype::from(r.value.clone()).into_bound_py_any(py)
                        ))
                    {
                        ret.push(x?);
                    }
                    Ok(ret)
                }
            }
        )*
    };
}

common_meas_get_set!(
    [PyCoreTEXT2_0, PyOptical2_0, PyTemporal2_0],
    [PyCoreTEXT3_0, PyOptical3_0, PyTemporal3_0],
    [PyCoreTEXT3_1, PyOptical3_1, PyTemporal3_1],
    [PyCoreTEXT3_2, PyOptical3_2, PyTemporal3_2],
    [PyCoreDataset2_0, PyOptical2_0, PyTemporal2_0],
    [PyCoreDataset3_0, PyOptical3_0, PyTemporal3_0],
    [PyCoreDataset3_1, PyOptical3_1, PyTemporal3_1],
    [PyCoreDataset3_2, PyOptical3_2, PyTemporal3_2]
);

macro_rules! common_coretext_meas_get_set {
    ($([$pytype:ident, $timetype:ident]),*) => {
        $(
            #[pymethods]
            impl $pytype {
                fn push_temporal(
                    &mut self,
                    name: String,
                    t: $timetype,
                    r: BigDecimal,
                    notrunc: bool,
                ) -> PyResult<()> {
                    let n = str_to_shortname(name)?;
                    self.0
                        .push_temporal(n, t.into(), Range(r), notrunc)
                        .def_terminate(PushTemporalFailure)
                        .map_or_else(|e| Err(handle_failure(e)), handle_warnings)
                }

                fn insert_time_channel(
                    &mut self,
                    i: usize,
                    name: String,
                    t: $timetype,
                    r: BigDecimal,
                    notrunc: bool,
                ) -> PyResult<()> {
                    let n = str_to_shortname(name)?;
                    self.0
                        .insert_temporal(i.into(), n, t.into(), Range(r), notrunc)
                        .def_terminate(InsertTemporalFailure)
                        .map_or_else(|e| Err(handle_failure(e)), handle_warnings)
                }

                fn unset_measurements(
                    &mut self,
                ) -> PyResult<()> {
                    self.0.unset_measurements()
                        .map_err(|e| PyreflowException::new_err(e.to_string()))
                }
            }
        )*
    };
}

common_coretext_meas_get_set!(
    [PyCoreTEXT2_0, PyTemporal2_0],
    [PyCoreTEXT3_0, PyTemporal3_0]
);

macro_rules! coredata_meas_get_set {
    ($([$pytype:ident, $timetype:ident]),*) => {
        $(
            #[pymethods]
            impl $pytype {
                fn push_temporal(
                    &mut self,
                    name: String,
                    t: $timetype,
                    xs: PySeries,
                    r: BigDecimal,
                    notrunc: bool,
                ) -> PyResult<()> {
                    let n = str_to_shortname(name)?;
                    let col = series_to_fcs(xs.into()).map_err(PyreflowException::new_err)?;
                    self.0
                        .push_temporal(n, t.into(), col, Range(r), notrunc)
                        .def_terminate(PushTemporalFailure)
                        .map_or_else(|e| Err(handle_failure(e)), handle_warnings)
                }

                fn insert_time_channel(
                    &mut self,
                    i: usize,
                    name: String,
                    t: $timetype,
                    xs: PySeries,
                    r: BigDecimal,
                    notrunc: bool,
                ) -> PyResult<()> {
                    let n = str_to_shortname(name)?;
                    let col = series_to_fcs(xs.into()).map_err(PyreflowException::new_err)?;
                    self.0
                        .insert_temporal(i.into(), n, t.into(), col, Range(r), notrunc)
                        .def_terminate(InsertTemporalFailure)
                        .map_or_else(|e| Err(handle_failure(e)), handle_warnings)
                }

                fn unset_data(
                    &mut self,
                ) -> PyResult<()> {
                    self.0.unset_data()
                        .map_err(|e| PyreflowException::new_err(e.to_string()))
                }

                #[getter]
                fn data(&self) -> PyDataFrame {
                    let ns = self.0.all_shortnames();
                    let columns = self
                        .0
                        .data()
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
        )*

    };
}

coredata_meas_get_set!(
    [PyCoreDataset2_0, PyTemporal2_0],
    [PyCoreDataset3_0, PyTemporal3_0],
    [PyCoreDataset3_1, PyTemporal3_1],
    [PyCoreDataset3_2, PyTemporal3_2]
);

macro_rules! coretext2_0_meas_methods {
    ($([$pytype:ident, $opttype:ident, $timetype:ident]),*) => {
        $(
            #[pymethods]
            impl $pytype {
                fn remove_measurement_by_index<'py>(
                    &mut self,
                    index: usize,
                    py: Python<'py>,
                ) -> PyResult<(Option<String>, Bound<'py, PyAny>)> {
                    let r = self.0
                        .remove_measurement_by_index(index.into())
                        .map_err(|e| PyreflowException::new_err(e.to_string()))?;
                    r.both(
                        |p| {
                            let a = $timetype::from(p.value).into_bound_py_any(py)?;
                            Ok((Some(p.key.to_string()), a))
                        },
                        |p| {
                            let a = $opttype::from(p.value).into_bound_py_any(py)?;
                            Ok((p.key.0.map(|n| n.to_string()), a))
                        },
                    )
                }

                #[pyo3(signature = (m, r, notrunc=false, name=None))]
                fn push_measurement(
                    &mut self,
                    m: $opttype,
                    r: BigDecimal,
                    notrunc: bool,
                    name: Option<String>,
                ) -> PyResult<String> {
                    let n = name.map(str_to_shortname).transpose()?;
                    self.0
                        .push_optical(n.into(), m.into(), Range(r), notrunc)
                        .def_map_value(|x| x.to_string())
                        .def_terminate(InsertOpticalFailure)
                        .map_or_else(|e| Err(handle_failure(e)), handle_warnings)
                }

                #[pyo3(signature = (i, m, r, notrunc=false, name=None))]
                fn insert_optical(
                    &mut self,
                    i: usize,
                    m: $opttype,
                    r: BigDecimal,
                    notrunc: bool,
                    name: Option<String>,
                ) -> PyResult<String> {
                    let n = name.map(str_to_shortname).transpose()?;
                    self.0
                        .insert_optical(i.into(), n.into(), m.into(), Range(r), notrunc)
                        .def_map_value(|x| x.to_string())
                        .def_terminate(InsertOpticalFailure)
                        .map_or_else(|e| Err(handle_failure(e)), handle_warnings)
                }
            }
        )*
    }
}

coretext2_0_meas_methods!(
    [PyCoreTEXT2_0, PyOptical2_0, PyTemporal2_0],
    [PyCoreTEXT3_0, PyOptical3_0, PyTemporal3_0]
);

macro_rules! coretext3_1_meas_methods {
    ($([$pytype:ident, $opttype:ident, $timetype:ident]),*) => {
        $(
            #[pymethods]
            impl $pytype {
                fn remove_measurement_by_index<'py>(
                    &mut self,
                    index: usize,
                    py: Python<'py>,
                ) -> PyResult<(String, Bound<'py, PyAny>)> {
                    let r = self.0
                        .remove_measurement_by_index(index.into())
                        .map_err(|e| PyreflowException::new_err(e.to_string()))?;
                    r.both(
                        |p| {
                            let a = $timetype::from(p.value).into_bound_py_any(py)?;
                            Ok((p.key.to_string(), a))
                        },
                        |p| {
                            let a = $opttype::from(p.value).into_bound_py_any(py)?;
                            Ok((p.key.0.to_string(), a))
                        },
                    )
                }

                fn push_optical(
                    &mut self,
                    m: $opttype,
                    name: String,
                    r: BigDecimal,
                    notrunc: bool,
                ) -> PyResult<()> {
                    let n = str_to_shortname(name)?;
                    self.0
                        .push_optical(Identity(n), m.into(), Range(r), notrunc)
                        .def_map_value(|_| ())
                        .def_terminate(PushOpticalFailure)
                        .map_or_else(|e| Err(handle_failure(e)), handle_warnings)
                }

                fn insert_optical(
                    &mut self,
                    i: usize,
                    m: $opttype,
                    name: String,
                    r: BigDecimal,
                    notrunc: bool,
                ) -> PyResult<()> {
                    let n = str_to_shortname(name)?;
                    self.0
                        .insert_optical(i.into(), Identity(n), m.into(), Range(r), notrunc)
                        .def_map_value(|_| ())
                        .def_terminate(InsertOpticalFailure)
                        .map_or_else(|e| Err(handle_failure(e)), handle_warnings)
                }
            }
        )*
    };
}

coretext3_1_meas_methods!(
    [PyCoreTEXT3_1, PyOptical3_1, PyTemporal3_1],
    [PyCoreTEXT3_2, PyOptical3_2, PyTemporal3_2]
);

macro_rules! set_measurements2_0 {
    ($([$pytype:ident, $opttype:ident, $timetype:ident]),*) => {
        $(
            #[pymethods]
            impl $pytype {
                fn set_measurements(
                    &mut self,
                    xs: Vec<Bound<'_, PyAny>>,
                    prefix: String,
                ) -> PyResult<()> {
                    let sp = str_to_shortname_prefix(prefix)?;
                    let mut ys = vec![];
                    for x in xs {
                        let y = if let Ok((n, m)) = any_to_opt_named_pair::<$opttype>(x.clone()) {
                            Element::NonCenter((n, m.into()))
                        } else {
                            let (n, t) = any_to_named_pair::<$timetype>(x)?;
                            Element::Center((n, t.into()))
                        };
                        ys.push(y);
                    }
                    self.0
                        .set_measurements(ys, sp)
                        .map_err(|e| PyreflowException::new_err(e.to_string()))
                }
            }
        )*
    }
}

set_measurements2_0!(
    [PyCoreTEXT2_0, PyOptical2_0, PyTemporal2_0],
    [PyCoreTEXT3_0, PyOptical3_0, PyTemporal3_0],
    [PyCoreDataset2_0, PyOptical2_0, PyTemporal2_0],
    [PyCoreDataset3_0, PyOptical3_0, PyTemporal3_0]
);

macro_rules! set_measurements3_1 {
    ($([$pytype:ident, $opttype:ident, $timetype:ident]),*) => {
        $(
            #[pymethods]
            impl $pytype {
                pub fn set_measurements(
                    &mut self,
                    xs: Vec<Bound<'_, PyAny>>,
                ) -> PyResult<()> {
                    let mut ys = vec![];
                    for x in xs {
                        let y = if let Ok((n, m)) = any_to_named_pair::<$opttype>(x.clone()) {
                            Element::NonCenter((Identity(n), m.into()))
                        } else {
                            let (n, t) = any_to_named_pair::<$timetype>(x)?;
                            Element::Center((n, t.into()))
                        };
                        ys.push(y);
                    }
                    self.0
                        .set_measurements(ys)
                        .map_err(|e| PyreflowException::new_err(e.to_string()))
                }
            }
        )*
    }
}

set_measurements3_1!(
    [PyCoreTEXT3_1, PyOptical3_1, PyTemporal3_1],
    [PyCoreTEXT3_2, PyOptical3_2, PyTemporal3_2],
    [PyCoreDataset3_1, PyOptical3_1, PyTemporal3_1],
    [PyCoreDataset3_2, PyOptical3_2, PyTemporal3_2]
);

macro_rules! coredata2_0_meas_methods {
    ($([$pytype:ident, $opttype:ident, $timetype:ident]),*) => {
        $(
            #[pymethods]
            impl $pytype {
                fn set_measurements_and_data(
                    &mut self,
                    xs: Vec<Bound<'_, PyAny>>,
                    df: PyDataFrame,
                    prefix: String,
                ) -> PyResult<()> {
                    let sp = str_to_shortname_prefix(prefix)?;
                    let mut ys = vec![];
                    for x in xs {
                        let y = if let Ok((n, m)) = any_to_opt_named_pair::<$opttype>(x.clone()) {
                            Element::NonCenter((n, m.into()))
                        } else {
                            let (n, t) = any_to_named_pair::<$timetype>(x)?;
                            Element::Center((n, t.into()))
                        };
                        ys.push(y);
                    };
                    let go = || {
                        let cols = dataframe_to_fcs(df.into())
                            .map_err(|e| e.to_string())?;
                        self.0.set_measurements_and_data(ys, cols, sp)
                            .map_err(|e| e.to_string())
                    };
                    go().map_err(PyreflowException::new_err)
                }
            }
        )*
    }
}

coredata2_0_meas_methods!(
    [PyCoreDataset2_0, PyOptical2_0, PyTemporal2_0],
    [PyCoreDataset3_0, PyOptical3_0, PyTemporal3_0]
);

macro_rules! coredata3_1_meas_methods {
    ($([$pytype:ident, $opttype:ident, $timetype:ident]),*) => {
        $(
            #[pymethods]
            impl $pytype {
                fn set_measurements_and_data(
                    &mut self,
                    xs: Vec<Bound<'_, PyAny>>,
                    df: PyDataFrame,
                ) -> PyResult<()> {
                    let mut ys = vec![];
                    for x in xs {
                        let y = if let Ok((n, m)) = any_to_named_pair::<$opttype>(x.clone()) {
                            Element::NonCenter((Identity(n), m.into()))
                        } else {
                            let (n, t) = any_to_named_pair::<$timetype>(x)?;
                            Element::Center((n, t.into()))
                        };
                        ys.push(y);
                    };
                    let go = || {
                        let cols = dataframe_to_fcs(df.into())
                        .map_err(|e| e.to_string())?;
                        self.0.set_measurements_and_data(ys, cols)
                        .map_err(|e| e.to_string())
                    };
                    go().map_err(PyreflowException::new_err)
                }
            }
        )*
    }
}

coredata3_1_meas_methods!(
    [PyCoreDataset3_1, PyOptical3_1, PyTemporal3_1],
    [PyCoreDataset3_2, PyOptical3_2, PyTemporal3_2]
);

// Get/set methods for setting $PnN (2.0-3.0)
macro_rules! shortnames_methods {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                fn set_measurement_shortnames_maybe(
                    &mut self,
                    names: Vec<Option<String>>,
                ) -> PyResult<()> {
                    let mut ns = vec![];
                    for x in names {
                        ns.push(x.map(str_to_shortname).transpose()?);
                    }
                    self.0
                        .set_measurement_shortnames_maybe(ns)
                        .map_err(|e| PyreflowException::new_err(e.to_string()))
                        .map(|_| ())
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

// Get/set methods for setting integer measurement types (2.0-3.0)
// macro_rules! integer_2_0_methods {
//     ($($pytype:ident),*) => {
//         $(
//             #[pymethods]
//             impl $pytype {
//                 fn set_data_integer(&mut self, rs: Vec<u64>, byteord: Vec<u8>) -> PyResult<()> {
//                     let o = vec_to_byteord(byteord)?;
//                     self.0
//                         .set_data_integer(rs, o)
//                         .map_err(|e| PyreflowException::new_err(e.to_string()))
//                 }
//             }
//         )*
//     };
// }

// integer_2_0_methods!(
//     PyCoreTEXT2_0,
//     PyCoreTEXT3_0,
//     PyCoreDataset2_0,
//     PyCoreDataset3_0
// );

// Get/set methods for setting integer measurement types (3.1-3.2)
// macro_rules! integer_methods {
//     ($($pytype:ident),*) => {
//         $(
//             #[pymethods]
//             impl $pytype {
//                 fn set_data_integer(&mut self, rs: Vec<PyNumRangeSetter>) -> PyResult<()> {
//                     self.0
//                         .set_data_integer(rs.into_iter().map(|x| x.into()).collect())
//                         .map_err(|e| PyreflowException::new_err(e.to_string()))
//                 }
//             }
//         )*
//     };
// }

// integer_methods!(
//     PyCoreTEXT3_1,
//     PyCoreTEXT3_2,
//     PyCoreDataset3_1,
//     PyCoreDataset3_2
// );

// Get/set methods for $BYTEORD (3.1-3.2)
// macro_rules! endian_methods {
//     ($($pytype:ident),*) => {
//         $(
//             #[pymethods]
//             impl $pytype {
//                 #[getter]
//                 fn get_big_endian(&self) -> bool {
//                     self.0.get_big_endian()
//                 }

//                 #[setter]
//                 fn set_big_endian(&mut self, is_big: bool) {
//                     self.0.set_big_endian(is_big)
//                 }
//             }
//         )*
//     };
// }

// endian_methods!(
//     PyCoreTEXT3_1,
//     PyCoreTEXT3_2,
//     PyCoreDataset3_1,
//     PyCoreDataset3_2
// );

// Get/set methods for $PnE (3.0-3.2)
macro_rules! scales_methods {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn get_all_scales(&self) -> Vec<PyScale> {
                    self.0.all_scales().into_iter().map(|x| x.into()).collect()
                }

                #[getter]
                fn get_scales(&self) -> Vec<(usize, PyScale)> {
                    self.0
                        .scales()
                        .into_iter()
                        .map(|(i, x)| (i.into(), x.into()))
                        .collect()
                }

                #[setter]
                fn set_scales(&mut self, xs: Vec<PyScale>) -> PyResult<()> {
                    self.0
                        .set_scales(xs.into_iter().map(|x| x.into()).collect())
                        .map_err(|e| PyreflowException::new_err(e.to_string()))
                }
            }
        )*
    };
}

scales_methods!(
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
                    self.0
                        .measurements_named_vec()
                        .as_center()
                        .and_then(|x| x.value.specific.timestep())
                        .map(|x| x.0.into())
                }

                #[setter]
                fn set_timestep(&mut self, x: f32) -> PyResult<bool> {
                    let ts = Timestep(f32_to_positive_float(x)?);
                    let res = self.0
                        .temporal_mut()
                        .map(|y| y.value.specific.set_timestep(ts))
                        .is_some();
                    Ok(res)
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
                    self.0
                        .wavelengths()
                        .into_iter()
                        .map(|(i, x)| (i.into(), x.map(|y| y.0.into())))
                        .collect()
                }

                #[setter]
                fn set_wavelengths(&mut self, xs: Vec<Option<f32>>) -> PyResult<()> {
                    let ys = xs
                        .into_iter()
                        .map(|x| x.map(|y| f32_to_positive_float(y).map(|z| z.into())).transpose())
                        .collect::<Result<Vec<_>, _>>()?;
                    self.0
                        .set_wavelengths(ys)
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
                    self.0
                        .wavelengths()
                        .into_iter()
                        .map(|(i, x)| {
                            (
                                i.into(),
                                x.map(
                                    |y| y
                                        .0
                                        .iter()
                                        .copied()
                                        .map(|x| x.into()).collect()
                                ).unwrap_or_default(),
                            )
                        })
                        .collect()
                }

                #[setter]
                fn set_wavelengths(&mut self, xs: Vec<Vec<f32>>) -> PyResult<()> {
                    let ys = xs
                        .into_iter()
                        .map(|ys| ys
                             .into_iter()
                             .map(f32_to_positive_float)
                             .collect::<Result<Vec<_>, _>>()
                             .map(
                                 |zs| NonEmpty::from_vec(zs).map(Wavelengths)
                             )
                        )
                        .collect::<Result<Vec<_>, _>>()?;
                    self.0
                        .set_wavelengths(ys)
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
        get_set_copied!(
            $($pytype,)*
            [metaroot, specific, modification],
            get_originality,
            set_originality,
            originality,
            PyOriginality
        );

        get_set_copied!(
            $($pytype,)*
            [metaroot, specific, modification],
            get_last_modified,
            set_last_modified,
            last_modified,
            NaiveDateTime
        );

        get_set_str!(
            $($pytype,)*
            [metaroot, specific, modification],
            get_last_modifier,
            set_last_modifier,
            last_modifier
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
        get_set_str!($($pytype,)* [metaroot, specific, carrier], get_carriertype, set_carriertype, carriertype);
        get_set_str!($($pytype,)* [metaroot, specific, carrier], get_carrierid,   set_carrierid,   carrierid);
        get_set_str!($($pytype,)* [metaroot, specific, carrier], get_locationid,  set_locationid,  locationid);
    };
}

carrier_methods!(PyCoreTEXT3_2, PyCoreDataset3_2);

// Get/set methods for $PLATEID/$WELLID/$PLATENAME (3.1-3.2)
macro_rules! plate_methods {
    ($($pytype:ident),*) => {
        get_set_str!($($pytype,)* [metaroot, specific, plate], get_wellid,    set_wellid,    wellid);
        get_set_str!($($pytype,)* [metaroot, specific, plate], get_plateid,   set_plateid,   plateid);
        get_set_str!($($pytype,)* [metaroot, specific, plate], get_platename, set_platename, platename);
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
                    self.0.compensation().map(|x| x.matrix().to_pyarray(py))
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
                    self.0.spillover().map(|x| x.matrix().to_pyarray(py))
                }

                #[getter]
                fn get_spillover_names(&self) -> Vec<String> {
                    self.0
                        .spillover()
                        .map(|x| x.measurements())
                        .unwrap_or_default()
                        .iter()
                        .map(|x| x.as_ref().to_string())
                        .collect()
                }

                fn set_spillover(
                    &mut self,
                    names: Vec<String>,
                    a: PyReadonlyArray2<f32>,
                ) -> Result<(), PyErr> {
                    let mut ns = vec![];
                    for x in names {
                        ns.push(str_to_shortname(x)?);
                    }
                    let m = a.as_matrix().into_owned();
                    self.0
                        .set_spillover(ns, m)
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

// Get/set methods for $VOL (3.1-3.2)
macro_rules! vol_methods {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn get_vol(&self) -> Option<f32> {
                    self.0.metaroot.specific.vol.as_ref_opt().copied().map(|x| x.0.into())
                }

                #[setter]
                fn set_vol(&mut self, vol: Option<f32>) -> PyResult<()> {
                    let x = vol.map(f32_to_nonneg_float).transpose()?.map(Vol);
                    self.0.metaroot.specific.vol = x.into();
                    Ok(())
                }
            }
        )*
    };
}

vol_methods!(
    PyCoreTEXT3_1,
    PyCoreTEXT3_2,
    PyCoreDataset3_1,
    PyCoreDataset3_2
);

// Get/set methods for $PnG (3.0-3.2)
macro_rules! gain_methods {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn get_gains(&self) -> Vec<(usize, Option<f32>)> {
                    self.0
                        .gains()
                        .into_iter()
                        .map(|(i, x)| (
                            i.into(),
                            x.as_ref().copied().map(|y| y.0.into())
                        ))
                        .collect()
                }

                #[setter]
                fn set_gains(&mut self, xs: Vec<Option<f32>>) -> PyResult<()> {
                    let mut ys = vec![];
                    for x in xs {
                        ys.push(x.map(f32_to_positive_float).transpose()?.map(Gain));
                    }
                    self.0
                        .set_gains(ys)
                        .map_err(|e| PyreflowException::new_err(e.to_string()))
                }
            }
        )*
    };
}

gain_methods!(
    PyCoreTEXT3_0,
    PyCoreTEXT3_1,
    PyCoreTEXT3_2,
    PyCoreDataset3_0,
    PyCoreDataset3_1,
    PyCoreDataset3_2
);

// Get/set methods for (optional) $CYT (2.0-3.1)
//
// 3.2 is required which is why it is not included here
get_set_str!(
    PyCoreTEXT2_0,
    PyCoreTEXT3_0,
    PyCoreTEXT3_1,
    PyCoreDataset2_0,
    PyCoreDataset3_0,
    PyCoreDataset3_1,
    [metaroot, specific],
    get_cyt,
    set_cyt,
    cyt
);

// Get/set methods for $FLOWRATE (3.2)
get_set_str!(
    PyCoreTEXT3_2,
    PyCoreDataset3_2,
    [metaroot, specific],
    get_flowrate,
    set_flowrate,
    flowrate
);

// Get/set methods for $CYTSN (3.0-3.2)
get_set_str!(
    PyCoreTEXT3_0,
    PyCoreTEXT3_1,
    PyCoreTEXT3_2,
    PyCoreDataset3_0,
    PyCoreDataset3_1,
    PyCoreDataset3_2,
    [metaroot, specific],
    get_cytsn,
    set_cytsn,
    cytsn
);

// Get/set methods for $PnDET (3.2)
meas_get_set!(
    det_names,
    set_det_names,
    String,
    PyCoreTEXT3_2,
    PyCoreDataset3_2
);

// Get/set methods for $PnCALIBRATION (3.1)
meas_get_set!(
    calibrations,
    set_calibrations,
    PyCalibration3_1,
    PyCoreTEXT3_1,
    PyCoreDataset3_1
);

// Get/set methods for $PnCALIBRATION (3.2)
meas_get_set!(
    calibrations,
    set_calibrations,
    PyCalibration3_2,
    PyCoreTEXT3_2,
    PyCoreDataset3_2
);

// Get/set methods for $PnTAG (3.2)
meas_get_set!(tags, set_tags, String, PyCoreTEXT3_2, PyCoreDataset3_2);

// Get/set methods for $PnTYPE (3.2)
meas_get_set!(
    measurement_types,
    set_measurement_types,
    PyOpticalType,
    PyCoreTEXT3_2,
    PyCoreDataset3_2
);

// Get/set methods for $PnFEATURE (3.2)
meas_get_set!(
    features,
    set_features,
    PyFeature,
    PyCoreTEXT3_2,
    PyCoreDataset3_2
);

// Get/set methods for $PnANALYTE (3.2)
meas_get_set!(
    analytes,
    set_analytes,
    String,
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
                df: PyDataFrame,
                analysis: Vec<u8>,
                others: Vec<Vec<u8>>,
            ) -> PyResult<$to> {
                let cols = dataframe_to_fcs(df.into())
                    .map_err(|e| PyreflowException::new_err(e.to_string()))?;
                self.0
                    .clone()
                    .into_coredataset(
                        cols,
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

// fn handle_errors<E, X, Y>(res: error::ImpureResult<X, E>) -> PyResult<Y>
// where
//     E: fmt::Display,
//     Y: From<X>,
// {
//     handle_pure(res.map_err(PyImpureError)?)
// }

// fn handle_pure<E, X, Y>(succ: error::PureSuccess<X, E>) -> PyResult<Y>
// where
//     E: fmt::Display,
//     Y: From<X>,
// {
//     let (err, warn) = succ.deferred.split();
//     Python::with_gil(|py| -> PyResult<()> {
//         let wt = py.get_type::<PyreflowWarning>();
//         for w in warn {
//             let s = CString::new(w.to_string())?;
//             PyErr::warn(py, &wt, &s, 0)?;
//         }
//         Ok(())
//     })?;
//     if err.is_empty() {
//         Ok(succ.data.into())
//     } else {
//         let es: Vec<_> = err.iter().map(|e| e.to_string()).collect();
//         let deferred = &es[..].join("\n");
//         let msg = format!("Errors encountered:\n{deferred}");
//         Err(PyreflowException::new_err(msg))
//     }
// }

// impl<E> From<error::ImpureFailure<E>> for PyImpureError<E> {
//     fn from(value: error::ImpureFailure<E>) -> Self {
//         Self(value)
//     }
// }

// impl<E> From<PyImpureError<E>> for PyErr
// where
//     E: fmt::Display,
// {
//     fn from(err: PyImpureError<E>) -> Self {
//         let inner = err.0;
//         let reason = match inner.reason {
//             error::ImpureError::IO(e) => format!("IO ERROR: {e}"),
//             error::ImpureError::Pure(e) => format!("CRITICAL PYREFLOW ERROR: {e}"),
//         };
//         let es: Vec<_> = inner
//             .deferred
//             .into_errors()
//             .iter()
//             .map(|e| e.to_string())
//             .collect();
//         let deferred = &es[..].join("\n");
//         let msg = format!("{reason}\n\nOther errors encountered:\n{deferred}");
//         PyreflowException::new_err(msg)
//     }
// }

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
    fn new(timestep: f32) -> PyResult<Self> {
        to_positive_float(timestep).map(|ts| Temporal3_0::new(ts.into()).into())
    }
}

#[pymethods]
impl PyTemporal3_1 {
    #[new]
    fn new(timestep: f32) -> PyResult<Self> {
        to_positive_float(timestep).map(|ts| Temporal3_1::new(ts.into()).into())
    }
}

#[pymethods]
impl PyTemporal3_2 {
    #[new]
    fn new(timestep: f32) -> PyResult<Self> {
        to_positive_float(timestep).map(|ts| Temporal3_2::new(ts.into()).into())
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
                    key: String,
                    value: String
                ) -> PyResult<Option<String>> {
                    let k = str_to_nonstd_key(key)?;
                    Ok(self.0.common.nonstandard_keywords.insert(k, value))
                }

                fn nonstandard_get(&self, key: String) -> PyResult<Option<String>> {
                    let k = str_to_nonstd_key(key)?;
                    Ok(self.0.common.nonstandard_keywords.get(&k).map(|x| x.clone()))
                }

                fn nonstandard_remove(&mut self, key: String) -> PyResult<Option<String>> {
                    let k = str_to_nonstd_key(key)?;
                    Ok(self.0.common.nonstandard_keywords.remove(&k))
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

macro_rules! optical_common {
    ($($pytype:ident),*) => {
        get_set_str!($($pytype,)* [], get_filter,    set_filter,    filter);
        get_set_str!($($pytype,)* [], get_detector_type,    set_detector_type,    detector_type);
        get_set_str!($($pytype,)* [], get_percent_emitted,    set_percent_emitted,    percent_emitted);

        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn get_detector_voltage(&self) -> Option<f32> {
                    self.0.detector_voltage.as_ref_opt().map(|x| x.0.into())
                }

                #[setter]
                fn set_detector_voltage(&mut self, x: Option<f32>) -> PyResult<()> {
                    let y = x.map(to_non_neg_float).transpose()?;
                    self.0.detector_voltage = y.map(|z| z.into()).into();
                    Ok(())
                }

                // TODO not DRY
                #[getter]
                fn get_power(&self) -> Option<f32> {
                    self.0.power.as_ref_opt().map(|x| x.0.into())
                }

                #[setter]
                fn set_power(&mut self, x: Option<f32>) -> PyResult<()> {
                    let y = x.map(to_non_neg_float).transpose()?;
                    self.0.power = y.map(|z| z.into()).into();
                    Ok(())
                }
            }
        )*

    };
}

optical_common!(PyOptical2_0, PyOptical3_0, PyOptical3_1, PyOptical3_2);

// $PnL (2.0-3.0)
// get_set_copied!(
//     PyOptical2_0,
//     PyOptical3_0,
//     [specific],
//     get_wavelength,
//     set_wavelength,
//     wavelength,
//     u32
// );

// $PnE (2.0)
get_set_copied!(
    PyOptical2_0,
    [specific],
    get_scale,
    set_scale,
    scale,
    PyScale
);

// $PnG (3.0-3.2)
macro_rules! meas_get_set_gain {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn get_gain(&self) -> Option<f32> {
                    self.0.specific.gain.as_ref_opt().map(|x| x.0.into())
                }

                #[setter]
                fn set_gain(&mut self, x: Option<f32>) -> PyResult<()> {
                    let y = x.map(to_positive_float).transpose()?;
                    self.0.specific.gain = y.map(|z| z.into()).into();
                    Ok(())
                }
            }
        )*
    };
}

meas_get_set_gain!(PyOptical3_0, PyOptical3_1, PyOptical3_2);

// $PnE (3.0-3.2)
macro_rules! meas_get_set_scale {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn get_scale(&self) -> PyScale {
                    self.0.specific.scale.into()
                }

                #[setter]
                fn set_scale(&mut self, x: PyScale) {
                    self.0.specific.scale = x.into();
                }
            }
        )*
    };
}

meas_get_set_scale!(PyOptical3_0, PyOptical3_1, PyOptical3_2);

// #PnL (2.0-3.0)
macro_rules! meas_get_set_wavelengths {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn get_wavelength(&self) -> Option<f32> {
                    self.0.specific.wavelength.as_ref_opt().map(|w| w.0.into())
                }

                #[setter]
                fn set_wavelength(&mut self, x: Option<f32>) -> PyResult<()> {
                    let y = x.map(f32_to_positive_float).transpose()?;
                    self.0.specific.wavelength = y.map(|z| z.into()).into();
                    Ok(())
                }
            }
        )*
    };
}

meas_get_set_wavelengths!(PyOptical2_0, PyOptical3_0);

// #PnL (3.1-3.2)
macro_rules! meas_get_set_wavelengths {
    ($($pytype:ident),*) => {
        $(
            #[pymethods]
            impl $pytype {
                #[getter]
                fn get_wavelengths(&self) -> Vec<f32> {
                    self.0
                        .specific
                        .wavelengths
                        .as_ref_opt()
                        .map(|ws| ws.0.iter().copied().map(|x| x.into()).collect())
                        .unwrap_or_default()
                }

                #[setter]
                fn set_wavelengths(&mut self, xs: Vec<f32>) -> PyResult<()> {
                    if xs.is_empty() {
                        self.0.specific.wavelengths = None.into();
                    } else {
                        let ys = xs
                            .into_iter()
                            .map(f32_to_positive_float)
                            .collect::<Result<Vec<_>, _>>()?;
                        let ws = Some(NonEmpty::from_vec(ys).unwrap().into()).into();
                        self.0.specific.wavelengths = ws;
                    }
                    Ok(())
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
                fn set_timestep(&mut self, x: f32) -> PyResult<()> {
                    self.0.specific.timestep = to_positive_float(x)?.into();
                    Ok(())
                }
            }
        )*
    };
}

meas_get_set_timestep!(PyTemporal3_0, PyTemporal3_1, PyTemporal3_2);

// $PnCalibration (3.1)
get_set_cloned!(
    PyOptical3_1,
    [specific],
    get_calibration,
    set_calibration,
    calibration,
    PyCalibration3_1
);

// $PnD (3.1-3.2)
get_set_copied!(
    PyOptical3_1,
    PyOptical3_2,
    PyTemporal3_1,
    PyTemporal3_2,
    [specific],
    get_display,
    set_display,
    display,
    PyDisplay
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
get_set_str!(
    PyOptical3_2,
    [specific],
    get_detector_name,
    set_detector_name,
    detector_name
);

// $PnTAG (3.2)
get_set_str!(PyOptical3_2, [specific], get_tag, set_tag, tag);

// $PnTYPE (3.2)
get_set_cloned!(
    PyOptical3_2,
    [specific],
    get_measurement_type,
    set_measurement_type,
    measurement_type,
    PyOpticalType
);

// $PnTYPE (3.2)
get_set_copied!(
    PyOptical3_2,
    [specific],
    get_feature,
    set_feature,
    feature,
    PyFeature
);

// $PnTYPE (3.2)
get_set_str!(PyOptical3_2, [specific], get_analyte, set_analyte, analyte);

// $PnCalibration (3.2)
get_set_cloned!(
    PyOptical3_2,
    [specific],
    get_calibration,
    set_calibration,
    calibration,
    PyCalibration3_2
);

fn any_to_opt_named_pair<'py, X>(a: Bound<'py, PyAny>) -> PyResult<(MaybeValue<Shortname>, X)>
where
    X: FromPyObject<'py>,
{
    let tup: (Bound<'py, PyAny>, Bound<'py, PyAny>) = a.extract()?;
    let n_maybe: Option<Bound<'py, PyAny>> = tup.0.extract()?;
    let n: Option<String> = if let Some(nn) = n_maybe {
        Some(nn.extract()?)
    } else {
        None
    };
    let sn = n.map(str_to_shortname).transpose()?;
    let m: X = tup.1.extract()?;
    Ok((sn.into(), m))
}

fn any_to_named_pair<'py, X>(a: Bound<'py, PyAny>) -> PyResult<(Shortname, X)>
where
    X: FromPyObject<'py>,
{
    let tup: (Bound<'py, PyAny>, Bound<'py, PyAny>) = a.extract()?;
    let n: String = tup.0.extract()?;
    let m: X = tup.1.extract()?;
    let sn = str_to_shortname(n)?;
    Ok((sn, m))
}

// fn any_to_range(a: Bound<'_, PyAny>) -> PyResult<Range> {
//     any_to_float_or_int(a).map(Range)
// }

fn to_positive_float(x: f32) -> PyResult<PositiveFloat> {
    PositiveFloat::try_from(x).map_err(|e| PyreflowException::new_err(e.to_string()))
}

fn to_non_neg_float(x: f32) -> PyResult<NonNegFloat> {
    NonNegFloat::try_from(x).map_err(|e| PyreflowException::new_err(e.to_string()))
}

fn str_to_shortname(s: String) -> PyResult<Shortname> {
    s.parse::<Shortname>()
        .map_err(|e| PyreflowException::new_err(e.to_string()))
}

fn str_to_shortname_prefix(s: String) -> PyResult<ShortnamePrefix> {
    s.parse::<ShortnamePrefix>()
        .map_err(|e| PyreflowException::new_err(e.to_string()))
}

fn str_to_nonstd_key(s: String) -> PyResult<NonStdKey> {
    s.parse::<NonStdKey>()
        .map_err(|e| PyreflowException::new_err(e.to_string()))
}

fn str_to_nonstd_meas_pat(s: String) -> PyResult<NonStdMeasPattern> {
    s.parse::<NonStdMeasPattern>()
        .map_err(|e| PyreflowException::new_err(e.to_string()))
}

fn str_to_time_pat(s: String) -> PyResult<TimePattern> {
    s.parse::<TimePattern>()
        .map_err(|e| PyreflowException::new_err(e.to_string()))
}

fn str_to_date_pat(s: String) -> PyResult<DatePattern> {
    s.parse::<DatePattern>()
        .map_err(|e| PyreflowException::new_err(e.to_string()))
}

fn strs_to_key_patterns(lits: Vec<String>, pats: Vec<String>) -> PyResult<KeyPatterns> {
    let mut ls = KeyPatterns::try_from_literals(lits)
        .map_err(|e| PyreflowException::new_err(e.to_string()))?;
    let ps = KeyPatterns::try_from_patterns(pats)
        .map_err(|e| PyreflowException::new_err(e.to_string()))?;
    ls.extend(ps);
    Ok(ls)
}

// fn vec_to_byteord(xs: Vec<u8>) -> PyResult<ByteOrd> {
//     ByteOrd::try_from(xs).map_err(|e| PyreflowException::new_err(e.to_string()))
// }

fn f32_to_positive_float(x: f32) -> PyResult<PositiveFloat> {
    PositiveFloat::try_from(x).map_err(|e| PyreflowException::new_err(e.to_string()))
}

fn f32_to_nonneg_float(x: f32) -> PyResult<NonNegFloat> {
    NonNegFloat::try_from(x).map_err(|e| PyreflowException::new_err(e.to_string()))
}

macro_rules! column_to_buf {
    ($col:expr, $prim:ident) => {
        let ca = $col.$prim().unwrap();
        if ca.first_non_null().is_some() {
            return Err(format!("column {} has null values", $col.name()));
        }
        let buf = ca.chunks()[0]
            .as_any()
            .downcast_ref::<PrimitiveArray<$prim>>()
            .unwrap()
            .values()
            .clone();
        return Ok(FCSColumn(buf).into());
    };
}

fn series_to_fcs(ser: Series) -> Result<AnyFCSColumn, String> {
    match ser.dtype() {
        DataType::UInt8 => {
            column_to_buf!(ser, u8);
        }
        DataType::UInt16 => {
            column_to_buf!(ser, u16);
        }
        DataType::UInt32 => {
            column_to_buf!(ser, u32);
        }
        DataType::UInt64 => {
            column_to_buf!(ser, u64);
        }
        DataType::Float32 => {
            column_to_buf!(ser, f32);
        }
        DataType::Float64 => {
            column_to_buf!(ser, f64);
        }
        t => Err(format!("invalid datatype: {t}")),
    }
}

fn dataframe_to_fcs(mut df: DataFrame) -> Result<Vec<AnyFCSColumn>, String> {
    // make sure data is contiguous
    df.rechunk_mut();
    let mut cols = Vec::with_capacity(df.width());
    for c in df.iter() {
        cols.push(series_to_fcs(c.clone())?);
    }
    Ok(cols)
}

struct ConvertFailure;

impl fmt::Display for ConvertFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "could not change FCS version")
    }
}

struct SetTemporalFailure;

impl fmt::Display for SetTemporalFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "could not convert to/from temporal measurement")
    }
}

struct PushTemporalFailure;

impl fmt::Display for PushTemporalFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "could not push temporal measurement")
    }
}

struct InsertTemporalFailure;

impl fmt::Display for InsertTemporalFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "could not push temporal measurement")
    }
}

struct PushOpticalFailure;

impl fmt::Display for PushOpticalFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "could not push optical measurement")
    }
}

struct InsertOpticalFailure;

impl fmt::Display for InsertOpticalFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "could not push optical measurement")
    }
}
