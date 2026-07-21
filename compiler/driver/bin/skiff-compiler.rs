use std::path::PathBuf;

use skiff_compiler::authoring::{build_authoring_object, AuthoringObject};
use skiff_compiler::ecosystem_store::run_ecosystem_store_adapter;

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
    if object == "__ecosystem-store" {
        return run_internal_ecosystem_store(args);
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

fn run_internal_ecosystem_store(
    mut args: impl Iterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if args.next().as_deref() != Some("--artifact-root") {
        return Err("internal ecosystem-store requires --artifact-root <dir>".into());
    }
    let artifact_root = PathBuf::from(
        args.next()
            .ok_or("internal ecosystem-store requires --artifact-root <dir>")?,
    );
    if let Some(argument) = args.next() {
        return Err(format!("unknown internal ecosystem-store option {argument}").into());
    }
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    run_ecosystem_store_adapter(&artifact_root, stdin.lock(), stdout.lock())
}

#[cfg(test)]
mod tests {
    use super::USAGE;

    #[test]
    fn ecosystem_store_internal_action_is_absent_from_public_help() {
        assert!(!USAGE.contains("__ecosystem-store"));
        for object in ["package", "contract", "deployment", "assembly"] {
            assert!(USAGE.contains(object));
        }
    }
}
