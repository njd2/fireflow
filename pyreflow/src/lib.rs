use fireflow_core::api;
use fireflow_core::api::VersionedTime;
use fireflow_core::config::Strict;
use fireflow_core::config::{self, OffsetCorrection};
use fireflow_core::error;
use fireflow_core::text::byteord::*;
use fireflow_core::text::ranged_float::*;
use fireflow_core::text::scale::*;
use fireflow_core::text::spillover::*;
use fireflow_core::validated::datepattern::DatePattern;
use fireflow_core::validated::nonstandard::*;
use fireflow_core::validated::pattern::*;
use fireflow_core::validated::shortname::*;
use fireflow_core::validated::textdelim::TEXTDelim;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
use itertools::Itertools;
use nonempty::NonEmpty;
use numpy::{PyArray2, PyReadonlyArray2, ToPyArray};
use pyo3::class::basic::CompareOp;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyWarning};
use pyo3::prelude::*;
use pyo3::type_object::PyTypeInfo;
use pyo3::types::IntoPyDict;
use pyo3::types::PyDict;
use pyo3::IntoPyObjectExt;
use pyo3_polars::PyDataFrame;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::ffi::CString;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::path;

#[pymodule]
fn pyreflow(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("PyreflowException", py.get_type::<PyreflowException>())?;
    m.add("PyreflowWarning", py.get_type::<PyreflowWarning>())?;
    m.add_class::<PyTEXTDelim>()?;
    m.add_class::<PyNonStdMeasPattern>()?;
    m.add_class::<PyDatePattern>()?;
    m.add_class::<PyShortname>()?;
    m.add_function(wrap_pyfunction!(read_fcs_header, m)?)?;
    m.add_function(wrap_pyfunction!(read_fcs_raw_text, m)?)?;
    m.add_function(wrap_pyfunction!(read_fcs_std_text, m)?)?;
    m.add_function(wrap_pyfunction!(read_fcs_file, m)?)
}

#[allow(clippy::too_many_arguments)]
#[pyfunction]
#[pyo3(signature = (p, begin_text=0, end_text=0, begin_data=0, end_data=0,
                    begin_analysis=0, end_analysis=0, version_override=None))]
fn read_fcs_header(
    p: path::PathBuf,
    begin_text: i32,
    end_text: i32,
    begin_data: i32,
    end_data: i32,
    begin_analysis: i32,
    end_analysis: i32,
    version_override: Option<PyVersion>,
) -> PyResult<PyHeader> {
    let conf = config::HeaderConfig {
        version_override: version_override.map(|x| x.0),
        text: config::OffsetCorrection {
            begin: begin_text,
            end: end_text,
        },
        data: config::OffsetCorrection {
            begin: begin_data,
            end: end_data,
        },
        analysis: config::OffsetCorrection {
            begin: begin_analysis,
            end: end_analysis,
        },
    };
    handle_errors(api::read_fcs_header(&p, &conf))
}

#[allow(clippy::too_many_arguments)]
#[pyfunction]
#[pyo3(signature = (
    p,

    strict=false,

    begin_text=0,
    end_text=0,
    begin_data=0,
    end_data=0,
    begin_analysis=0,
    end_analysis=0,

    text_begin_stext=0,
    text_end_stext=0,
    allow_double_delim=false,
    force_ascii_delim=false,
    enforce_final_delim=false,
    enforce_unique=false,
    enforce_even=false,
    enforce_nonempty=false,
    error_on_invalid_utf8=false,
    enforce_keyword_ascii=false,
    enforce_stext=false,
    repair_offset_spaces=false,
    date_pattern=None,
    version_override=None)
)]
fn read_fcs_raw_text(
    p: path::PathBuf,

    strict: bool,

    begin_text: i32,
    end_text: i32,
    begin_data: i32,
    end_data: i32,
    begin_analysis: i32,
    end_analysis: i32,

    text_begin_stext: i32,
    text_end_stext: i32,
    allow_double_delim: bool,
    force_ascii_delim: bool,
    enforce_final_delim: bool,
    enforce_unique: bool,
    enforce_even: bool,
    enforce_nonempty: bool,
    error_on_invalid_utf8: bool,
    enforce_keyword_ascii: bool,
    enforce_stext: bool,
    repair_offset_spaces: bool,
    date_pattern: Option<PyDatePattern>,
    version_override: Option<PyVersion>,
) -> PyResult<PyRawTEXT> {
    let header = config::HeaderConfig {
        version_override: version_override.map(|x| x.0),
        text: config::OffsetCorrection {
            begin: begin_text,
            end: end_text,
        },
        data: config::OffsetCorrection {
            begin: begin_data,
            end: end_data,
        },
        analysis: config::OffsetCorrection {
            begin: begin_analysis,
            end: end_analysis,
        },
    };

    let conf = config::RawTextReadConfig {
        header,
        stext: config::OffsetCorrection {
            begin: text_begin_stext,
            end: text_end_stext,
        },
        allow_double_delim,
        force_ascii_delim,
        enforce_final_delim,
        enforce_unique,
        enforce_even,
        enforce_nonempty,
        error_on_invalid_utf8,
        enforce_keyword_ascii,
        enforce_stext,
        repair_offset_spaces,
        date_pattern: date_pattern.map(|x| x.0),
    };
    handle_errors(api::read_fcs_raw_text(&p, &conf.set_strict(strict)))
}

#[allow(clippy::too_many_arguments)]
#[pyfunction]
#[pyo3(signature = (
    p,

    strict=false,

    begin_text=0,
    end_text=0,
    begin_data=0,
    end_data=0,
    begin_analysis=0,
    end_analysis=0,

    text_begin_stext=0,
    text_end_stext=0,
    allow_double_delim=false,
    force_ascii_delim=false,
    enforce_final_delim=false,
    enforce_unique=false,
    enforce_even=false,
    enforce_nonempty=false,
    error_on_invalid_utf8=false,
    enforce_keyword_ascii=false,
    enforce_stext=false,
    repair_offset_spaces=false,
    disallow_deprecated=false,

    time_ensure=false,
    time_ensure_timestep=false,
    time_ensure_linear=false,
    time_ensure_nogain=false,
    disallow_deviant=false,
    disallow_nonstandard=false,

    shortname_prefix=None,
    nonstandard_measurement_pattern=None,
    time_pattern=None,
    date_pattern=None,
    version_override=None)
)]
fn read_fcs_std_text(
    p: path::PathBuf,

    strict: bool,

    begin_text: i32,
    end_text: i32,
    begin_data: i32,
    end_data: i32,
    begin_analysis: i32,
    end_analysis: i32,

    text_begin_stext: i32,
    text_end_stext: i32,
    allow_double_delim: bool,
    force_ascii_delim: bool,
    enforce_final_delim: bool,
    enforce_unique: bool,
    enforce_even: bool,
    enforce_nonempty: bool,
    error_on_invalid_utf8: bool,
    enforce_keyword_ascii: bool,
    enforce_stext: bool,
    repair_offset_spaces: bool,
    disallow_deprecated: bool,

    time_ensure: bool,
    time_ensure_timestep: bool,
    time_ensure_linear: bool,
    time_ensure_nogain: bool,

    disallow_deviant: bool,
    disallow_nonstandard: bool,

    shortname_prefix: Option<PyShortnamePrefix>,
    nonstandard_measurement_pattern: Option<PyNonStdMeasPattern>,
    time_pattern: Option<PyTimePattern>,
    date_pattern: Option<PyDatePattern>,
    version_override: Option<PyVersion>,
) -> PyResult<PyStandardizedTEXT> {
    let header = config::HeaderConfig {
        version_override: version_override.map(|x| x.0),
        text: config::OffsetCorrection {
            begin: begin_text,
            end: end_text,
        },
        data: config::OffsetCorrection {
            begin: begin_data,
            end: end_data,
        },
        analysis: config::OffsetCorrection {
            begin: begin_analysis,
            end: end_analysis,
        },
    };

    let raw = config::RawTextReadConfig {
        header,
        stext: config::OffsetCorrection {
            begin: text_begin_stext,
            end: text_end_stext,
        },
        allow_double_delim,
        force_ascii_delim,
        enforce_final_delim,
        enforce_unique,
        enforce_even,
        enforce_nonempty,
        error_on_invalid_utf8,
        enforce_keyword_ascii,
        enforce_stext,
        repair_offset_spaces,
        date_pattern: date_pattern.map(|x| x.0),
    };

    let conf = config::StdTextReadConfig {
        raw,
        shortname_prefix: shortname_prefix.map(|x| x.0).unwrap_or_default(),
        time: config::TimeConfig {
            pattern: time_pattern.map(|x| x.0),
            ensure: time_ensure,
            ensure_timestep: time_ensure_timestep,
            ensure_linear: time_ensure_linear,
            ensure_nogain: time_ensure_nogain,
        },
        disallow_deviant,
        disallow_nonstandard,
        disallow_deprecated,
        nonstandard_measurement_pattern: nonstandard_measurement_pattern.map(|x| x.0),
    };

    handle_errors(api::read_fcs_std_text(&p, &conf.set_strict(strict)))
}

