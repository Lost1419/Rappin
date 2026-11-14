mod support;
use proptest::prelude::*;
use rappin::{capabilities::capability, commands::tokenize, Event};
use serde_json::json;
proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn quoted_arguments_round_trip(values in prop::collection::vec(".{0,40}", 0..20)) {
        let input = values.iter().map(|v| format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))).collect::<Vec<_>>().join(" ");
        let tokens = tokenize(&input).unwrap();
        prop_assert_eq!(tokens.iter().map(|t| t.value.clone()).collect::<Vec<_>>(), values);
        for token in tokens {
            prop_assert!(input.is_char_boundary(token.start));
            prop_assert!(input.is_char_boundary(token.end));
            prop_assert!(token.start < token.end && token.end <= input.len());
        }
    }
    #[test]
    fn arbitrary_text_and_payloads_are_bounded_inputs(text in ".{0,4096}", number in any::<i64>()) {
        let _ = tokenize(&text);
        if let Ok(event) = serde_json::from_str::<Event>(&text) { let _ = event.parse_data(); }
        let operation = capability("edit_channel").unwrap();
        let result = operation.validate(&json!({"id":uuid::Uuid::nil(),"body":{"slowmode":number}}));
        if !(0..=21600).contains(&number) { prop_assert!(result.is_err()); }
        let invalid = operation.validate(&json!({"id":format!("not-uuid:{text}"),"body":{"slowmode":number}}));
        prop_assert!(invalid.is_err());
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]
    #[test]
    fn polling_commits_only_the_ordered_completed_prefix(failures in prop::collection::vec(any::<bool>(), 1..10), reverse in any::<bool>()) {
        use rappin::{Bot, EventCursor, Error};
        use std::sync::{Arc, Mutex};
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let root = std::env::temp_dir().join(format!("rappin-property-{}",uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            let path = root.join("cursor.json");
            EventCursor::new(&path).save(10).await.unwrap();
            let mut events: Vec<_> = (0..failures.len()).map(|i| support::event(11+i as u64, "step")).collect();
            // Identical duplicate deliveries and reverse batches must not reorder execution.
            events.extend(events.clone());
            if reverse { events.reverse(); }
            let (base, mut requests, http) = support::routed_mock(vec![
                ("GET /me".into(),vec![support::response(200,support::profile())]),
                ("GET /events".into(),vec![support::response(200,json!({"events":events,"last":10+failures.len() as u64}))]),
            ]).await;
            let called = Arc::new(Mutex::new(Vec::new()));
            let seen = called.clone(); let decisions = failures.clone();
            let bot = Bot::builder("offline-fixture").base_url(base).sync_commands(false).log(false)
                .configure(|settings| settings.persistence.cursor = Some(rappin::EventCursor::new(&path))).configure(|settings| settings.execution.concurrency = 1)
                .command("step","No remote effects", move |ctx| {
                    let seen=seen.clone(); let decisions=decisions.clone();
                    async move {
                        let mut seen = seen.lock().unwrap();
                        let index = seen.len(); seen.push(ctx.message().expect("message identity").event.id);
                        if decisions[index] { return Err(Error::Config("fixture failure".into())); }
                        if index + 1 == decisions.len() { ctx.bot().unwrap().stop(); }
                        Ok(())
                    }
                }).build().expect("valid bot configuration");
            let result = tokio::time::timeout(std::time::Duration::from_secs(3),bot.run()).await.unwrap();
            let first_failure = failures.iter().position(|v| *v);
            assert_eq!(result.is_err(), first_failure.is_some());
            let completed = first_failure.unwrap_or(failures.len());
            assert_eq!(EventCursor::new(&path).load().await.unwrap(),Some(10+completed as u64));
            assert_eq!(called.lock().unwrap().len(),completed+usize::from(first_failure.is_some()));
            while let Ok(request) = requests.try_recv() { assert!(request.starts_with("GET ")); }
            http.abort();
            std::fs::remove_dir_all(root).unwrap();
        });
    }
}
