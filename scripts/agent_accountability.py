"""Small, voluntary registry, action-chain and commit-trailer validator.

Hashes identify records; they do not authenticate an author. See AGENT_ACCOUNTABILITY.md.
"""

import argparse
import hashlib
import json
import re
import shutil
import subprocess
from datetime import datetime
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REGISTRY = ROOT / "docs/stewardship/agents.json"
EVENTS = ROOT / "docs/stewardship/actions.jsonl"
ZERO = "0" * 64


def canonical(value):
    """UTF-8 sorted-key compact JSON; ASCII escaping, no floats, no newline."""
    if isinstance(value, dict):
        if not all(isinstance(k, str) for k in value):
            raise ValueError("canonical keys must be strings")
        for item in value.values():
            canonical(item)
    elif isinstance(value, list):
        for item in value:
            canonical(item)
    elif value is not None and not isinstance(value, (str, bool, int)):
        raise ValueError("canonical values must be null, boolean, integer, string, list or object")
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()


def digest(value):
    return hashlib.sha256(canonical(value)).hexdigest()


def registry(path=REGISTRY):
    document = json.loads(path.read_text())
    if document["schema"] != 1:
        raise ValueError("unsupported registry schema")
    identities, names, ids, tasks, emails = {}, set(), set(), set(), set()
    for entry in document["agents"]:
        identity = entry["identity"]
        required = {"agent_id", "task_id", "provider", "model", "config_version", "created_at"}
        if set(identity) != required or not identity["agent_id"] or not identity["task_id"]:
            raise ValueError("incomplete immutable identity")
        fingerprint = digest(identity)
        if entry["fingerprint"] != fingerprint:
            raise ValueError("identity fingerprint mismatch")
        if fingerprint in identities or identity["agent_id"] in ids or identity["task_id"] in tasks:
            raise ValueError("duplicate agent identity/task")
        for name in [entry["display_name"], *entry["aliases"]]:
            if not name or name.casefold() in names:
                raise ValueError("agent names and aliases must be unique")
            names.add(name.casefold())
        if identity["config_version"] not in [c["version"] for c in entry["configurations"]]:
            raise ValueError("initial configuration missing")
        versions = set()
        for config in entry["configurations"]:
            if config["version"] in versions or config["sha256"] != digest(
                {"version": config["version"], "settings": config["settings"]}
            ):
                raise ValueError("invalid configuration revision")
            versions.add(config["version"])
        for email in entry.get("git_emails", []):
            if email.casefold() in emails:
                raise ValueError("shared agent email")
            emails.add(email.casefold())
        ids.add(identity["agent_id"])
        tasks.add(identity["task_id"])
        identities[fingerprint] = entry
    return identities


def validate_payload(payload, identities):
    required = {
        "timestamp",
        "agent_fingerprint",
        "agent_name",
        "config_sha256",
        "task_id",
        "parent_task_id",
        "authorization",
        "action",
        "target",
        "outcome",
        "references",
        "usage",
        "limitations",
    }
    if set(payload) != required:
        raise ValueError("action fields incomplete or unknown")
    entry = identities.get(payload["agent_fingerprint"])
    if entry is None:
        raise ValueError("unregistered agent action")
    if payload["agent_name"] not in [entry["display_name"], *entry["aliases"]]:
        raise ValueError("agent action name mismatch")
    if payload["task_id"] != entry["identity"]["task_id"]:
        raise ValueError("agent action task mismatch")
    if payload["parent_task_id"] != entry["delegation"]["parent_task_id"]:
        raise ValueError("agent action parent mismatch")
    if payload["config_sha256"] not in [c["sha256"] for c in entry["configurations"]]:
        raise ValueError("unregistered action configuration")
    stamp = datetime.fromisoformat(payload["timestamp"].replace("Z", "+00:00"))
    if not payload["timestamp"].endswith("Z") or stamp.utcoffset().total_seconds() != 0:
        raise ValueError("action requires UTC timestamp")
    if not all(payload[k] for k in ("authorization", "action", "target", "outcome")):
        raise ValueError("action requires authorization, target and outcome")
    if not isinstance(payload["references"], list) or not payload["references"]:
        raise ValueError("action needs evidence or an explicit unavailable-evidence reference")
    if not isinstance(payload["limitations"], list):
        raise ValueError("limitations must be a list")
    usage = payload["usage"]
    if set(usage) != {"status", "receipt", "amount", "unit"}:
        raise ValueError("invalid usage evidence")
    if usage["status"] == "unknown":
        if any(usage[k] is not None for k in ("receipt", "amount", "unit")):
            raise ValueError("unknown usage cannot assert spend")
    elif usage["status"] != "receipt" or not usage["receipt"]:
        raise ValueError("usage must be unknown or linked to an available receipt")


def verify_events(path, identities, expected=None):
    previous, count = ZERO, 0
    for line in path.read_text().splitlines():
        if not line.strip():
            raise ValueError("blank action record")
        row = json.loads(line)
        if set(row) != {"sequence", "previous_sha256", "payload", "sha256"}:
            raise ValueError("invalid action envelope")
        count += 1
        if row["sequence"] != count or row["previous_sha256"] != previous:
            raise ValueError("action order/chain mismatch")
        validate_payload(row["payload"], identities)
        unsigned = {k: row[k] for k in ("sequence", "previous_sha256", "payload")}
        if row["sha256"] != digest(unsigned):
            raise ValueError("action digest mismatch")
        previous = row["sha256"]
    if expected is not None and previous != expected:
        raise ValueError("external checkpoint mismatch (possible rewrite or truncation)")
    return count, previous


