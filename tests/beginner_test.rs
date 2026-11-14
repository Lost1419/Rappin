#![cfg(feature = "derive")]
use rappin::{Arguments, Bot, CommandError, CommandSpec, FromArgs};
use std::sync::Arc;

#[derive(Debug, PartialEq, FromArgs)]
struct Roll {
    /// Number of sides.
    #[arg(default = 6, min = 2, max = 100)]
    sides: u32,
    /// Optional label.
    #[arg(rest, max = 40)]
    label: Option<String>,
}

fn parse<T: FromArgs>(input: &str) -> Result<T, CommandError> {
    let mut args = Arguments::new(input)?;
    let value = T::parse(&mut args)?;
    args.finish()?;
    Ok(value)
}

#[test]
fn derived_arguments_share_bounds_optional_rest_defaults_and_help() {
    assert_eq!(
        parse::<Roll>("").unwrap(),
        Roll {
            sides: 6,
            label: None
        }
    );
    assert_eq!(
        parse::<Roll>("20 \"two words\" more").unwrap(),
        Roll {
            sides: 20,
            label: Some("two words more".into())
        }
    );
    for input in ["1", "101", "-2", "4294967296", "no", "\"unterminated"] {
        assert!(parse::<Roll>(input).is_err(), "{input}");
    }
    let schema = rappin::CommandSchema {
        name: "roll".into(),
        description: "Roll a die".into(),
        aliases: vec![],
        arguments: Roll::schema().unwrap(),
    };
    for input in ["", "20 \"two words\" more"] {
        assert!(schema.parse(input).is_ok());
    }
    for input in ["1", "101", "-2", "4294967296", "no", "\"unterminated"] {
        assert!(schema.parse(input).is_err());
    }
    let spec = CommandSpec::<Roll>::new("roll", "Roll a die");
    assert!(spec.info().help().contains("Number of sides. (default 6)"));
    assert_eq!(spec.info().arguments, schema.info().arguments);
}

#[derive(FromArgs)]
struct Unicode {
    #[arg(min = 2, max = 3)]
    value: String,
}

#[test]
fn derived_string_bounds_count_characters() {
    assert_eq!(parse::<Unicode>("🦀é").unwrap().value, "🦀é");
    assert!(parse::<Unicode>("🦀").is_err());
    assert!(parse::<Unicode>("abcd").is_err());
}

#[test]
fn typed_state_is_shared_missing_and_duplicate_values_fail_explicitly() {
    let value = Arc::new(String::from("settings"));
    let bot = Bot::builder("unused-token")
        .state(value.clone())
        .build()
        .unwrap();
    assert!(Arc::ptr_eq(
        &value,
        &bot.handle().state::<String>().unwrap()
    ));
    assert!(bot.handle().state::<u32>().is_err());
    assert!(Bot::builder("unused-token")
        .state(Arc::new(1u32))
        .state(Arc::new(2u32))
        .build()
        .is_err());
}

#[test]
fn definition_failures_are_returned_before_network_or_startup() {
    assert!(Bot::builder("").build().is_err());
    assert!(Bot::builder("unused-token")
        .command_spec(CommandSpec::<()>::new("status", "Status"), |_, ()| async {
            Ok(())
        })
        .command_spec(
            CommandSpec::<()>::new("status", "Duplicate"),
            |_, ()| async { Ok(()) }
        )
        .build()
        .is_err());
}

#[derive(FromArgs)]
struct Names {
    args: String,
    value: String,
    __rappin_arguments: String,
}

#[derive(FromArgs)]
struct SinglePrecision {
    #[arg(min = 0.0, max = 2.0)]
    value: f32,
}

#[test]
fn field_names_are_hygienic_and_rounding_sensitive_parsers_are_deferred() {
    let value = parse::<Names>("one two three").unwrap();
    assert_eq!(value.args, "one");
    assert_eq!(value.value, "two");
    assert_eq!(value.__rappin_arguments, "three");
    assert!(Names::schema().is_some());
    assert!(SinglePrecision::schema().is_none());
    assert_eq!(parse::<SinglePrecision>("1.5").unwrap().value, 1.5);
    assert!(parse::<SinglePrecision>("NaN").is_err());
    assert!(parse::<SinglePrecision>("3").is_err());
}

#[derive(Debug)]
struct Custom(u32);
impl std::str::FromStr for Custom {
    type Err = std::num::ParseIntError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse().map(Self)
    }
}
#[derive(FromArgs)]
struct CustomArgs {
    value: Custom,
}
#[test]
fn custom_parsers_run_at_execution_and_remain_deferred_in_preparation() {
    assert!(CustomArgs::schema().is_none());
    assert_eq!(parse::<CustomArgs>("42").unwrap().value.0, 42);
    assert!(parse::<CustomArgs>("invalid").is_err());
}

mod custom_identity {
    #[derive(Debug)]
    pub struct UserId(pub u32);
    impl std::str::FromStr for UserId {
        type Err = std::num::ParseIntError;
        fn from_str(value: &str) -> Result<Self, Self::Err> {
            value.parse().map(Self)
        }
    }
}
#[derive(FromArgs)]
struct CustomIdentity {
    user: custom_identity::UserId,
}
#[test]
fn custom_types_sharing_sdk_names_keep_their_parser_and_defer_preparation() {
    assert!(CustomIdentity::schema().is_none());
    assert_eq!(parse::<CustomIdentity>("17").unwrap().user.0, 17);
}
