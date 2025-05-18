use fireflow_core::api;
use fireflow_core::config;
use fireflow_core::error::*;
use fireflow_core::validated::datepattern::DatePattern;
use fireflow_core::validated::nonstandard::NonStdMeasPattern;
use fireflow_core::validated::pattern::*;

use clap::{arg, value_parser, ArgMatches, Command};
use polars::prelude::*;
use serde::ser::Serialize;
use std::io;
use std::path::PathBuf;

fn print_json<T: Serialize>(j: &T) {
    println!("{}", serde_json::to_string(j).unwrap());
}

pub fn print_parsed_data(s: &mut api::StandardizedDataset, _delim: &str) {
    let df = s.dataset.as_data();
    let nrows = df.nrows();
    let cols: Vec<_> = df.iter_columns().collect();
    let ncols = cols.len();
    if ncols == 0 {
        return;
    }
    let mut ns = s.dataset.shortnames().into_iter();
    print!("{}", ns.next().unwrap());
    for n in ns {
        print!("\t{n}");
    }
    for r in 0..nrows {
        println!();
        print!("{}", cols[0].pos_to_string(r));
        for c in 1..ncols {
            print!("\t{}", cols[c].pos_to_string(r));
        }
    }
}

// TODO use warnings_are_errors flag
fn handle_failure<X>(res: Result<X, ImpureFailure>) -> io::Result<X> {
    res.map_err(|Failure { reason, deferred }| {
        for e in deferred.errors {
            match e.level {
                PureErrorLevel::Error => eprintln!("ERROR: {}", e.msg),
                PureErrorLevel::Warning => eprintln!("WARNING: {}", e.msg),
            };
        }
        match reason {
            ImpureError::Pure(e) => io::Error::other(format!("CRITICAL FCS ERROR: {}", e)),
            ImpureError::IO(e) => io::Error::other(format!("IO ERROR: {}", e)),
        }
    })
}

fn handle_result<X>(res: ImpureResult<X>) -> io::Result<X> {
    handle_failure(res).and_then(|PureSuccess { data, deferred }| {
        if deferred.has_errors() {
            for e in deferred.errors {
                match e.level {
                    PureErrorLevel::Error => eprintln!("ERROR: {}", e.msg),
                    PureErrorLevel::Warning => eprintln!("WARNING: {}", e.msg),
                };
            }
            Err(io::Error::other(
                "encountered at least one error".to_string(),
            ))
        } else {
            // these are all warnings by definition since we checked above
            for e in deferred.errors {
                eprintln!("WARNING: {}", e.msg);
            }
            Ok(data)
        }
    })
}

