use crate::config::StdTextReadConfig;
use crate::core::{CarrierData, ModificationData, PlateData, UnstainedData};
use crate::error::*;
use crate::validated::nonstandard::*;

use super::compensation::*;
use super::datetimes::*;
use super::keywords::*;
use super::optionalkw::*;
use super::timestamps::*;

use nalgebra::DMatrix;
use std::fmt::Display;
use std::str::FromStr;

/// A structure to look up and parse keywords in the TEXT segment
///
/// Given a hash table of keywords (as String pairs) and a configuration, this
/// provides an interface to extract keywords. If found, the keyword will be
/// removed from the table and parsed to its native type (number, list, matrix,
/// etc). If lookup or parsing fails, an error/warning will be logged within the
/// state depending on if the key is required or optional.
///
/// Errors in all cases are deferred until the end of the state's lifetime, at
/// which point the errors are returned along with the result of the computation
/// or failure (if applicable).
pub(crate) struct KwParser<'a, 'b> {
    raw_keywords: &'b mut StdKeywords,
    deferred: PureErrorBuf,
    pub(crate) conf: &'a StdTextReadConfig,
}

impl<'a, 'b> KwParser<'a, 'b> {
    /// Run a computation within a keyword lookup context which may fail.
    ///
    /// This is like 'run' except the computation may not return anything (None)
    /// in which case Err(Failure(...)) will be returned along with the reason
    /// for failure which must be given a priori.
    ///
    /// Any errors which are logged must be pushed into the state's error buffer
    /// directly, as errors are not allowed to be returned by the inner
    /// computation.
    pub(crate) fn try_run<X, F>(
        kws: &'b mut StdKeywords,
        conf: &'a StdTextReadConfig,
        reason: String,
        f: F,
    ) -> PureResult<X>
    where
        F: FnOnce(&mut Self) -> Option<X>,
    {
        let mut st = Self::from(kws, conf);
        if let Some(data) = f(&mut st) {
            Ok(PureSuccess {
                data,
                deferred: st.collect(),
            })
        } else {
            Err(st.into_failure(reason))
        }
    }

    pub(crate) fn lookup_meta_req<V>(&mut self) -> Option<V>
    where
        V: ReqMetaKey,
        V: FromStr,
        <V as FromStr>::Err: Display,
    {
        V::remove_meta_req(self.raw_keywords)
            .map_err(|e| self.deferred.push_error(e))
            .ok()
    }

    pub(crate) fn lookup_meta_opt<V>(&mut self, dep: bool) -> OptionalKw<V>
    where
        V: OptMetaKey,
        V: FromStr,
        <V as FromStr>::Err: Display,
    {
        let res = V::remove_meta_opt(self.raw_keywords);
        self.process_opt(res, V::std(), dep)
    }

    pub(crate) fn lookup_meas_req<V>(&mut self, n: MeasIdx) -> Option<V>
    where
        V: ReqMeasKey,
        V: FromStr,
        <V as FromStr>::Err: Display,
    {
        V::remove_meas_req(self.raw_keywords, n)
            .map_err(|e| self.deferred.push_error(e))
            .ok()
    }

    pub(crate) fn lookup_meas_opt<V>(&mut self, n: MeasIdx, dep: bool) -> OptionalKw<V>
    where
        V: OptMeasKey,
        V: FromStr,
        <V as FromStr>::Err: Display,
    {
        let res = V::remove_meas_opt(self.raw_keywords, n);
        self.process_opt(res, V::std(n), dep)
    }

    pub(crate) fn lookup_timestamps<T>(&mut self, dep: bool) -> Timestamps<T>
    where
        T: PartialOrd,
        T: Copy,
        Btim<T>: OptMetaKey,
        <Btim<T> as FromStr>::Err: Display,
        Etim<T>: OptMetaKey,
        <Etim<T> as FromStr>::Err: Display,
    {
        Timestamps::new(
            self.lookup_meta_opt(dep),
            self.lookup_meta_opt(dep),
            self.lookup_meta_opt(dep),
        )
        .unwrap_or_else(|e| {
            self.deferred.push_warning(e.to_string());
            Timestamps::default()
        })
    }

    pub(crate) fn lookup_datetimes(&mut self) -> Datetimes {
        let b = self.lookup_meta_opt(false);
        let e = self.lookup_meta_opt(false);
        Datetimes::new(b, e).unwrap_or_else(|w| {
            self.deferred.push_warning(w.to_string());
            Datetimes::default()
        })
    }

    pub(crate) fn lookup_modification(&mut self) -> ModificationData {
        ModificationData {
            last_modifier: self.lookup_meta_opt(false),
            last_modified: self.lookup_meta_opt(false),
            originality: self.lookup_meta_opt(false),
        }
    }

