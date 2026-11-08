#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 8192 { return; }
    if let Ok(text) = std::str::from_utf8(bytes) {
        if let Ok(tokens) = rappin::commands::tokenize(text) {
            for token in tokens {
                assert!(text.is_char_boundary(token.start));
                assert!(text.is_char_boundary(token.end));
                assert!(token.start < token.end && token.end <= text.len());
            }
        }
        if let Ok(value) = serde_json::from_str(text) {
            for name in ["send_message", "edit_channel", "create_category"] {
                let _ = rappin::capabilities::capability(name).unwrap().validate(&value);
            }
            if let Ok(message) = serde_json::from_str::<rappin::CreateMessage>(text) {
                let _ = message.validate();
            }
        }
    }
});
