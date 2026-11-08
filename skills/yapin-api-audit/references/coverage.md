# Coverage review

Read `report.changes` even if operation additions are empty. Watch schema enums,
description-only semantics, optional/null/required fields, multipart behavior,
security, events, `x-*` extensions, limits and rollout flags. Shared schemas require
tracing direct/transitive `$ref` consumers; the helper does not resolve the graph.
Readable guide changes can describe browser/portal features with no bot REST API.

For each relevant delta, locate and assess:

1. Public request/response DTOs and lossless serialization.
2. Typed builders, bounds, cross-field rules and validation before I/O.
3. Client and shared Api/ApiExt behavior, response decoding and native/worker parity.
4. Catalogue schemas, availability, resource/server scope and separate actor checks.
5. CLI/MCP exposure, dynamic-command parsing/help and offline discovery.
6. Documentation, runnable examples and meaningful fixture coverage.

Use statuses `covered`, `partial`, `missing`, `not_applicable`, or `unknown`, with
file/symbol evidence and a short reason. Extensible permission strings may already
accept a new name while a schema enum still rejects it. Raw JSON calls can reach
an endpoint without providing typed support, validation or safe capability binding.
Never infer access precedence or hierarchy ordering solely from field presence.

Known audit probes to prevent recurring omissions:

- A message field such as `sticker_id` may require mutually exclusive payload
  fields, server-resource validation and feature availability, even though the
  endpoint already exists. Inspect exhaustive public structs before adding fields.
- A permission such as `use_activities` can appear only in descriptions and override
  enums. Check role/everyone/channel/category paths and shared schema consumers.
- Gateway events, gateway API requests, webhooks and polling have separate contracts.
  Do not normalize payload differences or infer idempotency from local operation IDs.

Snapshot manifests preserve when/where bytes were fetched, not platform deploy
history. Source contents must still be interpreted. An unchanged public spec cannot
prove the server has no undocumented changes. Avoid authenticated probes unless
the user requests them; no traffic load or mutation is part of this audit.
