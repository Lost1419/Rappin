#![cfg(feature = "derive")]
use std::{path::PathBuf, process::Command};

struct Project(PathBuf);
impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn derives_support_renamed_dependencies_and_report_definition_errors_at_compile_time() {
    let project =
        Project(std::env::temp_dir().join(format!("rappin-derive-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir_all(project.0.join("src")).unwrap();
    std::fs::write(project.0.join("Cargo.toml"), format!(
        "[package]\nname=\"derive-fixture\"\nversion=\"0.0.0\"\nedition=\"2021\"\n[workspace]\n[dependencies]\nbot_sdk={{package=\"rappin-rs\",path={:?},features=[\"derive\"]}}\n",
        env!("CARGO_MANIFEST_DIR")
    )).unwrap();
    let cases = [
        (
            "struct Args { #[arg(default = 9223372036854775807, max = 9223372036854775806)] value: i64 }",
            Some("default is outside"),
        ),
        (
            "#[arg(crate = \"bot_sdk\")] struct Args { value: String }",
            None,
        ),
        (
            "struct Args { #[arg(default = 6, min = 2, max = 100)] sides: u32 }",
            None,
        ),
        (
            "struct Args { #[arg(rest)] words: String, tail: String }",
            Some("rest requires the final"),
        ),
        (
            "struct Args { optional: Option<String>, required: String }",
            Some("required arguments must precede"),
        ),
        (
            "struct Args { #[arg(default = 1, min = 2)] sides: u32 }",
            Some("default is outside"),
        ),
        (
            "struct Args { #[arg(min = 4, max = 2)] sides: u32 }",
            Some("minimum exceeds maximum"),
        ),
        (
            "struct Args { #[arg(unknown)] value: String }",
            Some("expected default, min, max or rest"),
        ),
        (
            "struct Args { #[arg(default = make_value())] value: u32 }",
            Some("require literal values"),
        ),
    ];
    for (definition, diagnostic) in cases {
        std::fs::write(
            project.0.join("src/main.rs"),
            format!("#[derive(bot_sdk::FromArgs)]\n{definition}\nfn main() {{}}\n"),
        )
        .unwrap();
        let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
            .args(["check", "--offline", "--quiet"])
            .current_dir(&project.0)
            .env(
                "CARGO_TARGET_DIR",
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/derive-check"),
            )
            .env("CARGO_PROFILE_DEV_DEBUG", "0")
            .env("CARGO_INCREMENTAL", "0")
            .output()
            .unwrap();
        let errors = String::from_utf8_lossy(&output.stderr);
        if let Some(diagnostic) = diagnostic {
            assert!(!output.status.success(), "{definition}");
            assert!(errors.contains(diagnostic), "{definition}\n{errors}");
        } else {
            assert!(output.status.success(), "{errors}");
        }
    }
}

mod properties {
    use proptest::prelude::*;
    use rappin::{Arguments, CommandError, CommandSpec, FromArgs};

    #[derive(FromArgs)]
    struct Label {
        /// Display label.
        #[arg(min = 2, max = 4, rest)]
        text: String,
    }

    #[derive(FromArgs)]
    struct Limits {
        #[arg(min = -9223372036854775807, max = 9223372036854775806)]
        value: i64,
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(96))]

        #[test]
        fn derived_string_bounds_count_scalars(
            chars in proptest::collection::vec(prop::sample::select(vec!['a', 'é', '\u{301}', '🦀']), 0..8)
        ) {
            let text: String = chars.into_iter().collect();
            let input = format!(r#""{text}""#);
            let mut arguments = Arguments::new(&input).unwrap();
            match Label::parse(&mut arguments) {
                Ok(label) => {
                    prop_assert!((2..=4).contains(&text.chars().count()));
                    prop_assert_eq!(label.text, text);
                    arguments.finish().unwrap();
                }
                Err(error) => {
                    prop_assert!(!(2..=4).contains(&text.chars().count()));
                    prop_assert!(matches!(error, CommandError::Invalid { .. }), "unexpected error: {error}");
                }
            }
        }

        #[test]
        fn derived_integer_bounds_are_exact_near_i64_limits(
            value in prop_oneof![i64::MIN..=i64::MIN + 8, i64::MAX - 8..=i64::MAX]
        ) {
            let mut arguments = Arguments::new(&value.to_string()).unwrap();
            match Limits::parse(&mut arguments) {
                Ok(args) => {
                    prop_assert!(value != i64::MIN && value != i64::MAX);
                    prop_assert_eq!(args.value, value);
                }
                Err(_) => prop_assert!(value == i64::MIN || value == i64::MAX),
            }
        }
    }

    #[test]
    fn rest_uses_quote_aware_tokens_and_emits_help() {
        let mut arguments = Arguments::new(r#"a "é🦀""#).unwrap();
        assert_eq!(Label::parse(&mut arguments).unwrap().text, "a é🦀");
        assert!(matches!(
            Arguments::new(r#"a "é🦀"#),
            Err(CommandError::Quoting)
        ));
        let spec = CommandSpec::<Label>::new("label", "Label a record");
        assert!(spec
            .info()
            .help()
            .contains("Display label. (minimum 2) (maximum 4)"));
    }
}
