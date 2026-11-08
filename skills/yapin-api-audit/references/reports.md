# Version-specific support reports

The skill works standalone. Choose an exact crate version from Cargo.lock or the
consumer's explicit selection. CLI version and an application's SDK dependency
can differ. A current Git checkout does not establish published-crate behavior.

Run these commands from the skill directory, using new output paths:

```sh
python3 scripts/support.py release --version 0.1.28 --out /tmp/rappin-0.1.28-source
python3 scripts/audit.py snapshot --out /tmp/yapin-current-evidence
python3 scripts/audit.py compare /tmp/rappin-0.1.28-source/source/docs/openapi.json /tmp/yapin-current-evidence --output /tmp/rappin-contract-diff.json
python3 scripts/support.py report /tmp/rappin-0.1.28-source /tmp/yapin-current-evidence --reviews references/coverage-0.1.28.json --json /tmp/rappin-support.json --markdown /tmp/rappin-support.md
```

`release` fetches only the fixed public crates.io/static.crates.io package URLs.
It requires the registry's SHA-256 checksum, checks the Cargo package identity,
rejects traversal/links/duplicate archive members, bounds input sizes, and records
every exported source file hash. It does not run Cargo, build scripts or macros.
Snapshot integrity checks are local evidence checks, not signatures or a sandbox
for code the user later chooses to build.

`report` binds a reviewed data file to the exact crate checksum and both the
OpenAPI hash and normalized developer-guide text hash. Equal version numbers are
insufficient. A changed selection produces Unknown statuses and clears old rollout
and feature requirements. Source paths/hashes and optional symbols are checked
before matching support claims are printed. The full operation inventory remains
Unknown: a feature review does not certify every behavior of an associated endpoint.

Statuses describe the selected surface:

- `supported`: the reviewed SDK surface provides the stated feature, subject to
  the listed validation, platform checks and rollout restrictions.
- `partial`: a usable subset exists, with a specific missing range or workflow.
- `missing`: source review established the named typed/bound workflow is absent.
- `unknown`: the surface has not been assessed against this evidence selection.
- `not_applicable`: the surface is not the right interface for this feature, such
  as a browser activity runtime or an intake runner in a command catalogue.

Low-level support means a request transport can express a call; it does not
provide typed validation, requester authorization or automatic effect recovery.
Worker support may require parent configuration/protocols. CLI and MCP share a
column but can differ; the reason must identify those differences explicitly.

## Maintaining review data

The bundled `coverage-0.1.28.json` is a worked source review, with these fields:

- `format: 1`, `sdk.package`, exact `sdk.version`, `sdk.crate_sha256`.
- `api.version`, `api.openapi_sha256`, `api.developer_text_sha256` from the captured
  evidence. `reviewed_at` is the actual UTC assessment time.
- `features[]`: stable `id`, `title`, official `api_sources`, `rollout` restrictions,
  associated `{method,path}` operations and per-surface `required_features`.
- `surfaces`: exactly `typed`, `low_level`, `validated_bindings`, `worker`, `cli_mcp`.
  Each has a status, concrete reason and source `evidence` entries with relative
  `path`, SHA-256, and optional `symbol`. Supported/partial/missing claims require
  evidence. Missing claims should point to the relevant complete type/dispatcher/
  catalogue and describe the search performed, rather than cite a random file.

Keep factual claims tied to the selected published source. Inspect transitive
schema references and native/worker/CLI paths before recording a verdict. When
platform docs conflict, preserve the conflict and use Unknown for facts that
cannot be established. Record tests only if actually run; do not imply that source
review is an authenticated integration test.

To update public docs, retain the JSON review, generate JSON and Markdown into
temporary output paths, review both, then copy the approved artifacts into the
repository's versioned report paths. Include check time, source links/hashes and
the limited review scope. Never silently replace an earlier release's assessment
with findings from main. Old reports remain dated evidence.
