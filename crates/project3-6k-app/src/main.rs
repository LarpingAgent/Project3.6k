use tracing_subscriber::EnvFilter;

fn main() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if let Err(error) = tracing_subscriber::fmt().with_env_filter(filter).try_init() {
        eprintln!("logging initialization failed: {error}");
        std::process::exit(1);
    }

    let options = match parse_args(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}\nUsage: project3-6k [--run-frames N | --auto-close]");
            std::process::exit(2);
        }
    };

    if let Err(error) = project3_6k_platform::run_with_options(options) {
        tracing::error!(%error, "application exited with an error");
        std::process::exit(1);
    }
}

fn parse_args(
    args: impl IntoIterator<Item = String>,
) -> Result<project3_6k_platform::RunOptions, String> {
    let mut run_frames = None;
    let mut auto_close = false;
    let mut args = args.into_iter();

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--run-frames" => {
                if run_frames.is_some() {
                    return Err("--run-frames may only be specified once".into());
                }
                let value = args
                    .next()
                    .ok_or_else(|| "--run-frames requires a positive integer".to_owned())?;
                let frames = value
                    .parse::<u64>()
                    .map_err(|_| "--run-frames requires a positive integer".to_owned())?;
                if frames == 0 {
                    return Err("--run-frames must be greater than zero".into());
                }
                run_frames = Some(frames);
            }
            "--auto-close" => {
                if auto_close {
                    return Err("--auto-close may only be specified once".into());
                }
                auto_close = true;
            }
            unknown => return Err(format!("unrecognized argument: {unknown}")),
        }
    }

    if auto_close && run_frames.is_some() {
        return Err("--auto-close and --run-frames cannot be combined".into());
    }

    Ok(project3_6k_platform::RunOptions {
        run_frames,
        auto_close,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_args;
    use project3_6k_platform::RunOptions;

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn parses_positive_frame_count() {
        assert_eq!(
            parse_args(arguments(&["--run-frames", "60"])),
            Ok(RunOptions {
                run_frames: Some(60),
                auto_close: false,
            })
        );
    }

    #[test]
    fn parses_auto_close() {
        assert_eq!(
            parse_args(arguments(&["--auto-close"])),
            Ok(RunOptions {
                run_frames: None,
                auto_close: true,
            })
        );
    }

    #[test]
    fn rejects_zero_missing_duplicate_and_unknown_arguments() {
        assert!(parse_args(arguments(&["--run-frames", "0"])).is_err());
        assert!(parse_args(arguments(&["--run-frames"])).is_err());
        assert!(parse_args(arguments(&["--run-frames", "1", "--run-frames", "2"])).is_err());
        assert!(parse_args(arguments(&["--unknown"])).is_err());
    }

    #[test]
    fn rejects_conflicting_auto_close_and_frame_count() {
        assert!(parse_args(arguments(&["--auto-close", "--run-frames", "1"])).is_err());
    }
}