#[allow(clippy::too_many_arguments)]
#[pyfunction]
#[pyo3(signature = (
    p,

    strict=false,

    header_begin_text=0,
    header_end_text=0,
    header_begin_data=0,
    header_end_data=0,
    header_begin_analysis=0,
    header_end_analysis=0,
    text_begin_stext=0,
    text_end_stext=0,
    text_begin_data=0,
    text_end_data=0,
    text_begin_analysis=0,
    text_end_analysis=0,

    allow_double_delim=false,
    force_ascii_delim=false,
    enforce_final_delim=false,
    enforce_unique=false,
    enforce_even=false,
    enforce_nonempty=false,
    error_on_invalid_utf8=false,
    enforce_keyword_ascii=false,
    enforce_stext=false,
    repair_offset_spaces=false,
    disallow_deprecated=false,

    time_ensure=false,
    time_ensure_timestep=false,
    time_ensure_linear=false,
    time_ensure_nogain=false,

    disallow_deviant=false,
    disallow_nonstandard=false,
    enforce_data_width_divisibility=false,
    enforce_matching_tot=false,

    shortname_prefix=None,
    nonstandard_measurement_pattern=None,
    time_pattern=None,
    date_pattern=None,
    version_override=None)
)]
fn read_fcs_file(
    p: path::PathBuf,

    strict: bool,

    header_begin_text: i32,
    header_end_text: i32,
    header_begin_data: i32,
    header_end_data: i32,
    header_begin_analysis: i32,
    header_end_analysis: i32,

    text_begin_stext: i32,
    text_end_stext: i32,
    text_begin_data: i32,
    text_end_data: i32,
    text_begin_analysis: i32,
    text_end_analysis: i32,

    allow_double_delim: bool,
    force_ascii_delim: bool,
    enforce_final_delim: bool,
    enforce_unique: bool,
    enforce_even: bool,
    enforce_nonempty: bool,
    error_on_invalid_utf8: bool,
    enforce_keyword_ascii: bool,
    enforce_stext: bool,
    repair_offset_spaces: bool,
    disallow_deprecated: bool,

    time_ensure: bool,
    time_ensure_timestep: bool,
    time_ensure_linear: bool,
    time_ensure_nogain: bool,

    disallow_deviant: bool,
    disallow_nonstandard: bool,
    enforce_data_width_divisibility: bool,
    enforce_matching_tot: bool,

    shortname_prefix: Option<PyShortnamePrefix>,
    nonstandard_measurement_pattern: Option<PyNonStdMeasPattern>,
    time_pattern: Option<PyTimePattern>,
    date_pattern: Option<PyDatePattern>,
    version_override: Option<PyVersion>,
) -> PyResult<PyStandardizedDataset> {
    let header = config::HeaderConfig {
        version_override: version_override.map(|x| x.0),
        text: config::OffsetCorrection {
            begin: header_begin_text,
            end: header_end_text,
        },
        data: config::OffsetCorrection {
            begin: header_begin_data,
            end: header_end_data,
        },
        analysis: config::OffsetCorrection {
            begin: header_begin_analysis,
            end: header_end_analysis,
        },
    };

    let raw = config::RawTextReadConfig {
        header,
        stext: config::OffsetCorrection {
            begin: text_begin_stext,
            end: text_end_stext,
        },
        allow_double_delim,
        force_ascii_delim,
        enforce_final_delim,
        enforce_unique,
        enforce_even,
        enforce_nonempty,
        error_on_invalid_utf8,
        enforce_keyword_ascii,
        enforce_stext,
        repair_offset_spaces,
        date_pattern: date_pattern.map(|x| x.0),
    };

    let standard = config::StdTextReadConfig {
        raw,
        shortname_prefix: shortname_prefix.map(|x| x.0).unwrap_or_default(),
        time: config::TimeConfig {
            pattern: time_pattern.map(|x| x.0),
            ensure: time_ensure,
            ensure_timestep: time_ensure_timestep,
            ensure_linear: time_ensure_linear,
            ensure_nogain: time_ensure_nogain,
        },
        disallow_deviant,
        disallow_deprecated,
        disallow_nonstandard,
        nonstandard_measurement_pattern: nonstandard_measurement_pattern.map(|x| x.0),
    };

    let conf = config::DataReadConfig {
        standard,
        data: OffsetCorrection {
            begin: text_begin_data,
            end: text_end_data,
        },
        analysis: OffsetCorrection {
            begin: text_begin_analysis,
            end: text_end_analysis,
        },
        enforce_data_width_divisibility,
        enforce_matching_tot,
    };

    handle_errors(api::read_fcs_file(&p, &conf.set_strict(strict)))
}

macro_rules! pywrap {
    ($pytype:ident, $rstype:path, $name:expr) => {
        #[pyclass(name = $name)]
        #[derive(Clone)]
        #[repr(transparent)]
        struct $pytype($rstype);

        impl From<$rstype> for $pytype {
            fn from(value: $rstype) -> Self {
                Self(value)
            }
        }

        impl From<$pytype> for $rstype {
            fn from(value: $pytype) -> Self {
                value.0
            }
        }
    };
}

macro_rules! py_eq {
    ($pytype:ident) => {
        impl PartialEq for $pytype {
            fn eq(&self, other: &Self) -> bool {
                self.0 == other.0
            }
        }

        impl Eq for $pytype {}

        #[pymethods]
        impl $pytype {
            fn __eq__(&self, other: &Self) -> bool {
                self.0 == other.0
            }
        }
    };
}

macro_rules! py_ord {
    ($pytype:ident) => {
        impl PartialEq for $pytype {
            fn eq(&self, other: &Self) -> bool {
                self.0 == other.0
            }
        }

        impl Eq for $pytype {}

        impl Ord for $pytype {
            fn cmp(&self, other: &Self) -> Ordering {
                self.0.cmp(&other.0)
            }
        }

        impl PartialOrd for $pytype {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.0.cmp(&other.0))
            }
        }

        #[pymethods]
        impl $pytype {
            fn __richcmp__(&self, other: &Self, op: CompareOp) -> bool {
                op.matches(self.0.cmp(&other.0))
            }
        }
    };
}