    pub(crate) fn lookup_plate(&mut self, dep: bool) -> PlateData {
        PlateData {
            wellid: self.lookup_meta_opt(dep),
            platename: self.lookup_meta_opt(dep),
            plateid: self.lookup_meta_opt(dep),
        }
    }

    pub(crate) fn lookup_carrier(&mut self) -> CarrierData {
        CarrierData {
            locationid: self.lookup_meta_opt(false),
            carrierid: self.lookup_meta_opt(false),
            carriertype: self.lookup_meta_opt(false),
        }
    }

    pub(crate) fn lookup_unstained(&mut self) -> UnstainedData {
        UnstainedData::new_unchecked(self.lookup_meta_opt(false), self.lookup_meta_opt(false))
    }

    pub(crate) fn lookup_compensation_2_0(&mut self, par: Par) -> OptionalKw<Compensation> {
        // column = src measurement
        // row = target measurement
        // These are "flipped" in 2.0, where "column" goes TO the "row"
        let n = par.0;
        let mut any_error = false;
        let mut matrix = DMatrix::<f32>::identity(n, n);
        for r in 0..n {
            for c in 0..n {
                // TODO sketchy
                let k = StdKey(format!("DFC{c}TO{r}").as_bytes().to_vec());
                if let Some(x) = self.lookup_dfc(&k) {
                    matrix[(r, c)] = x;
                } else {
                    any_error = true;
                }
            }
        }
        if any_error {
            None
        } else {
            // TODO will return none if matrix is less than 2x2, which is a
            // warning
            Compensation::new(matrix)
        }
        .into()
    }

    pub(crate) fn lookup_dfc(&mut self, k: &StdKey) -> Option<f32> {
        self.raw_keywords.remove(k).and_then(|v| {
            v.parse::<f32>()
                .inspect_err(|e| {
                    let msg = format!("{e} (key={k}, value='{v}'))");
                    self.deferred.push_warning(msg);
                })
                .ok()
        })
    }

    // pub(crate) fn lookup_all_nonstandard(&mut self) -> NonStdKeywords {
    //     let mut ns = HashMap::new();
    //     self.raw_keywords.nonstd.retain(|k, v| {
    //         if let Ok(nk) = k.parse::<NonStdKey>() {
    //             ns.insert(nk, v.clone());
    //             false
    //         } else {
    //             true
    //         }
    //     });
    //     ns
    // }

    // TODO I don't really need a hash table for this. It would be easier and
    // probably faster to just use a paired vector, although I would need to
    // ensure the keys are unique.
    // pub(crate) fn lookup_all_meas_nonstandard(&mut self, n: MeasIdx) -> NonStdKeywords {
    //     let mut ns = HashMap::new();
    //     // ASSUME the pattern does not start with "$" and has a %n which will be
    //     // subbed for the measurement index. The pattern will then be turned
    //     // into a legit rust regular expression, which may fail depending on
    //     // what %n does, so check it each time.
    //     if let Some(p) = &self.conf.nonstandard_measurement_pattern {
    //         match p.from_index(n) {
    //             Ok(pattern) => self.raw_keywords.nonstd.retain(|k, v| {
    //                 if let Some(nk) = pattern.try_match(k.as_ref()) {
    //                     ns.insert(nk, v.clone());
    //                     false
    //                 } else {
    //                     true
    //                 }
    //             }),
    //             Err(err) => self.deferred.push_warning(err.to_string()),
    //         }
    //     }
    //     ns
    // }

    pub fn push_error(&mut self, e: String) {
        self.deferred.push_error(e)
    }

    pub fn push_warning(&mut self, e: String) {
        self.deferred.push_warning(e)
    }

    // auxiliary functions

    fn collect(self) -> PureErrorBuf {
        self.deferred
    }

    fn into_failure(self, reason: String) -> PureFailure {
        Failure {
            reason,
            deferred: self.collect(),
        }
    }

    fn from(kws: &'b mut StdKeywords, conf: &'a StdTextReadConfig) -> Self {
        KwParser {
            raw_keywords: kws,
            deferred: PureErrorBuf::default(),
            conf,
        }
    }

    fn process_opt<V>(
        &mut self,
        res: Result<OptionalKw<V>, String>,
        k: StdKey,
        dep: bool,
    ) -> OptionalKw<V> {
        res.inspect(|_| {
            if dep {
                let msg = format!("deprecated key: {k}");
                self.deferred
                    .push_msg_leveled(msg, self.conf.disallow_deprecated);
            }
        })
        .map_err(|e| self.deferred.push_warning(e))
        .unwrap_or(None.into())
    }
}
