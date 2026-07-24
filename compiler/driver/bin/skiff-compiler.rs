use std::path::PathBuf;

use skiff_compiler::authoring::{build_authoring_object, AuthoringObject};
use skiff_compiler::CompilerPlatformSources;

const USAGE: &str = "usage: skiff-compiler <package|contract|deployment|assembly> <build|publish> <root> --artifact-root <dir> [--json]";

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    run_with_args(std::env::args().skip(1))
}

fn run_with_args(
    mut args: impl Iterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
    let mut platform_source_root = None;
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
            "--platform-source-root" => {
                if platform_source_root.is_some() {
                    return Err("--platform-source-root was provided more than once".into());
                }
                platform_source_root = Some(PathBuf::from(
                    args.next()
                        .ok_or("--platform-source-root requires a path")?,
                ));
            }
            "--json" => json = true,
            _ => return Err(format!("unknown option {argument}\n{USAGE}").into()),
        }
    }
    let artifact_root = artifact_root.ok_or("--artifact-root is required")?;
    let platform_source_root = platform_source_root.ok_or("--platform-source-root is required")?;
    let platform_sources = CompilerPlatformSources::new(&platform_source_root)?;
    let receipt = build_authoring_object(
        &platform_sources,
        object,
        &root,
        &artifact_root,
        publish_pointer,
    )?;
    if json {
        println!("{}", serde_json::to_string_pretty(&receipt)?);
    } else {
        println!("{}", serde_json::to_string(&receipt)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{run_with_args, USAGE};

    #[test]
    fn internal_actions_are_absent_from_public_help() {
        assert!(!USAGE.contains("platform-source"));
        for object in ["package", "contract", "deployment", "assembly"] {
            assert!(USAGE.contains(object));
        }
    }

    #[test]
    fn authoring_actions_require_exactly_one_platform_source_root() {
        for object in ["package", "contract", "deployment", "assembly"] {
            let missing = run_error(&[
                object,
                "build",
                "/missing-authoring-root",
                "--artifact-root",
                "/tmp/skiff-artifacts",
            ]);
            assert_eq!(missing, "--platform-source-root is required");

            let duplicate = run_error(&[
                object,
                "build",
                "/missing-authoring-root",
                "--artifact-root",
                "/tmp/skiff-artifacts",
                "--platform-source-root",
                "/missing-platform-root-a",
                "--platform-source-root",
                "/missing-platform-root-b",
            ]);
            assert_eq!(
                duplicate,
                "--platform-source-root was provided more than once"
            );
        }
    }

    #[test]
    fn authoring_actions_reject_relative_or_unreadable_platform_source_roots_first() {
        let relative = run_error(&[
            "package",
            "build",
            "/missing-authoring-root",
            "--artifact-root",
            "/tmp/skiff-artifacts",
            "--platform-source-root",
            "relative/platform-root",
        ]);
        assert!(relative.contains("must be absolute"), "{relative}");

        let unreadable = run_error(&[
            "contract",
            "build",
            "/missing-authoring-root",
            "--artifact-root",
            "/tmp/skiff-artifacts",
            "--platform-source-root",
            "/missing-skiff-platform-root",
        ]);
        assert!(
            unreadable.contains("compiler platform source"),
            "{unreadable}"
        );
        assert!(!unreadable.contains("contract.yml"), "{unreadable}");
    }

    fn run_error(args: &[&str]) -> String {
        run_with_args(args.iter().map(|argument| (*argument).to_owned()))
            .unwrap_err()
            .to_string()
    }
}