macro_rules! py_disp {
    ($pytype:ident) => {
        impl fmt::Display for $pytype {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
                self.0.fmt(f)
            }
        }

        #[pymethods]
        impl $pytype {
            fn __repr__(&self) -> String {
                let name = <$pytype as PyTypeInfo>::NAME;
                format!("{}({})", name, self.0.to_string())
            }
        }
    };
}

macro_rules! py_parse {
    ($pytype:ident, $from:ident) => {
        #[pymethods]
        impl $pytype {
            #[new]
            fn new(s: String) -> PyResult<$pytype> {
                s.parse::<$from>()
                    .map($pytype::from)
                    .map_err(|e| PyreflowException::new_err(e.to_string()))
            }
        }
    };
}

pywrap!(PySegment, api::Segment, "Segment");
pywrap!(PyVersion, api::Version, "Version");
pywrap!(PyHeader, api::Header, "Header");
pywrap!(PyRawTEXT, api::RawTEXT, "RawTEXT");
pywrap!(PyOffsets, api::ParseParameters, "Offsets");
pywrap!(
    PyStandardizedTEXT,
    api::StandardizedTEXT,
    "StandardizedTEXT"
);
pywrap!(
    PyStandardizedDataset,
    api::StandardizedDataset,
    "StandardizedDataset"
);
pywrap!(PyCoreTEXT2_0, api::CoreTEXT2_0, "CoreTEXT2_0");
pywrap!(PyCoreTEXT3_0, api::CoreTEXT3_0, "CoreTEXT3_0");
pywrap!(PyCoreTEXT3_1, api::CoreTEXT3_1, "CoreTEXT3_1");
pywrap!(PyCoreTEXT3_2, api::CoreTEXT3_2, "CoreTEXT3_2");

pywrap!(PyCoreDataset2_0, api::CoreDataset2_0, "CoreDataset2_0");
pywrap!(PyCoreDataset3_0, api::CoreDataset3_0, "CoreDataset3_0");
pywrap!(PyCoreDataset3_1, api::CoreDataset3_1, "CoreDataset3_1");
pywrap!(PyCoreDataset3_2, api::CoreDataset3_2, "CoreDataset3_2");

pywrap!(PyMeasurement2_0, api::Measurement2_0, "Measurement2_0");
pywrap!(PyMeasurement3_0, api::Measurement3_0, "Measurement3_0");
pywrap!(PyMeasurement3_1, api::Measurement3_1, "Measurement3_1");
pywrap!(PyMeasurement3_2, api::Measurement3_2, "Measurement3_2");
pywrap!(PyDatePattern, DatePattern, "DatePattern");

pywrap!(PyShortname, Shortname, "Shortname");
pywrap!(PyNumRangeSetter, api::NumRangeSetter, "NumRangeSetter");
pywrap!(
    PyAsciiRangeSetter,
    api::AsciiRangeSetter,
    "AsciiRangeSetter"
);
pywrap!(
    PyMixedColumnSetter,
    api::MixedColumnSetter,
    "MixedColumnSetter"
);
pywrap!(PyTimePattern, TimePattern, "TimePattern");
pywrap!(PyUnicode, api::Unicode, "Unicode");
pywrap!(PyShortnamePrefix, ShortnamePrefix, "ShortnamePrefix");
pywrap!(PyNonStdMeasPattern, NonStdMeasPattern, "NonStdMeasPattern");
pywrap!(PyNonStdMeasKey, NonStdMeasKey, "NonStdMeasKey");
pywrap!(PyNonStdKey, NonStdKey, "NonStdKey");
pywrap!(PyTEXTDelim, TEXTDelim, "TEXTDelim");
pywrap!(PyCytSetter, MetaKwSetter<api::Cyt>, "CytSetter");
pywrap!(PyCalibration3_1, api::Calibration3_1, "Calibration3_1");
pywrap!(PyCalibration3_2, api::Calibration3_2, "Calibration3_2");
pywrap!(PyMeasurementType, api::MeasurementType, "MeasurementType");
pywrap!(PyFeature, api::Feature, "Feature");
pywrap!(PyPositiveFloat, PositiveFloat, "PositiveFloat");
pywrap!(PyNonNegFloat, NonNegFloat, "NonNegFloat");

impl From<api::DetectorVoltage> for PyNonNegFloat {
    fn from(value: api::DetectorVoltage) -> Self {
        value.0.into()
    }
}

impl From<PyNonNegFloat> for api::DetectorVoltage {
    fn from(value: PyNonNegFloat) -> Self {
        value.0.into()
    }
}

impl From<api::Gain> for PyPositiveFloat {
    fn from(value: api::Gain) -> Self {
        value.0.into()
    }
}

impl From<PyPositiveFloat> for api::Gain {
    fn from(value: PyPositiveFloat) -> Self {
        value.0.into()
    }
}

impl From<api::Vol> for PyNonNegFloat {
    fn from(value: api::Vol) -> Self {
        value.0.into()
    }
}

impl From<PyNonNegFloat> for api::Vol {
    fn from(value: PyNonNegFloat) -> Self {
        value.0.into()
    }
}

pywrap!(PyEndian, Endian, "Endian");
pywrap!(PyByteOrd, ByteOrd, "ByteOrd");
pywrap!(PyMode, api::Mode, "Mode");
pywrap!(PyOriginality, api::Originality, "Originality");
pywrap!(PyAlphaNumType, api::AlphaNumType, "AlphaNumType");
pywrap!(PyScale, Scale, "Scale");
pywrap!(PySpillover, Spillover, "Spillover");

py_parse!(PyDatePattern, DatePattern);
py_disp!(PyDatePattern);

py_parse!(PyShortname, Shortname);
py_disp!(PyShortname);
py_eq!(PyShortname);

impl Hash for PyShortname {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

py_parse!(PyNonStdMeasPattern, NonStdMeasPattern);
py_disp!(PyNonStdMeasPattern);

#[pymethods]
impl PyTEXTDelim {
    #[new]
    fn new(x: u8) -> PyResult<PyTEXTDelim> {
        TEXTDelim::new(x)
            .map(PyTEXTDelim::from)
            .map_err(|e| PyreflowException::new_err(e.to_string()))
    }
}

py_ord!(PyVersion);
py_disp!(PyVersion);

#[pymethods]
impl PySegment {
    fn begin(&self) -> u32 {
        self.0.begin()
    }

    fn end(&self) -> u32 {
        self.0.end()
    }

    fn nbytes(&self) -> u32 {
        self.0.nbytes()
    }

    fn __repr__(&self) -> String {
        format!("({},{})", self.begin(), self.end())
    }
}

#[pymethods]
impl PyHeader {
    #[getter]
    fn version(&self) -> PyVersion {
        self.0.version.into()
    }

    #[getter]
    fn text(&self) -> PySegment {
        self.0.text.into()
    }

    #[getter]
    fn data(&self) -> PySegment {
        self.0.data.into()
    }

    #[getter]
    fn analysis(&self) -> PySegment {
        self.0.analysis.into()
    }
}

#[pymethods]
impl PyRawTEXT {
    #[getter]
    fn version(&self) -> PyVersion {
        self.0.version.into()
    }

    #[getter]
    fn offsets(&self) -> PyOffsets {
        self.0.parse.clone().into()
    }

