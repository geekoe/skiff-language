use std::path::PathBuf;

use skiff_compiler::authoring::{build_authoring_object, AuthoringObject};

const USAGE: &str = "usage: skiff-compiler <package|contract|deployment|assembly> <build|publish> <root> --artifact-root <dir> [--json]";

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut args = std::env::args().skip(1);
    let object = args.next().ok_or(USAGE)?;
    if object == "-h" || object == "--help" {
        println!("{USAGE}");
        return Ok(());
    }
    let object = AuthoringObject::parse(&object)?;
    let action = args.next().ok_or(USAGE)?;
    let publish_pointer = match action.as_str() {
        "build" => false,
        "publish" => true,
        _ => {
            return Err(
                format!("unknown authoring action {action}; expected build or publish").into(),
            )
        }
    };
    let root = PathBuf::from(args.next().ok_or(USAGE)?);
    let mut artifact_root = None;
    let mut json = false;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--artifact-root" => {
                if artifact_root.is_some() {
                    return Err("--artifact-root was provided more than once".into());
                }
                artifact_root = Some(PathBuf::from(
                    args.next().ok_or("--artifact-root requires a path")?,
                ));
            }
            "--json" => json = true,
            _ => return Err(format!("unknown option {argument}\n{USAGE}").into()),
        }
    }
    let artifact_root = artifact_root.ok_or("--artifact-root is required")?;
    let receipt = build_authoring_object(object, &root, &artifact_root, publish_pointer)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&receipt)?);
    } else {
        println!("{}", serde_json::to_string(&receipt)?);
    }
    Ok(())
}
