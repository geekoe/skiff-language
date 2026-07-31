use std::path::PathBuf;

use super::*;

fn test_source(relative_path: &str, module_path: &str, text: &str) -> CompilerSourceFile {
    CompilerSourceFile::parse(
        PathBuf::from(relative_path),
        module_path.to_string(),
        false,
        false,
        text.to_string(),
        relative_path,
    )
    .expect("test source should parse")
}

#[test]
fn official_std_private_modules_do_not_create_root_private_alias() {
    let sources = vec![
        test_source(
            "log.skiff",
            "std.log",
            r#"
                    type LogEntry {
                      helper: root.__private.helper.HelperState
                    }
                "#,
        ),
        test_source(
            "helper.skiff",
            "std.__private.helper",
            r#"
                    type HelperState {
                      value: string
                    }
                "#,
        ),
    ];

    let error = match parse_publication_sources(Path::new("/tmp/std-private-root-alias"), &sources)
    {
        Ok(_) => panic!("std private modules must not create root.__private aliases"),
        Err(error) => error.to_string(),
    };

    assert!(
            error.contains(
                "root reference `root.__private.helper.HelperState` resolves to module `__private/helper.skiff` which does not exist"
            ),
            "unexpected error: {error}"
        );
}

#[test]
fn official_std_public_modules_keep_stripped_root_aliases() {
    let sources = vec![
        test_source(
            "log.skiff",
            "std.log",
            r#"
                    type LogEntry {
                      event: root.telemetry.Event
                    }
                "#,
        ),
        test_source(
            "telemetry.skiff",
            "std.telemetry",
            r#"
                    type Event {
                      value: string
                    }
                "#,
        ),
    ];

    parse_publication_sources(Path::new("/tmp/std-public-root-alias"), &sources)
        .expect("std public modules should keep stripped root aliases");
}