    // TODO this is a gotcha because if someone tries to modify a keyword like
    // 'std.keywords.cells = "2112"' then it the modification will actually be
    // done to a copy of 'keywords' rather than 'std'.
    #[getter]
    fn keywords<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        self.0.keywords.clone().into_py_dict(py)
    }
}

#[pymethods]
impl PyOffsets {
    #[getter]
    fn prim_text(&self) -> PySegment {
        self.0.prim_text.into()
    }

    #[getter]
    fn supp_text(&self) -> Option<PySegment> {
        self.0.supp_text.map(|x| x.into())
    }

    #[getter]
    fn data(&self) -> PySegment {
        self.0.data.into()
    }

    #[getter]
    fn analysis(&self) -> PySegment {
        self.0.analysis.into()
    }

    #[getter]
    fn nextdata(&self) -> Option<u32> {
        self.0.nextdata
    }
}

#[pymethods]
impl PyStandardizedTEXT {
    #[getter]
    fn offsets(&self) -> PyOffsets {
        self.0.parse.clone().into()
    }

    #[getter]
    fn deviant<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        self.0.deviant.clone().into_py_dict(py)
    }

    #[getter]
    fn remainder<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        self.0.remainder.clone().into_py_dict(py)
    }

    #[getter]
    fn standardized(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match &self.0.standardized {
            // TODO this copies all data from the "union type" into a new
            // version-specific type. This might not be a big deal, but these
            // types might be rather large with lots of strings.
            api::AnyCoreTEXT::FCS2_0(x) => PyCoreTEXT2_0::from((**x).clone()).into_py_any(py),
            api::AnyCoreTEXT::FCS3_0(x) => PyCoreTEXT3_0::from((**x).clone()).into_py_any(py),
            api::AnyCoreTEXT::FCS3_1(x) => PyCoreTEXT3_1::from((**x).clone()).into_py_any(py),
            api::AnyCoreTEXT::FCS3_2(x) => PyCoreTEXT3_2::from((**x).clone()).into_py_any(py),
        }
    }
}

#[pymethods]
impl PyStandardizedDataset {
    #[getter]
    fn offsets(&self) -> PyOffsets {
        self.0.parse.clone().into()
    }

    #[getter]
    fn deviant<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        self.0.deviant.clone().into_py_dict(py)
    }

    #[getter]
    fn text(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        match &self.0.dataset {
            // TODO this copies all data from the "union type" into a new
            // version-specific type. This might not be a big deal, but these
            // types might be rather large with lots of strings.
            api::AnyCoreDataset::FCS2_0(x) => PyCoreDataset2_0::from(x.clone()).into_py_any(py),
            api::AnyCoreDataset::FCS3_0(x) => PyCoreDataset3_0::from(x.clone()).into_py_any(py),
            api::AnyCoreDataset::FCS3_1(x) => PyCoreDataset3_1::from(x.clone()).into_py_any(py),
            api::AnyCoreDataset::FCS3_2(x) => PyCoreDataset3_2::from(x.clone()).into_py_any(py),
        }
    }

    #[getter]
    fn data(&self) -> PyDataFrame {
        // NOTE polars Series is a wrapper around an Arc so clone just
        // increments the ref count for each column rather than "deepcopy" the
        // whole dataset.
        PyDataFrame(self.0.dataset.as_data().clone())
    }
}

