#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 8192 { return; }
    if let Ok(event) = serde_json::from_slice::<rappin::Event>(bytes) {
        let _ = event.parse_data();
        let encoded = serde_json::to_vec(&event).unwrap();
        let decoded: rappin::Event = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(event, decoded);
    }
    if let Ok(event) = serde_json::from_slice::<rappin::hot_reload::CogEvent>(bytes) {
        let _ = (event.message(), event.interaction(), event.argv(), event.author_id(), event.channel_id());
        let encoded = serde_json::to_vec(&event).unwrap();
        let _: rappin::hot_reload::CogEvent = serde_json::from_slice(&encoded).unwrap();
    }
});
