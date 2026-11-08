//! Offline costs only; no live API or remote mutation is measured.
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rappin::{
    capabilities::capability, commands::tokenize, Bot, Cache, Channel, CreateMessage, Event,
};
use serde_json::json;
use uuid::Uuid;
fn benchmark(c: &mut Criterion) {
    c.bench_function("arguments/quoted", |b| {
        b.iter(|| tokenize(black_box(r#"20 "two words" escaped\ value"#)).unwrap())
    });
    let event_json = json!({"id":1,"type":"message","data":{"id":Uuid::nil(),"channel_id":Uuid::nil(),"author_id":Uuid::nil(),"command":"ping","content":"!ping","author_bot":false}}).to_string();
    c.bench_function("event/decode", |b| {
        b.iter(|| serde_json::from_str::<Event>(black_box(&event_json)).unwrap())
    });
    let event: Event = serde_json::from_str(&event_json).unwrap();
    c.bench_function("event/typed_payload", |b| {
        b.iter(|| black_box(&event).parse_data().unwrap())
    });
    let message = CreateMessage::new("benchmark message");
    c.bench_function("message/serialize", |b| {
        b.iter(|| serde_json::to_vec(black_box(&message)).unwrap())
    });
    let operation = capability("send_message").unwrap();
    let arguments = json!({"channel":Uuid::nil(),"content":"benchmark message"});
    c.bench_function("capability/validate", |b| {
        b.iter(|| operation.validate(black_box(&arguments)).unwrap())
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let cache = Cache::new();
    let channel: Channel = serde_json::from_value(
        json!({"id":Uuid::nil(),"server_id":Uuid::nil(),"name":"bench","kind":"text"}),
    )
    .unwrap();
    runtime.block_on(cache.update_channel(channel.clone()));
    c.bench_function("cache/channel_hit", |b| {
        b.iter(|| {
            runtime
                .block_on(cache.get_channel(black_box(&channel.id)))
                .unwrap()
        })
    });
    c.bench_function("cache/channel_replace", |b| {
        b.iter(|| runtime.block_on(cache.update_channel(black_box(channel.clone()))))
    });
    let bot = Bot::builder("offline-unused-token")
        .sync_commands(false)
        .log(false)
        .command("ping", "No effects", |_| async { Ok(()) })
        .build();
    runtime.block_on(bot.cache().set_me(
        serde_json::from_value(json!({"id":Uuid::from_u128(1),"username":"benchmark"})).unwrap(),
    ));
    c.bench_function("dispatch/native_no_effects", |b| {
        b.iter(|| {
            runtime
                .block_on(bot.process_event(black_box(event.clone())))
                .unwrap()
        })
    });
}
criterion_group!(benches, benchmark);
criterion_main!(benches);