macro_rules! get_set_str {
    ($pytype:ident, $($rest:ident,)+ [$($root:ident),*], $get:ident, $set:ident, $field:ident) => {
        get_set_str!($pytype, [$($root),*], $get, $set, $field);
        get_set_str!($($rest,)+ [$($root),*], $get, $set, $field);
    };

    ($pytype:ident, [$($root:ident),*], $get:ident, $set:ident, $field:ident) => {
        #[pymethods]
        impl $pytype {
            #[getter]
            fn $get(&self) -> Option<String> {
                self.0.$($root.)*$field.as_ref_opt().map(|x| x.clone().into())
            }

            #[setter]
            fn $set(&mut self, s: Option<String>) {
                self.0.$($root.)*$field = s.map(|x| x.into()).into()
            }
        }
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
}

macro_rules! meas_get_set {
    ($pytype:ident, $($rest:ident,)+ [$($root:ident),*], $get:ident, $set:ident, $t:path) => {
        meas_get_set!($pytype, [$($root),*], $get, $set, $t);
        meas_get_set!($($rest,)+ [$($root),*], $get, $set, $t);
    };

    ($pytype:ident, [$($root:ident),*], $get:ident, $set:ident, $t:path) => {
        #[pymethods]
        impl $pytype {
            #[getter]
            fn $get(&self) -> Vec<(usize, Option<$t>)> {
                self.0
                    .$($root.)*
                    $get()
                    .into_iter()
                    .map(|(i, x)| (i.into(), x.map(|y| y.clone().into())))
                    .collect()
            }

            #[setter]
            fn $set(&mut self, xs: Vec<Option<$t>>) -> bool {
                self.0
                    .$($root.)*
                    $set(xs.into_iter().map(|x| x.map(|y| y.into())).collect())
            }
        }
    };
}

macro_rules! convert_methods {
    ($pytype:ident, $inner:ident, $([$fn:ident, $to:ident]),+) => {
        #[pymethods]
        impl $pytype {
            $(
                fn $fn(&self) -> PyResult<$to> {
                    let new = self.0.clone().$inner();
                    handle_errors(new.map_err(|e| e.into()))
                }
            )*
        }
    };
}

convert_methods!(
    PyCoreTEXT2_0,
    try_convert,
    [version_3_0, PyCoreTEXT3_0],
    [version_3_1, PyCoreTEXT3_1],
    [version_3_2, PyCoreTEXT3_2]
);

convert_methods!(
    PyCoreTEXT3_0,
    try_convert,
    [version_2_0, PyCoreTEXT2_0],
    [version_3_1, PyCoreTEXT3_1],
    [version_3_2, PyCoreTEXT3_2]
);

convert_methods!(
    PyCoreTEXT3_1,
    try_convert,
    [version_2_0, PyCoreTEXT2_0],
    [version_3_0, PyCoreTEXT3_0],
    [version_3_2, PyCoreTEXT3_2]
);

convert_methods!(
    PyCoreTEXT3_2,
    try_convert,
    [version_2_0, PyCoreTEXT2_0],
    [version_3_0, PyCoreTEXT3_0],
    [version_3_1, PyCoreTEXT3_1]
);

convert_methods!(
    PyCoreDataset2_0,
    try_convert,
    [version_3_0, PyCoreDataset3_0],
    [version_3_1, PyCoreDataset3_1],
    [version_3_2, PyCoreDataset3_2]
);

convert_methods!(
    PyCoreDataset3_0,
    try_convert,
    [version_2_0, PyCoreDataset2_0],
    [version_3_1, PyCoreDataset3_1],
    [version_3_2, PyCoreDataset3_2]
);

convert_methods!(
    PyCoreDataset3_1,
    try_convert,
    [version_2_0, PyCoreDataset2_0],
    [version_3_0, PyCoreDataset3_0],
    [version_3_2, PyCoreDataset3_2]
);

convert_methods!(
    PyCoreDataset3_2,
    try_convert,
    [version_2_0, PyCoreDataset2_0],
    [version_3_0, PyCoreDataset3_0],
    [version_3_1, PyCoreDataset3_1]
);

#[pymethods]
impl PyCoreTEXT2_0 {
    #[new]
    fn new(datatype: PyAlphaNumType, byteord: PyByteOrd, mode: PyMode) -> Self {
        api::CoreTEXT2_0::new(datatype.into(), byteord.into(), mode.into()).into()
    }
}

#[pymethods]
impl PyCoreTEXT3_0 {
    #[new]
    fn new(datatype: PyAlphaNumType, byteord: PyByteOrd, mode: PyMode) -> Self {
        api::CoreTEXT3_0::new(datatype.into(), byteord.into(), mode.into()).into()
    }

    #[getter]
    fn get_unicode(&self) -> Option<PyUnicode> {
        self.0
            .metadata
            .specific
            .unicode
            .as_ref_opt()
            .map(|x| x.clone().into())
    }

    #[setter]
    fn set_unicode(&mut self, x: Option<PyUnicode>) {
        self.0.metadata.specific.unicode = x.map(|y| y.into()).into();
    }
}

#[pymethods]
impl PyCoreTEXT3_1 {
    #[new]
    fn new(datatype: PyAlphaNumType, is_big: bool, mode: PyMode) -> Self {
        api::CoreTEXT3_1::new(datatype.into(), is_big, mode.into()).into()
    }
}

#[pymethods]
impl PyCoreTEXT3_2 {
    #[new]
    fn new(datatype: PyAlphaNumType, is_big: bool, cyt: String) -> Self {
        api::CoreTEXT3_2::new(datatype.into(), is_big, cyt).into()
    }

    #[getter]
    fn get_datetime_begin(&self) -> Option<DateTime<FixedOffset>> {
        self.0.metadata.specific.datetimes.begin_naive()
    }

    #[setter]
    fn set_datetime_begin(&mut self, x: Option<DateTime<FixedOffset>>) -> PyResult<()> {
        self.0
            .metadata
            .specific
            .datetimes
            .set_begin_naive(x)
            .map_err(|e| PyreflowException::new_err(e.to_string()))
    }

    #[getter]
    fn get_datetime_end(&self) -> Option<DateTime<FixedOffset>> {
        self.0.metadata.specific.datetimes.end_naive()
    }

    #[setter]
    fn set_datetime_end(&mut self, x: Option<DateTime<FixedOffset>>) -> PyResult<()> {
        self.0
            .metadata
            .specific
            .datetimes
            .set_end_naive(x)
            .map_err(|e| PyreflowException::new_err(e.to_string()))
    }

    #[getter]
    fn get_datatypes(&self) -> Vec<PyAlphaNumType> {
        self.0.datatypes().into_iter().map(|x| x.into()).collect()
    }

    #[getter]
    fn get_cyt(&self) -> String {
        self.0.metadata.specific.cyt.0.clone()
    }

    #[setter]
    fn set_cyt(&mut self, x: String) {
        self.0.metadata.specific.cyt = x.into()
    }

    #[getter]
    fn get_unstainedinfo(&self) -> Option<String> {
        self.0
            .metadata
            .specific
            .unstained
            .unstainedinfo
            .0
            .as_ref()
            .map(|x| x.clone().into())
    }

    #[setter]
    fn set_unstainedinfo(&mut self, x: Option<String>) {
        self.0.metadata.specific.unstained.unstainedinfo = x.map(|x| x.into()).into()
    }

    #[getter]
    fn get_unstained_centers(&self) -> Option<HashMap<PyShortname, f32>> {
        self.0.unstained_centers().map(|x| {
            <HashMap<Shortname, f32>>::from(x.clone())
                .into_iter()
                .map(|(k, v)| (k.into(), v))
                .collect()
        })
    }

    fn insert_unstained_center(&mut self, k: PyShortname, v: f32) -> Option<f32> {
        self.0.insert_unstained_center(k.into(), v)
    }

    fn remove_unstained_center(&mut self, k: PyShortname) -> Option<f32> {
        self.0.remove_unstained_center(&k.into())
    }

    fn clear_unstained_centers(&mut self) {
        self.0.clear_unstained_centers()
    }

    fn set_data_mixed(&mut self, cs: Vec<PyMixedColumnSetter>) -> bool {
        self.0
            .set_data_mixed(cs.into_iter().map(|x| x.into()).collect())
    }

    // TODO make function to add DATA/ANALYSIS, which will convert this to a CoreDataset
}

macro_rules! integer_2_0_methods {
    ($pytype:ident, $($rest:ident),+; $($root:ident),*) => {
        integer_2_0_methods!($pytype; $($root),*);
        integer_2_0_methods!($($rest),+; $($root),*);
    };

    ($pytype:ident; $($root:ident),*) => {
        #[pymethods]
        impl $pytype {
            fn set_data_integer(&mut self, rs: Vec<u64>, byteord: PyByteOrd) -> bool {
                self.0.$($root.)*set_data_integer(rs, byteord.into())
            }
        }
    };
}

integer_2_0_methods!(PyCoreTEXT2_0, PyCoreTEXT3_0;);
integer_2_0_methods!(PyCoreDataset2_0, PyCoreDataset3_0; text);

macro_rules! shortnames_methods {
    ($pytype:ident, $($rest:ident),+; $($root:ident),*) => {
        shortnames_methods!($pytype; $($root),*);
        shortnames_methods!($($rest),+; $($root),*);
    };

    ($pytype:ident; $($root:ident),*) => {
        #[pymethods]
        impl $pytype {
            pub fn set_measurement_shortnames_maybe(
                &mut self,
                ns: Vec<Option<PyShortname>>,
            ) -> PyResult<()> {
                let xs = ns.into_iter().map(|n| n.map(|y| y.into())).collect();
                self.0
                    .$($root.)*
                    set_measurement_shortnames_maybe(xs)
                    .map_err(|e| PyreflowException::new_err(e.to_string()))
                    .map(|_| ())
            }
        }
    };
}

shortnames_methods!(PyCoreTEXT2_0, PyCoreTEXT3_0;);
shortnames_methods!(PyCoreDataset2_0, PyCoreDataset3_0; text);

macro_rules! integer_methods {
    ($pytype:ident, $($rest:ident),+; $($root:ident),*) => {
        integer_methods!($pytype; $($root),*);
        integer_methods!($($rest),+; $($root),*);
    };

    ($pytype:ident; $($root:ident),*) => {
        #[pymethods]
        impl $pytype {
            fn set_data_integer(&mut self, rs: Vec<PyNumRangeSetter>) -> bool {
                self.0
                    .$($root.)*
                    set_data_integer(rs.into_iter().map(|x| x.into()).collect())
            }
        }
    };
}

integer_methods!(PyCoreTEXT3_1, PyCoreTEXT3_2;);
integer_methods!(PyCoreDataset3_1, PyCoreDataset3_2; text);

macro_rules! endian_methods {
    ($pytype:ident, $($rest:ident),+; $($root:ident),*) => {
        endian_methods!($pytype; $($root),*);
        endian_methods!($($rest),+; $($root),*);
    };

    ($pytype:ident; $($root:ident),*) => {
        #[pymethods]
        impl $pytype {
            #[getter]
            fn get_big_endian(&self) -> bool {
                self.0.$($root.)*metadata.specific.byteord == Endian::Big
            }

            #[setter]
            fn set_big_endian(&mut self, is_big: bool) {
                self.0.$($root.)*metadata.specific.byteord = Endian::is_big(is_big);
            }
        }
    };
}

endian_methods!(PyCoreTEXT3_1, PyCoreTEXT3_2;);
endian_methods!(PyCoreDataset3_1, PyCoreDataset3_2; text);

macro_rules! scales_methods {
    ($pytype:ident, $($rest:ident),+; $($root:ident),*) => {
        scales_methods!($pytype; $($root),*);
        scales_methods!($($rest),+; $($root),*);
    };

    ($pytype:ident; $($root:ident),*) => {
        #[pymethods]
        impl $pytype {
            #[getter]
            fn get_all_scales(&self) -> Vec<PyScale> {
                self.0.$($root.)*all_scales().into_iter().map(|x| x.into()).collect()
            }

            #[getter]
            fn get_scales(&self) -> Vec<(usize, PyScale)> {
                self.0
                    .$($root.)*
                    scales()
                    .into_iter()
                    .map(|(i, x)| (i.into(), x.into()))
                    .collect()
            }

            #[setter]
            fn set_scales(&mut self, xs: Vec<PyScale>) -> bool {
                self.0
                    .$($root.)*
                    set_scales(xs.into_iter().map(|x| x.into()).collect())
            }
        }
    };
}

macro_rules! timestep_methods {
    ($pytype:ident, $($rest:ident),+; $($root:ident),*) => {
        timestep_methods!($pytype; $($root),*);
        timestep_methods!($($rest),+; $($root),*);
    };

    ($pytype:ident; $($root:ident),*) => {
        get_set_str!($pytype, [$($root,)* metadata, specific], get_cytsn, set_cytsn, cytsn);

        #[pymethods]
        impl $pytype {
            #[getter]
            fn get_timestep(&self) -> Option<PyPositiveFloat> {
                self.0
                    .$($root.)*
                    measurements_named_vec()
                    .as_center()
                    .and_then(|x| x.value.specific.timestep())
                    .map(|x| x.0.into())
            }

            #[setter]
            fn set_timestep(&mut self, x: PyPositiveFloat) -> bool {
                self.0
                    .$($root.)*
                    as_center_mut()
                    .map(|y| y.value.specific.set_timestep(api::Timestep(x.into())))
                    .is_some()
            }
        }
    };
}

macro_rules! wavelength_methods {
    ($pytype:ident, $($rest:ident),+; $($root:ident),*) => {
        wavelength_methods!($pytype; $($root),*);
        wavelength_methods!($($rest),+; $($root),*);
    };

    ($pytype:ident; $($root:ident),*) => {
        #[pymethods]
        impl $pytype {
            #[getter]
            fn get_wavelengths(&self) -> Vec<(usize, Option<u32>)> {
                self.0
                    .$($root.)*
                    wavelengths()
                    .into_iter()
                    .map(|(i, x)| (i.into(), x.map(|y| y.0)))
                    .collect()
            }

            #[setter]
            fn set_wavelengths(&mut self, xs: Vec<Option<u32>>) -> bool {
                self.0
                    .$($root.)*
                    set_wavelengths(xs.into_iter().map(|x| x.map(|y| y.into())).collect())
            }
        }
    };
}

macro_rules! wavelengths_methods {
    ($pytype:ident, $($rest:ident),+; $($root:ident),*) => {
        wavelengths_methods!($pytype; $($root),*);
        wavelengths_methods!($($rest),+; $($root),*);
    };

    ($pytype:ident; $($root:ident),*) => {
        #[pymethods]
        impl $pytype {
            #[getter]
            fn get_wavelengths(&self) -> Vec<(usize, Vec<u32>)> {
                self.0
                    .$($root.)*
                    wavelengths()
                    .into_iter()
                    .map(|(i, x)| {
                        (
                            i.into(),
                            x.map(|y| y.0.iter().copied().collect()).unwrap_or_default(),
                        )
                    })
                    .collect()
            }

            #[setter]
            fn set_wavelengths(&mut self, xs: Vec<Vec<u32>>) -> bool {
                self.0.
                    $($root.)*
                    set_wavelengths(
                    xs.into_iter()
                        .map(|x| NonEmpty::from_vec(x).map(api::Wavelengths))
                        .collect(),
                )
            }
        }
    };
}

macro_rules! modification_methods {
    ($($pytype:ident),+; $($root:ident),*) => {
        get_set_copied!(
            $($pytype,)*
            [$($root,)* metadata, specific, modification],
            get_originality,
            set_originality,
            originality,
            PyOriginality
        );

        get_set_copied!(
            $($pytype,)*
            [$($root,)* metadata, specific, modification],
            get_last_modified,
            set_last_modified,
            last_modified,
            NaiveDateTime
        );

        get_set_str!(
            $($pytype,)*
            [$($root,)* metadata, specific, modification],
            get_last_modifier,
            set_last_modifier,
            last_modifier
        );
    };
}

macro_rules! carrier_methods {
    ($($pytype:ident),*; $($root:ident),*) => {
        get_set_str!(
            $($pytype,)*
            [$($root,)* metadata, specific, carrier],
            get_carriertype,
            set_carriertype,
            carriertype
        );

        get_set_str!(
            $($pytype,)*
            [$($root,)* metadata, specific, carrier],
            get_carrierid,
            set_carrierid,
            carrierid
        );

        get_set_str!(
            $($pytype,)*
            [$($root,)* metadata, specific, carrier],
            get_locationid,
            set_locationid,
            locationid
        );
    };
}

macro_rules! plate_methods {
    ($($pytype:ident),*; $($root:ident),*) => {
        get_set_str!(
            $($pytype,)*
            [$($root,)* metadata, specific, plate],
            get_wellid,
            set_wellid,
            wellid
        );

        get_set_str!(
            $($pytype,)*
            [$($root,)* metadata, specific, plate],
            get_plateid,
            set_plateid,
            plateid
        );

        get_set_str!(
            $($pytype,)*
            [$($root,)* metadata, specific, plate],
            get_platename,
            set_platename,
            platename
        );
    };
}

// macro_rules! spillover_methods {
//     ($pytype:ident, $($rest:ident),+; $($root:ident),*) => {
//         spillover_methods!($pytype; $($root),*);
//         spillover_methods!($($rest),+; $($root),*);
//     };

//     ($pytype:ident; $($root:ident),*) => {
//         #[pymethods]
//         impl $pytype {
//             #[getter]
//             fn get_spillover_matrix<'a>(
//                 &self, py:
//                 Python<'a>
//             ) -> Option<Bound<'a, PyArray2<f32>>> {
//                 self.0.spillover().map(|x| x.matrix().to_pyarray(py))
//             }

//             #[getter]
//             fn get_spillover_names(&self) -> Vec<String> {
//                 self.0
//                     .spillover()
//                     .map(|x| x.measurements())
//                     .unwrap_or_default()
//                     .iter()
//                     .map(|x| x.as_ref().to_string())
//                     .collect()
//             }

//             fn set_spillover(
//                 &mut self,
//                 ns: Vec<PyShortname>,
//                 a: PyReadonlyArray2<f32>,
//             ) -> Result<(), PyErr> {
//                 let m = a.as_matrix().into_owned();
//                 self.0
//                     .set_spillover(ns.into_iter().map(|x| x.into()).collect(), m)
//                     .map_err(|e| PyreflowException::new_err(e.to_string()))
//             }

//             fn unset_spillover(&mut self) {
//                 self.0.unset_spillover()
//             }
//         }
//     };
// }

macro_rules! vol_methods {
    ($($pytype:ident),*; $($root:ident)*) => {
        get_set_copied!(
            $($pytype,)*
            [$($root,)* metadata, specific],
            get_vol,
            set_vol,
            vol,
            PyNonNegFloat
        );
    };
}

macro_rules! common_methods {
    ($pytype:ident, $($rest:ident),+; $($root:ident),*) => {
        common_methods!($pytype; $($root),*);
        common_methods!($($rest),+; $($root),*);
    };

    ($pytype:ident; $($root:ident),*) => {
        // common metadata keywords
        get_set_copied!($pytype, [$($root,)*metadata], get_abrt, set_abrt, abrt, u32);
        get_set_copied!($pytype, [$($root,)*metadata], get_lost, set_lost, lost, u32);

        get_set_str!($pytype, [$($root,)*metadata], get_cells, set_cells, cells);
        get_set_str!($pytype, [$($root,)*metadata], get_com,   set_com,   com);
        get_set_str!($pytype, [$($root,)*metadata], get_exp,   set_exp,   exp);
        get_set_str!($pytype, [$($root,)*metadata], get_fil,   set_fil,   fil);
        get_set_str!($pytype, [$($root,)*metadata], get_inst,  set_inst,  inst);
        get_set_str!($pytype, [$($root,)*metadata], get_op,    set_op,    op);
        get_set_str!($pytype, [$($root,)*metadata], get_proj,  set_proj,  proj);
        get_set_str!($pytype, [$($root,)*metadata], get_smno,  set_smno,  smno);
        get_set_str!($pytype, [$($root,)*metadata], get_src,   set_src,   src);
        get_set_str!($pytype, [$($root,)*metadata], get_sys,   set_sys,   sys);

        // common measurement keywords
        meas_get_set!($pytype, [$($root),*], filters,           set_filters,           String);
        meas_get_set!($pytype, [$($root),*], powers,            set_powers,            u32);
        meas_get_set!($pytype, [$($root),*], detector_types,    set_detector_types,    String);
        meas_get_set!($pytype, [$($root),*], percents_emitted,  set_percents_emitted,  String);
        meas_get_set!($pytype, [$($root),*], detector_voltages, set_detector_voltages, PyNonNegFloat);

        #[pymethods]
        impl $pytype {
            // TODO add way to remove nonstandard
            #[pyo3(signature = (want_req=None, want_meta=None))]
            fn raw_keywords<'py>(
                &self,
                py: Python<'py>,
                want_req: Option<bool>,
                want_meta: Option<bool>,
            ) -> PyResult<Bound<'py, PyDict>> {
                self.0.$($root.)*raw_keywords(want_req, want_meta).clone().into_py_dict(py)
            }

            #[getter]
            fn par(&self) -> usize {
                self.0.$($root.)*par().0
            }

            fn insert_nonstandard(&mut self, k: PyNonStdKey, v: String) -> Option<String> {
                self.0.$($root.)*metadata.nonstandard_keywords.insert(k.into(), v)
            }

            fn remove_nonstandard(&mut self, k: PyNonStdKey) -> Option<String> {
                self.0.$($root.)*metadata.nonstandard_keywords.remove(&k.into())
            }

            fn get_nonstandard(&mut self, k: PyNonStdKey) -> Option<String> {
                self.0.$($root.)*metadata.nonstandard_keywords.get(&k.into()).cloned()
            }

            fn insert_meas_nonstandard(
                &mut self,
                xs: Vec<(PyNonStdKey, String)>,
            ) -> Option<Vec<Option<String>>> {
                let ys = xs.into_iter().map(|(k, v)| (k.into(), v)).collect();
                self.0.$($root.)*insert_meas_nonstandard(ys)
            }

            fn remove_meas_nonstandard(&mut self, ks: Vec<PyNonStdKey>) -> Option<Vec<Option<String>>> {
                let ys: Vec<_> = ks.into_iter().map(|k| k.into()).collect();
                self.0.$($root.)*remove_meas_nonstandard(ys.iter().collect())
            }

            fn get_meas_nonstandard(&mut self, ks: Vec<PyNonStdKey>) -> Option<Vec<Option<String>>> {
                let ys: Vec<_> = ks.into_iter().map(|k| k.into()).collect();
                self.0
                    .$($root.)*
                    get_meas_nonstandard(&ys)
                    .map(|rs| rs.into_iter().map(|r| r.cloned()).collect())
            }

            #[getter]
            fn get_btim(&self) -> Option<NaiveTime> {
                self.0.$($root.)*metadata.specific.timestamps.btim_naive()
            }

            #[setter]
            fn set_btim(&mut self, x: Option<NaiveTime>) -> PyResult<()> {
                self.0
                    .$($root.)*
                    metadata
                    .specific
                    .timestamps
                    .set_btim_naive(x)
                    .map_err(|e| PyreflowException::new_err(e.to_string()))
            }

            #[getter]
            fn get_etim(&self) -> Option<NaiveTime> {
                self.0.$($root.)*metadata.specific.timestamps.etim_naive()
            }

            #[setter]
            fn set_etim(&mut self, x: Option<NaiveTime>) -> PyResult<()> {
                self.0
                    .$($root.)*
                    metadata
                    .specific
                    .timestamps
                    .set_etim_naive(x)
                    .map_err(|e| PyreflowException::new_err(e.to_string()))
            }

            #[getter]
            fn get_date(&self) -> Option<NaiveDate> {
                self.0.$($root.)*metadata.specific.timestamps.date_naive()
            }

            #[setter]
            fn set_date(&mut self, x: Option<NaiveDate>) -> PyResult<()> {
                self.0
                    .$($root.)*
                    metadata
                    .specific
                    .timestamps
                    .set_date_naive(x)
                    .map_err(|e| PyreflowException::new_err(e.to_string()))
            }

            #[getter]
            fn trigger_name(&self) -> Option<PyShortname> {
                self.0.$($root.)*trigger_name().map(|x| x.clone().into())
            }

            #[getter]
            fn trigger_threshold(&self) -> Option<u32> {
                self.0.$($root.)*trigger_threshold()
            }

            #[setter]
            fn set_trigger_name(&mut self, n: PyShortname) -> bool {
                self.0.$($root.)*set_trigger_name(n.into())
            }

            #[setter]
            fn set_trigger_threshold(&mut self, x: u32) -> bool {
                self.0.$($root.)*set_trigger_threshold(x)
            }

            fn clear_trigger(&mut self) {
                self.0.$($root.)*clear_trigger()
            }

            fn set_time_channnel(&mut self, n: PyShortname) -> PyResult<()> {
                self.0.$($root.)*set_time_channel(&n.into()).map_err(|es| {
                    let f = es.into_iter().map(|e| e.to_string()).join(", ");
                    let s = format!("Error(s) when converting measurement to time: {f}");
                    PyreflowException::new_err(s)
                })
            }

            fn unset_time_channel(&mut self) -> bool {
                self.0.$($root.)*unset_time_channel()
            }

            #[getter]
            fn get_bytes(&self) -> Option<Vec<u8>> {
                self.0.$($root.)*bytes()
            }

            #[getter]
            fn get_ranges(&self) -> Vec<String> {
                // TODO strings, lame
                self.0.$($root.)*ranges().iter().map(|r| r.as_ref().to_string()).collect()
            }

            #[getter]
            fn get_longnames(&self) -> Vec<Option<String>> {
                self.0
                    .$($root.)*
                    longnames()
                    .into_iter()
                    .map(|x| x.map(|y| y.0.to_string()))
                    .collect()
            }

            #[setter]
            fn set_longnames(&mut self, ns: Vec<Option<String>>) -> bool {
                self.0.$($root.)*set_longnames(ns)
            }

            #[getter]
            fn shortnames_maybe(&self) -> Vec<Option<PyShortname>> {
                self.0
                    .$($root.)*
                    shortnames_maybe()
                    .into_iter()
                    .map(|x| x.map(|y| y.clone().into()))
                    .collect()
            }

            #[getter]
            fn all_shortnames(&self) -> Vec<PyShortname> {
                self.0
                    .$($root.)*
                    all_shortnames()
                    .into_iter()
                    .map(|x| x.into())
                    .collect()
            }

            #[setter]
            fn set_all_shortnames(&mut self, ns: Vec<PyShortname>) -> PyResult<()> {
                self.0
                    .$($root.)*
                    set_all_shortnames(ns.into_iter().map(|x| x.into()).collect())
                    .map_err(|e| PyreflowException::new_err(e.to_string()))
                    .map(|_| ())
            }

            fn set_data_f32(&mut self, ranges: Vec<f32>) -> bool {
                self.0.$($root.)*set_data_f32(ranges)
            }

            fn set_data_f64(&mut self, ranges: Vec<f64>) -> bool {
                self.0.$($root.)*set_data_f64(ranges)
            }

            fn set_data_ascii(&mut self, rs: Vec<PyAsciiRangeSetter>) -> bool {
                self.0
                    .$($root.)*
                    set_data_ascii(rs.into_iter().map(|x| x.into()).collect())
            }

            fn set_data_delimited(&mut self, ranges: Vec<u64>) -> bool {
                self.0.$($root.)*set_data_delimited(ranges)
            }
        }
    };
}

