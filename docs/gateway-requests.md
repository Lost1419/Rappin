# Opt-in gateway API requests

Enable the `gateway` feature, configure a `Gateway`/gateway event runner, then call
`Client::configure_gateway_requests(Some(GatewayRequestConfig::default()))` before
connecting. `None` restores HTTPS selection for subsequent JSON calls. HTTPS
remains the default. Client clones share configuration, request routing, rate-limit
buckets and telemetry. Use one gateway connection owner per client.

The connection advertises support with `ready.requests: true`. Each connection
negotiates independently. Supported JSON calls through `Client`, `Api`/`ApiExt`,
native contexts and the parent-owned worker proxy use request/response frames.
The JSON methods GET/POST/PUT/PATCH/DELETE are eligible. Uploads, binary/public
unauthenticated requests, other HTTP methods, oversized JSON request frames and
`/events` use HTTPS. An absent
connection, unavailable capability or disabled configuration selects HTTPS before
submission. A configuration change cannot retroactively move an in-flight call.
Use `Client::gateway_request_status()` for enabled/capability/connection state,
selected JSON transport and bounded counters/timing; no payloads or credentials
are included. Existing request observations and cumulative metrics still apply.

## Bounds and retry ownership

The pump allows eight admitted requests total and 100 submitted frames per rolling
ten seconds per connection. Correlation IDs include a random connection ID and a
monotonic counter. Out-of-order replies are matched independently; unknown/duplicate
IDs only increment a counter. The event queue holds 64 frames; overflow pauses event
admission while preserving API responses and heartbeats. Intake recovers coverage
from polling or reconnects only after admitted requests have settled. Request frames and received
messages are capped at 1 MiB when the request transport is enabled.

`GatewayRequestConfig::timeout` defaults to 15 seconds (valid range: greater than
zero through 120 seconds). Admission saturation returns a structured
`gateway_request` error with `submitted: false`, without switching to HTTPS.
A request cancelled/timed out before the submission state transition cannot later
be sent by the pump. Once that transition begins, even a socket write error cannot
prove the peer received no bytes: timeout, cancellation and disconnect are treated
as uncertain. Error JSON retains `submitted`, `outcome`, `retry_safety` and any
available structured connection cause. No automatic HTTPS fallback or reconnect
replay occurs for that attempt. Cancellation cannot return an error to the cancelled
future; its uncertainty is counted and interaction response slots remain protected.

A local socket-window limit returns a 429 response with retry advice without sending
a frame. Actual API responses use the existing HTTP-equivalent decoding and shared
rate-limit/retry budget. Only existing documented server-advised 429/503 recovery
can retry a known-unapplied request; the pump itself never retries. `request_once`
suppresses those extra attempts while retaining shared rate-limit waits. The request
timeout is separate from `max_retry_wait`. Partial workflows retain confirmed
receipts and stop at the original failed/uncertain step.

Heartbeats, event frames and API responses are handled by an independent pump even
when application handlers await requests. Socket writes are bounded to one second.
Do not enable dependency wire tracing: the gateway refuses credential submission
when those trace targets are active.

## Workers and example

No worker protocol change is needed. Existing workers route API requests through
the parent's selected transport. `WorkerCog::single_attempt_api()` and
`WorkerContext::api_once` require the existing protocol v6 support; extended UI
workers retain v7. Typed validation, authorization checks, structured remote errors
and original message identity remain in their existing layers.

Run [the example](../examples/gateway_api_requests.rs):
`cargo run --features gateway --example gateway_api_requests`, with
`YAPIN_BOT_TOKEN` configured. If the platform does not advertise the capability,
its replies continue over HTTPS. Hosting and application recovery policies remain
consumer-owned. This transport follows the request/response frames in the
[public OpenAPI](https://www.yapin.gg/openapi.json).

See [draining and operator retirement](draining-and-retirement.md) for bounded
shutdown, structured disconnect causes, polling-only admission and explicit
assessment of retained failed/uncertain work.
