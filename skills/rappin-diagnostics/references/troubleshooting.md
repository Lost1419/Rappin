# Troubleshooting paths

## Slow replies

Separate REST header timing, full-call/body decode time, gateway heartbeat RTT,
event delivery age and handler execution. Doctor's last request describes only
its own request, not another running bot's telemetry. A running Rust bot exposes
`transport_status` and Client request observations; consult that application's
logs or diagnostics to measure its actual path. Include Retry-After waits and
attempt counts before attributing time to the network or service. Use several
paced observations, not a single sample or an unbounded probe.

## Missing events or reconnects

Check configured versus active transport and fallback reason in the bot's
telemetry. The gateway is feature-gated and may deny a bot's beta access; polling
fallback is expected in that case. Distinguish a received cursor from a committed
cursor. An agent consumer replays its persisted pending batch until acknowledgement;
acknowledge only delivered IDs after processing them. A failed Rust handler can
prevent its batch checkpoint from advancing. Check backlog and handler saturation
before blaming the gateway. Keep a separate state file for each bot/consumer.

## Authentication and permissions

401 requires corrected/rotated credentials. 403 needs the relevant bot/server
permissions, not another retry. 404 can mean an invisible resource or a wrong ID;
resolve the target inside its server and inspect current membership/roles. Do not
change roles or moderation state without authorization. Custom caches may retain
old permissions; repeat the check with explicit fresh API data when warranted.

## Writes and rate limits

Inspect `status`, `retry_after`, `retry_safety`, and `outcome`. The client has
already applied bounded automatic retries for supported server-advised waits.
502/504 or transport failures on writes can have unknown outcomes. Check history
before a deliberate retry; identical content alone does not establish which
request created a message. A split-send failure returns confirmed completed
steps and the failed step index. Replaying the whole split duplicates successes.

## API origin

Doctor reports configured and active hosts. Default REST failover is restricted
to the documented bot and website API origins. Custom bases never fail over.
Preserve custom transport/endpoint choices while diagnosing them; do not change
a working configuration or re-enable redirects as a speculative fix.