common_methods!(PyCoreTEXT2_0, PyCoreTEXT3_0, PyCoreTEXT3_1, PyCoreTEXT3_2;);
common_methods!(PyCoreDataset2_0, PyCoreDataset3_0, PyCoreDataset3_1, PyCoreDataset3_2; text);

meas_get_set!(
    PyCoreTEXT3_0,
    PyCoreTEXT3_1,
    PyCoreTEXT3_2,
    [],
    gains,
    set_gains,
    PyPositiveFloat
);
meas_get_set!(
    PyCoreDataset3_0,
    PyCoreDataset3_1,
    PyCoreDataset3_2,
    [text],
    gains,
    set_gains,
    PyPositiveFloat
);

meas_get_set!(PyCoreTEXT3_2, [], det_names, set_det_names, String);
meas_get_set!(PyCoreDataset3_2, [text], det_names, set_det_names, String);
// TODO make sure we can make a calibration object

meas_get_set!(
    PyCoreTEXT3_1,
    [],
    calibrations,
    set_calibrations,
    PyCalibration3_1
);
meas_get_set!(
    PyCoreDataset3_1,
    [text],
    calibrations,
    set_calibrations,
    PyCalibration3_1
);

meas_get_set!(
    PyCoreTEXT3_2,
    [],
    calibrations,
    set_calibrations,
    PyCalibration3_2
);
meas_get_set!(
    PyCoreDataset3_2,
    [text],
    calibrations,
    set_calibrations,
    PyCalibration3_2
);