fn main() -> io::Result<()> {
    let begintext_arg = arg!(--"begintext-delta" [OFFSET] "adjustment for begin TEXT offset")
        .value_parser(value_parser!(i32));
    let endtext_arg = arg!(--"endtext-delta" [OFFSET] "adjustment for end TEXT offset")
        .value_parser(value_parser!(i32));

    let begindata_arg = arg!(--"begindata-delta" [OFFSET] "adjustment for begin DATA offset")
        .value_parser(value_parser!(i32));
    let enddata_arg = arg!(--"enddata-delta" [OFFSET] "adjustment for end DATA offset")
        .value_parser(value_parser!(i32));

    let delim_arg =
        arg!(-d --delimiter [DELIM] "delimiter to use for the table").default_value("\t");

    let repair_offset_spaces_arg =
        arg!(-o --"repair-offset-spaces" "remove spaces from offset keywords");

    let cmd = Command::new("fireflow")
        .about("read and write FCS files")
        .arg(
            arg!([INPUT_PATH] "input file path")
                .value_parser(value_parser!(PathBuf))
                .required(true)
        )

        .subcommand(
            Command::new("header")
                .about("show header as JSON")
        )

        .subcommand(
            Command::new("raw")
                .about("show raw keywords as JSON")
                .arg(arg!(-H --header "also show header"))
                .arg(&begintext_arg)
                .arg(&endtext_arg)
                .arg(&repair_offset_spaces_arg)
        )

        .subcommand(
            Command::new("std")
                .about("dump standardized keywords as JSON")
                .arg(arg!(-H --header "also show header"))
                .arg(arg!(-r --raw "also show raw"))
                .arg(&begintext_arg)
                .arg(&endtext_arg)
                .arg(arg!(-t --"time-name" [NAME] "name of time measurement"))
                .arg(arg!(-T --"ensure-time" "make sure time measurement exists"))
                .arg(arg!(-l --"ensure-time-linear" "ensure time measurement is linear"))
                .arg(arg!(-g --"ensure-time-nogain" "ensure time measurement does not have gain"))
                .arg(arg!(-d --"disallow-deviant" "disallow deviant keywords"))
                .arg(arg!(-n --"disallow-nonstandard" "disallow nonstandard keywords"))
                .arg(arg!(-D --"disallow-deprecated" "disallow deprecated keywords"))
                .arg(arg!(-p --"date-pattern" [PATTERN] "pattern to use when matching $DATE"))
                .arg(arg!(-P --"ns-meas-pattern" [PATTERN] "pattern used to for nonstandard measurement keywords"))
                .arg(&repair_offset_spaces_arg)
        )

        .subcommand(
            Command::new("measurements")
                .about("show a table of standardized measurement values")
                .arg(&begintext_arg)
                .arg(&endtext_arg)
                .arg(&delim_arg)
                // TODO this shouldn't be necessary since we aren't reading DATA
                .arg(&repair_offset_spaces_arg)
        )

        .subcommand(
            Command::new("spillover")
                .about("dump the spillover matrix if present")
                .arg(&begintext_arg)
                .arg(&endtext_arg)
                .arg(&delim_arg)
                // TODO this shouldn't be necessary since we aren't reading DATA
                .arg(&repair_offset_spaces_arg)
        )

        .subcommand(
            Command::new("data")
                .about("show a table of the DATA segment")
                .arg(&begintext_arg)
                .arg(&endtext_arg)
                .arg(&begindata_arg)
                .arg(&enddata_arg)
                .arg(&repair_offset_spaces_arg)
                .arg(&delim_arg)
        );

    let args = cmd.get_matches();

    let filepath = args.get_one::<PathBuf>("INPUT_PATH").unwrap();

    let get_text_delta = |args: &ArgMatches| {
        let mut begin = 0;
        let mut end = 0;
        if let Some(x) = args.get_one("begintext-delta") {
            begin = *x
        }
        if let Some(x) = args.get_one("endtext-delta") {
            end = *x
        }
        config::OffsetCorrection { begin, end }
    };

    match args.subcommand() {
        Some(("header", _)) => {
            let conf = config::HeaderConfig::default();
            let header = handle_result(api::read_fcs_header(filepath, &conf))?;
            print_json(&header);
        }

        Some(("raw", sargs)) => {
            let mut conf = config::RawTextReadConfig::default();
            get_text_delta(sargs);
            conf.repair_offset_spaces = sargs.get_flag("repair-offset-spaces");
            let raw = handle_result(api::read_fcs_raw_text(filepath, &conf))?;
            // if sargs.get_flag("header") {
            //     print_json(&header);
            // }
            print_json(&raw);
        }

        Some(("spillover", sargs)) => {
            let mut conf = config::StdTextReadConfig::default();
            get_text_delta(sargs);
            conf.raw.repair_offset_spaces = sargs.get_flag("repair-offset-spaces");
            let delim = sargs.get_one::<String>("delimiter").unwrap();

            let res = handle_result(api::read_fcs_std_text(filepath, &conf))?;
            res.standardized.print_spillover_table(delim)
        }

        Some(("measurements", sargs)) => {
            let mut conf = config::StdTextReadConfig::default();
            get_text_delta(sargs);
            conf.raw.repair_offset_spaces = sargs.get_flag("repair-offset-spaces");
            let delim = sargs.get_one::<String>("delimiter").unwrap();

            let res = handle_result(api::read_fcs_std_text(filepath, &conf))?;
            res.standardized.print_meas_table(delim)
        }

        Some(("std", sargs)) => {
            let mut conf = config::StdTextReadConfig::default();
            get_text_delta(sargs);

            // TODO refactor
            if let Some(d) = sargs.get_one::<String>("date-pattern").cloned() {
                conf.raw.date_pattern =
                    Some(handle_failure(d.parse::<DatePattern>().map_err(|e| {
                        Failure::from_many_errors(
                            "bad date pattern".to_string(),
                            vec![e.to_string()],
                        )
                        .into()
                    }))?);
            }

            if let Some(m) = sargs.get_one::<String>("ns-meas-pattern").cloned() {
                conf.nonstandard_measurement_pattern = Some(handle_failure(
                    m.parse::<NonStdMeasPattern>().map_err(|e| {
                        Failure::from_many_errors(
                            "bad ns-meas_pattern".to_string(),
                            vec![e.to_string()],
                        )
                        .into()
                    }),
                )?);
            }

            if let Some(m) = sargs.get_one::<String>("time-name").cloned() {
                conf.time.pattern = Some(handle_failure(m.parse::<TimePattern>().map_err(|e| {
                    Failure::from_many_errors("bad time-name".to_string(), vec![e.to_string()])
                        .into()
                }))?);
            }

            conf.time.ensure = sargs.get_flag("ensure-time");
            conf.time.ensure_linear = sargs.get_flag("ensure-time-linear");
            conf.time.ensure_nogain = sargs.get_flag("ensure-time-nogain");
            conf.disallow_deviant = sargs.get_flag("disallow-deviant");
            conf.disallow_nonstandard = sargs.get_flag("disallow-nonstandard");
            conf.disallow_deprecated = sargs.get_flag("disallow-deprecated");
            conf.raw.repair_offset_spaces = sargs.get_flag("repair-offset-spaces");

            let res = handle_result(api::read_fcs_std_text(filepath, &conf))?;
            if sargs.get_flag("header") {
                print_json(&res.parse);
            }
            // if sargs.get_flag("raw") {
            //     print_json(&res.raw);
            // }
            print_json(&res.standardized);
            // print_json(&res.nonfatal);
        }

        Some(("data", sargs)) => {
            let mut conf = config::DataReadConfig::default();

            get_text_delta(sargs);
            // TODO add DATA delta adjust
            conf.standard.raw.repair_offset_spaces = sargs.get_flag("repair-offset-spaces");
            let delim = sargs.get_one::<String>("delimiter").unwrap();

            let mut res = handle_result(api::read_fcs_file(filepath, &conf))?;
            // print_parsed_data(&mut res, delim);
        }

        _ => (),
    }
    Ok(())
}
