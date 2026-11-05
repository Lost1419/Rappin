# Cross-transport event contract and redacted reproduction

The [developer guide](https://www.yapin.gg/developers#gateway) says gateway `d` is
exactly one `/events` item and `s` is its event ID. The
[public OpenAPI](https://www.yapin.gg/openapi.json) 1.3.0 event schemas require outer
`created_at` as a timestamp. Message data also has its own `created_at`; that does
not document permission to omit, relocate or replace the envelope timestamp.

The following is a **synthetic minimal reproduction** of an SDK conflict. It is
not a claim that these were the two live deliveries in an earlier incident.
Legacy errors saved only a fingerprint and cannot reveal the missing payload.
Use the new report to establish the actual differing paths first.

First verified delivery, later persisted as Completed:

```json
{
  "id": 9001,
  "type": "message",
  "created_at": null,
  "data": {
    "id": "11111111-1111-4111-8111-111111111111",
    "channel_id": "22222222-2222-4222-8222-222222222222",
    "server_id": "33333333-3333-4333-8333-333333333333",
    "author_id": "44444444-4444-4444-8444-444444444444",
    "content": "<redacted>",
    "created_at": "2026-11-05T00:00:00Z"
  }
}
```

A competing delivery with the same bot/event ID and data but
`created_at: "2026-11-05T00:00:00Z"` at the outer level produces a conflict at
`/created_at`, Null versus String. The first object violates the currently
published outer timestamp schema; the legacy Rust Event decoder accepts it as
missing metadata. This demonstrates detection, not the cause of a live outage.
The SDK does not infer that envelope and message creation times are identical.

## Minimal evidence for a platform report

Capture the following without credentials or payload values:

1. SDK/API version, API build, relevant intake configuration and the authenticated
   bot identity, consistently pseudonymized if needed.
2. Last durable polling checkpoint and retained receipt stage/durability.
3. `IngressConflict`: sources (including recovery provenance), event ID,
   fingerprints, differing paths/types, truncation and disposition.
4. A GET-only polling request using the same bot and checkpoint: record its
   `after`, `wait`, configured `max_age`, returned event IDs and `last`. Do not edit
   a cursor or use a different bot/consumer as evidence for the same stream.
5. Whether a matching original payload was subsequently observed through polling.
   Record a bounded recovery timeout explicitly. An empty response with an older
   watermark does not establish that a higher completed event can be skipped.

A fingerprint mismatch alone cannot identify whether the producer changed data,
a transport transformed it, or consumer persistence discarded fields. Compare
new source reports and verify the consumer store round-trips Event and receipt
fields before attributing the problem upstream. The SDK tests synthetic timestamp
mismatches, actual identity/content conflicts, property-order equivalence,
concurrent delivery, restored completions and failed checkpoint persistence.

## Required platform guarantees

For each authenticated bot/event ID, specify:

- Whether the event type, identity fields, payload and envelope are immutable and
  identical across polling, gateway, webhook delivery and redelivery.
- Any intentional representation differences, with exact affected paths and
  canonical meaning. Clarify whether either timestamp can be absent/null, what
  each timestamp represents, and which value controls interaction deadlines.
- Polling visibility and retention/filter behavior, including `max_age`, expired
  commands/interactions, concurrent consumers, and the relationship between
  gateway sequence IDs and polling `last` watermarks.
- What an empty poll means when a different transport has delivered a higher ID,
  and how the caller can obtain authoritative coverage without skipping unseen
  events or replaying uncertain mutations.

If transports intentionally differ, the equivalence contract must preserve
requester/server/message identity, command arguments, meaningful content and
interaction expiry. Only documented equivalence should become SDK normalization.
Until then, contradictory polling payloads remain explicit failures. The optional
completed-only policy can retain original effects and wait for polling proof;
it cannot repair or authorize an unexplained platform contradiction.