def append_event(path, payload, identities):
    count, previous = verify_events(path, identities) if path.exists() else (0, ZERO)
    validate_payload(payload, identities)
    row = {"sequence": count + 1, "previous_sha256": previous, "payload": payload}
    row["sha256"] = digest(row)
    with path.open("a") as stream:
        stream.write(canonical(row).decode() + "\n")
    return row["sha256"]


def verify_prefix(old_path, new_path):
    """Local append-only convention; independent evidence still needs an external checkpoint."""
    if not new_path.read_bytes().startswith(old_path.read_bytes()):
        raise ValueError("previous action history was edited")


def check_commit(message, identities, kind="auto", author_email=None):
    trailers = {}
    # git trailers form a final paragraph. Examples in prose are not attribution.
    for line in message.strip().split("\n\n")[-1].splitlines():
        match = re.fullmatch(r"([A-Za-z-]+): (.+)", line)
        if match:
            key, value = match.groups()
            if key in trailers:
                raise ValueError("duplicate commit trailer")
            trailers[key] = value
    declared = trailers.get("Contribution-Kind")
    known_agent = any(
        author_email and author_email.casefold() in [e.casefold() for e in a.get("git_emails", [])]
        for a in identities.values()
    )
    agent_fields = any(k.startswith("Agent-") for k in trailers)
    if declared not in (None, "human", "agent"):
        raise ValueError("invalid contribution kind")
    if kind == "human":
        if known_agent or declared == "agent" or agent_fields:
            raise ValueError("agent contribution cannot use human exemption")
        return "human-exempt (caller attestation)"
    if kind == "auto" and not known_agent and declared != "agent" and not agent_fields:
        return "unmarked/exempt; human or unidentified external agent is not authenticated"
    if declared != "agent":
        raise ValueError("agent commit needs Contribution-Kind: agent")
    fingerprint = trailers.get("Agent-Fingerprint", "").removeprefix("sha256:")
    entry = identities.get(fingerprint)
    if entry is None or trailers.get("Agent-Fingerprint") != "sha256:" + fingerprint:
        raise ValueError("agent commit needs a registered SHA-256 fingerprint")
    if trailers.get("Agent-Name") not in [entry["display_name"], *entry["aliases"]]:
        raise ValueError("commit agent name mismatch")
    if trailers.get("Agent-Task") != entry["identity"]["task_id"]:
        raise ValueError("commit agent task mismatch")
    config = trailers.get("Agent-Config", "").removeprefix("sha256:")
    if trailers.get("Agent-Config") != "sha256:" + config or config not in [
        c["sha256"] for c in entry["configurations"]
    ]:
        raise ValueError("commit agent configuration mismatch")
    if not trailers.get("Agent-Action") or not trailers.get("Agent-Authority"):
        raise ValueError("commit needs an action ID and authorization reference")
    return "registered agent metadata (not authenticated)"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registry", type=Path, default=REGISTRY)
    sub = parser.add_subparsers(dest="command", required=True)
    verify = sub.add_parser("verify")
    verify.add_argument("--events", type=Path, default=EVENTS)
    verify.add_argument("--expected-head")
    verify.add_argument("--baseline", type=Path)
    append = sub.add_parser("append")
    append.add_argument("--events", type=Path, default=EVENTS)
    append.add_argument("--payload", type=Path, required=True)
    check = sub.add_parser("commit")
    check.add_argument("--message", type=Path, required=True)
    check.add_argument("--kind", choices=("auto", "human", "agent"), default="auto")
    check.add_argument("--author-email")
    audit = sub.add_parser("audit-range")
    audit.add_argument("--base", required=True)
    audit.add_argument("--head", required=True)
    search = sub.add_parser("history")
    search.add_argument("query", help="Agent name, fingerprint, task, PR, issue or action text")
    search.add_argument("--events", type=Path, default=EVENTS)
    args = parser.parse_args()
    identities = registry(args.registry)
    try:
        if args.command == "verify":
            result = verify_events(args.events, identities, args.expected_head)
            if args.baseline:
                verify_prefix(args.baseline, args.events)
            print(json.dumps({"records": result[0], "head_sha256": result[1]}))
        elif args.command == "append":
            print(append_event(args.events, json.loads(args.payload.read_text()), identities))
        elif args.command == "commit":
            print(check_commit(args.message.read_text(), identities, args.kind, args.author_email))
        elif args.command == "audit-range":
            if not all(
                re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", ref) for ref in (args.base, args.head)
            ):
                raise ValueError("audit range requires full hexadecimal commit IDs")
            git = shutil.which("git")
            if not git:
                raise ValueError("git is unavailable")
            # Arguments are full hex IDs; no shell or caller-supplied options.
            shas = subprocess.run(  # noqa: S603
                [git, "rev-list", "--reverse", args.base + ".." + args.head],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.splitlines()
            for sha in shas:
                data = subprocess.run(  # noqa: S603
                    [git, "show", "-s", "--format=%ae%n%B", sha],
                    check=True,
                    capture_output=True,
                    text=True,
                ).stdout
                email, message = data.split("\n", 1)
                print(sha, check_commit(message, identities, author_email=email))
        elif args.command == "history":
            verify_events(args.events, identities)
            for line in args.events.read_text().splitlines():
                if args.query.casefold() in line.casefold():
                    print(line)
    except (ValueError, KeyError) as error:
        parser.exit(1, f"accountability: {error}\n")


if __name__ == "__main__":
    main()
