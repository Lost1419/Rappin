# Yapin API support in Rappin

Choose a report matching the application's **published `rappin-rs` version**.
An installed CLI, a checkout of main and an application's lockfile can each use
different versions.

| Published crate | Checked against | Report | Structured data |
|---|---|---|---|
| 0.1.29 | API 1.3.0, captured 8 November 2026 UTC | [All 76 operations](https://github.com/Lost1419/Rappin/blob/main/docs/api-support-0.1.29.md) | [JSON](https://github.com/Lost1419/Rappin/blob/main/docs/api-support-0.1.29.json) |
| 0.1.28 | API 1.3.0, captured 8 November 2026 UTC | [Feature support](https://github.com/Lost1419/Rappin/blob/main/docs/api-support-0.1.28.md) | [JSON](https://github.com/Lost1419/Rappin/blob/main/docs/api-support-0.1.28.json) |

The 0.1.29 report assesses all 76 documented operations; the earlier 0.1.28 report
reviews nine selected features. Operation coverage describes the stated primary
request/response workflow, with explicit field and transport limitations. It does
not certify undocumented platform behavior or live authorization. Each row distinguishes typed
SDK support, low-level requests, validated bindings, worker support and CLI/MCP,
with feature requirements, rollout limits and links to the selected release source.

Supported means the stated SDK surface exists for the reviewed feature; platform
permissions and availability still apply. Partial identifies a concrete missing
subset. Missing refers to a reviewed absent workflow. Not applicable means the
interface is not relevant, such as running a browser activity through a Rust bot
endpoint. Generic JSON requests alone do not establish typed or validated support.

To check a different release or current upstream changes, use the public
[yapin-api-audit skill](https://github.com/Lost1419/Rappin/blob/main/skills/yapin-api-audit/SKILL.md). Copy the complete
skill directory, including scripts and references, into your agent's skills
directory. Its scripts also work directly with Python 3.11+ and no extra packages.

The skill downloads a selected crate without building it, verifies its registry
checksum and binds reviewed assertions to exact source/API hashes. An unmatched
version or changed API/guide snapshot produces Unknown until reassessed. Saved
reports retain their capture and review dates; they are not live status pages.
