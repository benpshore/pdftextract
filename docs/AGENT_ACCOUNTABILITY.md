# Agent accountability

Owner: Busybody Bob (Mara Keel alias). Maintenance epic: [#225](https://github.com/benpshore/pdftextract/issues/225).
This is a practical, voluntary metadata policy and validator, proposed in a draft.

## Identity and configuration

Every agent taking part in a recorded task needs a memorable unique name and a registry
entry, including ChatGPTWork, reviewers and researchers who never commit. Register
before new work. Do not mint retrospective identities for unidentified past agents.
The shared GitHub login is a transport account and cannot identify human versus agent.

The stable fingerprint is SHA-256 of the immutable `identity` object in
`stewardship/agents.json`: assigned stable agent ID, immutable task ID, provider,
actual model where available (otherwise null), initial configuration version and
registration timestamp. Serialization is UTF-8 JSON with sorted keys, compact
separators, ASCII escapes, no newline and no floating-point values. This is specified
by the small standard-library validator, not an unspecified JSON serializer.
Display names/aliases live outside the identity; Ben's rename to Busybody Bob preserves
Mara Keel as an alias. Each configuration revision has its own hash. Preserve earlier
identity/configuration records; add revisions, never silently replace them.

Requested Sol6.1, STANDARD speed and extra-high effort are recorded as requested
configuration. Runtime model and speed metadata are unavailable here; neither a request
nor a fingerprint proves which platform configuration executed.
The ChatGPTWork entry covers the currently observed parent delegation only. It does
not backfill authorship for earlier PRs. Other historical contributors remain in
`historical-attribution.json` with explicit unknowns.

## Every recorded action

Append an action for research, read/review decisions, tests, handoffs, commits, PR/issue
edits, archive/closure/destructive actions, configuration/security-policy changes,
approvals/denials and available usage evidence. Small read-only mapping batches may
share a record when its references enumerate the inputs; mutations get individual
records. Record timestamp, name/fingerprint, configuration, immutable task/parent ID,
who authorized scope, target, outcome, linked issue/PR/commit/review/test artifact or
receipt and limitations. Parent/child relationships belong to the task records.
Unknown platform actions, runtime metadata, token use and monetary/credit spend stay
unknown; a user-reported balance is not a usage receipt or spending authority.

No further agents, Fast mode or spending expansion without parent approval. The parent
owns the queue and follow-through. Do not create persistent signing credentials, external
storage permissions or security settings as part of recording evidence.

`actions.jsonl` is an append-only convention with ordered hash chaining. Use:

```sh
uv run python scripts/agent_accountability.py verify
uv run python scripts/agent_accountability.py history "Busybody Bob"
uv run python scripts/agent_accountability.py history "PR #204"
uv run python scripts/agent_accountability.py append --payload /tmp/action.json
uv run python scripts/agent_accountability.py verify --expected-head HASH_FROM_PRIOR_RECEIPT
```

Publish checkpoint hashes in existing GitHub PR/issue comments and link the exact Git
commit containing the prefix. Keep later corrections as appended records. References
are evidence locations, not an assertion that the validator authenticates their contents.
CI checks that the base branch's existing action bytes remain an unchanged prefix.
No base action file exists for the initial adoption PR, so its first prefix is anchored
by explicit GitHub receipts. Parent-supplied independent checkpoints can detect truncation.

## Agent commits and human exemption

Agent commits must carry the responsible identity even if a GitHub connector uses
Ben's transport account. For example:

```text
Contribution-Kind: agent
Agent-Name: Busybody Bob
Agent-Fingerprint: sha256:REGISTRY_FINGERPRINT
Agent-Config: sha256:CONFIGURATION_HASH
Agent-Task: IMMUTABLE_TASK_ID
Agent-Action: record:SEQUENCE
Agent-Authority: https://github.com/benpshore/pdftextract/issues/225
```

Before publication run `commit --kind agent --message /tmp/commit-message.txt`.
CI validates marked commits and known registered agent emails in the PR range.
Ben's ordinary human commits need no new trailers, hooks or keys. The explicit
`--kind human` fixture is caller attestation, not identity verification. Unmarked
commits from unknown authors are reported as exempt/unknown; this cannot detect every
external agent using Ben's account. A marked/known agent cannot use the human exemption.
Existing history is never rewritten for missing metadata.

## Assurance and limits

The registry fingerprint provides lookup and integrity, **not cryptographic signing,
proof of authorship, correctness or refund entitlement**. An agent-writable JSON file
and its hashes are not independently immutable: an agent can rewrite the whole chain
and its local checkpoint. A valid truncated prefix looks valid without an earlier
independent expected hash. GitHub comments, commits and Actions receipts add dated
external evidence under existing access, but administrators can change/delete refs or
comments and platform retention can expire. No new independently immutable storage,
trusted timestamping or credentials are established.

Git object format stays unchanged. Do not treat SHA-256 registry hashes as Git object
IDs. Searchable records help identify recurring failure patterns and support a
usage/refund investigation when receipts exist; absent platform actions and costs
cannot be reconstructed or fabricated. The validator enforces structure, prefix
consistency and declared metadata, not external-agent compliance or security clearance.
