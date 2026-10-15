"""Policy fixtures are synthetic; they do not mint identities for past agents."""

import copy
import importlib.util
import json
from pathlib import Path

import pytest

SPEC = importlib.util.spec_from_file_location(
    "agent_accountability", Path(__file__).resolve().parents[1] / "scripts/agent_accountability.py"
)
policy = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(policy)


@pytest.fixture
def identities(tmp_path):
    identity = {
        "agent_id": "fixture-reviewer",
        "task_id": "fixture-task",
        "provider": "fixture",
        "model": None,
        "config_version": 1,
        "created_at": "2026-10-04T00:00:00Z",
    }
    config = {"version": 1, "settings": {"effort": "fixture"}}
    config["sha256"] = policy.digest(config)
    entry = {
        "identity": identity,
        "fingerprint": policy.digest(identity),
        "display_name": "Fixture Reviewer",
        "aliases": [],
        "configurations": [config],
        "roles": ["review-only"],
        "git_emails": ["fixture-agent@agents.invalid"],
        "delegation": {"parent_task_id": "fixture-parent"},
    }
    path = tmp_path / "agents.json"
    path.write_text(json.dumps({"schema": 1, "agents": [entry]}))
    return policy.registry(path)


def payload(identities):
    fp, entry = next(iter(identities.items()))
    return {
        "timestamp": "2026-10-04T00:00:00Z",
        "agent_fingerprint": fp,
        "agent_name": entry["display_name"],
        "config_sha256": entry["configurations"][0]["sha256"],
        "task_id": "fixture-task",
        "parent_task_id": "fixture-parent",
        "authorization": "fixture user requested review",
        "action": "review",
        "target": "fixture PR",
        "outcome": "needs changes",
        "references": ["fixture://review/1"],
        "usage": {"status": "unknown", "receipt": None, "amount": None, "unit": None},
        "limitations": ["synthetic fixture"],
    }


def message(identities):
    fp, entry = next(iter(identities.items()))
    return (
        "Record fixture review\n\nContribution-Kind: agent\n"
        "Agent-Name: Fixture Reviewer\n"
        f"Agent-Fingerprint: sha256:{fp}\n"
        f"Agent-Config: sha256:{entry['configurations'][0]['sha256']}\n"
        "Agent-Task: fixture-task\nAgent-Action: record:1\nAgent-Authority: fixture user\n"
    )


def test_canonical_order_and_invalid_float():
    assert policy.digest({"b": 1, "a": "é"}) == policy.digest({"a": "é", "b": 1})
    assert policy.canonical({"a": "é"}) == b'{"a":"\\u00e9"}'
    with pytest.raises(ValueError, match="canonical values"):
        policy.digest({"cost": 1.5})


def test_human_is_exempt_without_changing_bens_workflow(identities):
    assert policy.check_commit("Ben's ordinary commit", identities, "human").startswith(
        "human-exempt"
    )
    assert "unmarked/exempt" in policy.check_commit("Ben's ordinary commit", identities)


def test_unidentified_external_agent_is_reported_as_unknown(identities):
    assert "unidentified external agent" in policy.check_commit("Unmarked", identities)


def test_agent_required_and_valid_trailers(identities):
    with pytest.raises(ValueError, match="Contribution-Kind"):
        policy.check_commit("No metadata", identities, "agent")
    assert "registered agent" in policy.check_commit(message(identities), identities, "agent")


@pytest.mark.parametrize(
    ("old", "new", "error"),
    [
        ("Fixture Reviewer", "Imposter", "name mismatch"),
        ("Agent-Task: fixture-task", "Agent-Task: other", "task mismatch"),
        ("Agent-Fingerprint: sha256:", "Agent-Fingerprint: sha1:", "fingerprint"),
        ("Agent-Config: sha256:", "Agent-Config: ", "configuration"),
        ("Agent-Action: record:1\n", "", "action ID"),
    ],
)
def test_invalid_agent_trailers(identities, old, new, error):
    with pytest.raises(ValueError, match=error):
        policy.check_commit(message(identities).replace(old, new), identities, "agent")


def test_agent_cannot_claim_human_exemption(identities):
    with pytest.raises(ValueError, match="human exemption"):
        policy.check_commit("Unmarked", identities, "human", "fixture-agent@agents.invalid")
    with pytest.raises(ValueError, match="Contribution-Kind"):
        policy.check_commit("Unmarked", identities, author_email="fixture-agent@agents.invalid")
    with pytest.raises(ValueError, match="human exemption"):
        policy.check_commit(message(identities), identities, "human")


def test_metadata_in_prose_is_not_a_trailer(identities):
    with pytest.raises(ValueError, match="Contribution-Kind"):
        policy.check_commit(
            message(identities) + "\nThis is subsequent prose.", identities, "agent"
        )


def test_review_only_research_tests_and_handoffs_need_identity(tmp_path, identities):
    path = tmp_path / "actions.jsonl"
    for action in ("review", "research", "test", "handoff", "approval", "configuration", "close"):
        record = payload(identities)
        record["action"] = action
        policy.append_event(path, record, identities)
    count, head = policy.verify_events(path, identities)
    assert count == 7
    assert len(head) == 64
    record["agent_fingerprint"] = "0" * 64
    with pytest.raises(ValueError, match="unregistered agent"):
        policy.append_event(path, record, identities)
    assert policy.verify_events(path, identities)[0] == 7


def test_edit_reorder_truncation_and_prefix(tmp_path, identities):
    path = tmp_path / "actions.jsonl"
    policy.append_event(path, payload(identities), identities)
    baseline = tmp_path / "baseline.jsonl"
    baseline.write_bytes(path.read_bytes())
    policy.append_event(path, payload(identities), identities)
    _, external_head = policy.verify_events(path, identities)
    original = path.read_text()
    policy.verify_prefix(baseline, path)
    path.write_text(original.replace("needs changes", "approved", 1))
    with pytest.raises(ValueError, match="digest mismatch"):
        policy.verify_events(path, identities)
    with pytest.raises(ValueError, match="history was edited"):
        policy.verify_prefix(baseline, path)
    path.write_text("\n".join(reversed(original.splitlines())) + "\n")
    with pytest.raises(ValueError, match="order/chain"):
        policy.verify_events(path, identities)
    path.write_bytes(baseline.read_bytes())
    # Without an external checkpoint a valid prefix cannot reveal truncation.
    assert policy.verify_events(path, identities)[0] == 1
    with pytest.raises(ValueError, match="checkpoint mismatch"):
        policy.verify_events(path, identities, external_head)


def test_wrong_parent_configuration_or_fabricated_spend_rejected(identities):
    for field, value in (("parent_task_id", "other"), ("config_sha256", "0" * 64)):
        row = payload(identities)
        row[field] = value
        with pytest.raises(ValueError):
            policy.validate_payload(row, identities)
    row = payload(identities)
    row["usage"]["amount"] = "57000"
    with pytest.raises(ValueError, match="unknown usage"):
        policy.validate_payload(row, identities)


def test_alias_and_config_revision_do_not_change_stable_identity(identities):
    revised = copy.deepcopy(identities)
    fp, entry = next(iter(revised.items()))
    entry["aliases"] = ["Fixture Reviewer"]
    entry["display_name"] = "Renamed Reviewer"
    config = {"version": 2, "settings": {"effort": "revised fixture"}}
    config["sha256"] = policy.digest(config)
    entry["configurations"].append(config)
    assert policy.digest(entry["identity"]) == fp
    row = payload(identities)
    row["config_sha256"] = config["sha256"]
    policy.validate_payload(row, revised)