meas_get_set!(PyCoreTEXT3_2, [], tags, set_tags, String);
meas_get_set!(PyCoreDataset3_2, [text], tags, set_tags, String);

meas_get_set!(
    PyCoreTEXT3_2,
    [],
    measurement_types,
    set_measurement_types,
    PyMeasurementType
);
meas_get_set!(
    PyCoreDataset3_2,
    [text],
    measurement_types,
    set_measurement_types,
    PyMeasurementType
);

meas_get_set!(PyCoreTEXT3_2, [], features, set_features, PyFeature);
meas_get_set!(PyCoreDataset3_2, [text], features, set_features, PyFeature);

meas_get_set!(PyCoreTEXT3_2, [], analytes, set_analytes, String);
meas_get_set!(PyCoreDataset3_2, [text], analytes, set_analytes, String);

wavelength_methods!(PyCoreTEXT2_0, PyCoreTEXT3_0;);
wavelength_methods!(PyCoreDataset2_0, PyCoreDataset3_0; text);

wavelengths_methods!(PyCoreTEXT3_1, PyCoreTEXT3_2;);
wavelengths_methods!(PyCoreDataset3_1, PyCoreDataset3_2; text);

// spillover_methods!(PyCoreTEXT3_1, PyCoreTEXT3_2;);
// spillover_methods!(PyCoreDataset3_1, PyCoreDataset3_2; text);

plate_methods!(PyCoreTEXT3_1, PyCoreTEXT3_2;);
plate_methods!(PyCoreDataset3_1, PyCoreDataset3_2; text);

carrier_methods!(PyCoreTEXT3_2;);
carrier_methods!(PyCoreDataset3_2; text);

scales_methods!(PyCoreTEXT3_0, PyCoreTEXT3_1, PyCoreTEXT3_2;);
scales_methods!(PyCoreDataset3_0, PyCoreDataset3_1, PyCoreDataset3_2; text);

get_set_str!(
    PyCoreTEXT2_0,
    PyCoreTEXT3_0,
    PyCoreTEXT3_1,
    [metadata, specific],
    get_cyt,
    set_cyt,
    cyt
);
get_set_str!(
    PyCoreDataset2_0,
    PyCoreDataset3_0,
    PyCoreDataset3_1,
    [text, metadata, specific],
    get_cyt,
    set_cyt,
    cyt
);

get_set_str!(
    PyCoreTEXT3_2,
    [metadata, specific],
    get_flowrate,
    set_flowrate,
    flowrate
);
get_set_str!(
    PyCoreDataset3_2,
    [text, metadata, specific],
    get_flowrate,
    set_flowrate,
    flowrate
);

modification_methods!(PyCoreTEXT3_1, PyCoreTEXT3_2;);
modification_methods!(PyCoreDataset3_1, PyCoreDataset3_2; text);

vol_methods!(PyCoreTEXT3_1, PyCoreTEXT3_2;);
vol_methods!(PyCoreDataset3_1, PyCoreDataset3_2; text);

timestep_methods!(PyCoreTEXT3_0, PyCoreTEXT3_1, PyCoreTEXT3_2;);
timestep_methods!(PyCoreDataset3_0, PyCoreDataset3_1, PyCoreDataset3_2; text);

struct PyImpureError(error::ImpureFailure);

fn handle_errors<X, Y>(res: error::ImpureResult<X>) -> PyResult<Y>
where
    Y: From<X>,
{
    handle_pure(res.map_err(PyImpureError)?)
}

// TODO use warnings_are_errors flag
fn handle_pure<X, Y>(succ: error::PureSuccess<X>) -> PyResult<Y>
where
    Y: From<X>,
{
    let (err, warn) = succ.deferred.split();
    Python::with_gil(|py| -> PyResult<()> {
        let wt = py.get_type::<PyreflowWarning>();
        for w in warn {
            let s = CString::new(w)?;
            PyErr::warn(py, &wt, &s, 0)?;
        }
        Ok(())
    })?;
    if err.is_empty() {
        Ok(succ.data.into())
    } else {
        let deferred = err.join("\n");
        let msg = format!("Errors encountered:\n{deferred}");
        Err(PyreflowException::new_err(msg))
    }
}

impl From<error::ImpureFailure> for PyImpureError {
    fn from(value: error::ImpureFailure) -> Self {
        Self(value)
    }
}

impl From<PyImpureError> for PyErr {
    fn from(err: PyImpureError) -> Self {
        let inner = err.0;
        let reason = match inner.reason {
            error::ImpureError::IO(e) => format!("IO ERROR: {e}"),
            error::ImpureError::Pure(e) => format!("CRITICAL PYREFLOW ERROR: {e}"),
        };
        let deferred = inner.deferred.into_errors().join("\n");
        let msg = format!("{reason}\n\nOther errors encountered:\n{deferred}");
        PyreflowException::new_err(msg)
    }
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
