"""Validate CAD contracts and generate their bilingual capability reference."""
import argparse
import hashlib
import json
import re
import subprocess
from copy import deepcopy
from pathlib import Path

from jsonschema import Draft202012Validator, FormatChecker, ValidationError
from referencing import Registry, Resource

import lcan

ROOT = Path(__file__).resolve().parents[2]
SCHEMAS = ROOT / "schema/cad"
FIXTURES = ROOT / "conformance/cad"
DOCS = ROOT / "docs/cad"
SPEC = ROOT / "spec/cad/Luma-CAD-Development-Specification-v1.0-RU.md"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, f"Duplicate JSON key {key!r}")
        result[key] = value
    return result


def load_json(source):
    def reject(token):
        raise ValueError(f"Invalid JSON constant {token}")

    return json.loads(source, object_pairs_hook=unique_object, parse_constant=reject)


def read(path):
    return load_json(path.read_text(encoding="utf-8"))


def sha(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def typed_manifest(value):
    if type(value) is int:
        return {"$u64": str(value)}
    if isinstance(value, dict):
        return {key: typed_manifest(item) for key, item in value.items()}
    if isinstance(value, list):
        return [typed_manifest(item) for item in value]
    return value


def fingerprint(domain, data):
    return lcan.digest(domain, lcan.encode(typed_manifest(data)))


def verify_inputs(inputs, read_bytes):
    for path, expected in inputs.items():
        require(sha(read_bytes(path)) == expected, f"Changed build input {path}")


def verify_lock(lock, repositories):
    verify_inputs(lock["contract"]["inputs"], lambda path: (ROOT / path).read_bytes())
    for name, provider in lock["providers"].items():
        if provider["selection"] == "selected":
            require("inputs" in provider and "toolchain" in provider, f"Missing build inputs for {name}")
            fields = {key: value for key, value in provider.items() if key != "build_fingerprint"}
            require(fingerprint("luma.cad.build.provider", fields) == provider["build_fingerprint"],
                    f"Fingerprint mismatch for {name}")
    for name, adapter in lock["adapters"].items():
        fields = {key: value for key, value in adapter.items() if key != "build_fingerprint"}
        fields["providers"] = {
            provider: lock["providers"][provider]["build_fingerprint"] for provider in adapter["compatible_with"]
        }
        require(fingerprint("luma.cad.build.adapter", fields) == adapter["build_fingerprint"],
                f"Adapter fingerprint mismatch for {name}")
    for name, target in lock["targets"].items():
        require(all(lock["providers"][provider]["selection"] == "selected" for provider in target["providers"]),
                f"Unselected provider on target {name}")
    for name, checkout in repositories.items():
        provider = lock["providers"][name]

        def source(path):
            # git show reads the pinned bytes even from a checkout composed with the V2 patch.
            return subprocess.check_output(["git", "-C", str(checkout), "show", f"{provider['commit']}:{path}"])

        head = subprocess.check_output(["git", "-C", str(checkout), "rev-parse", "HEAD"], text=True).strip()
        require(head == provider["commit"], f"Unexpected HEAD for {name}: {head}")
        for path in provider["review_paths"]:
            source(path)
        verify_inputs(provider.get("inputs", {}), source)
        for adapter in lock["adapters"].values():
            if adapter["provider"] == name:
                verify_inputs(adapter["inputs"], source)


def wire_examples():
    appendix = SPEC.read_text(encoding="utf-8").split('## Приложение C.')[1]
    return [load_json(block) for block in re.findall(r'```json\n(.*?)\n```', appendix, re.S)]


def expect_error(callback, *errors):
    try:
        callback()
    except errors:
        return
    raise ValueError("Negative fixture was accepted")


def validate_wire(value, validator, operations):
    validator.validate(value)
    if isinstance(value, dict) and value.get("action") == "prepare":
        bundle = value["source_bundle"]
        require(bundle["entry"] in bundle["files"], "Source bundle lacks its entry")
    if isinstance(value, dict) and "operation_version" in value:
        operation_id = value["operation_id"]
        require(operations.get(operation_id) == value["operation_version"]["major"],
                "UnsupportedOperationMajor")
    # UTF-8 encoding also rejects an escaped unpaired surrogate admitted by a JSON parser.
    json.dumps(value, ensure_ascii=False).encode("utf-8")


def verify_wire(validator, registry, operations):
    prepare, commit, receipt, error, _ = wire_examples()
    for example in (prepare, commit, receipt, error):
        validate_wire(example, validator("wire-v1"), operations)
    for patch in ({"schema_version": 2}, {"unknown_required": True}, {"expected_revision": 1}):
        expect_error(lambda: validator("wire-v1").validate({**commit, **patch}), ValidationError)
    command = {**prepare, "mode": "command"}
    expect_error(lambda: validator("wire-v1").validate(command), ValidationError)
    expect_error(lambda: load_json('{"mode":"recipe","mode":"command"}'), ValueError)
    expect_error(lambda: load_json('{"value":NaN}'), ValueError)
    expect_error(lambda: b"\xff".decode("utf-8"), UnicodeDecodeError)
    expect_error(lambda: validate_wire({**prepare, "source_bundle": {**prepare["source_bundle"],
                 "entry": "missing.luma"}}, validator("wire-v1"), operations), ValueError)
    for value in ("0", "9007199254740993", "18446744073709551615"):
        validator("wire-v1", "U64").validate(value)
        require(str(lcan.integer(value)) == value, "Wire integer rounded")
    for value in ("01", "18446744073709551616", -1):
        expect_error(lambda: validator("wire-v1", "U64").validate(value), ValidationError)
    for value in ("NaN", "Infinity", "-Infinity", "1e309"):
        expect_error(lambda: lcan.f64(value), ValueError)
    descriptor = deepcopy(registry["capabilities"]["cad.plate"]["descriptor"])
    validate_wire(descriptor, validator("wire-v1", "HostDescriptor"), operations)
    descriptor["operation_version"]["major"] += 1
    expect_error(lambda: validate_wire(descriptor, validator("wire-v1", "HostDescriptor"), operations), ValueError)


def verify_vectors():
    vectors = read(FIXTURES / "lcan-vectors-v1.json")
    results = {}
    for vector in vectors:
        name = vector["name"]
        require(name not in results, f"Duplicate LCAN vector {name}")
        payload = lcan.encode(vector["typed"])
        require(payload == vector["canonical_utf8"].encode(), f"Unexpected LCAN bytes for {name}")
        require(lcan.digest(vector["domain"], payload) == vector["digest"], f"Unexpected LCAN digest for {name}")
        if "equivalent" in vector:
            require(lcan.encode(vector["equivalent"]) == payload, f"Equivalent vector differs for {name}")
        results[name] = payload
    require(results["source-exact-utf8"] != results["source-comment-change"], "Source comments must affect identity")
    ordered = {"ordered": ["a", "z"], "set": {"$fixture_keyed_set": [{"key": "z"}, {"key": "a"}]}}
    require(lcan.encode(ordered) != results["set-versus-command-order"], "Command order disappeared")
    expect_error(lambda: lcan.encode({"$fixture_recipe": [{"key": "a", "dependencies": ["a"]}]}), ValueError)
    expect_error(lambda: lcan.encode({"$fixture_keyed_set": [{"key": "a"}, {"key": "a"}]}), ValueError)


def guard_state(lock, adapter, generation):
    return {
        "lock_digest": fingerprint("luma.cad.lock", lock),
        "provider_fingerprint": lock["adapters"][adapter]["build_fingerprint"],
        "captured_resource_generation": str(lcan.integer(generation)),
    }


def verify_guards(lock):
    accepted = guard_state(lock, "core-view-v2", "9007199254740993")

    def admit(candidate):
        require(candidate == accepted, "StalePreparedCandidate")

    admit(dict(accepted))
    changed_lock = deepcopy(lock)
    changed_lock["language"]["version"] = "0.1.4"
    expect_error(lambda: admit(guard_state(changed_lock, "core-view-v2", "9007199254740993")), ValueError)
    changed_provider = deepcopy(lock)
    changed_provider["adapters"]["core-view-v2"]["build_fingerprint"] = "sha256:" + "0" * 64
    new_guard = {**accepted, "provider_fingerprint":
                 changed_provider["adapters"]["core-view-v2"]["build_fingerprint"]}
    require(new_guard["provider_fingerprint"] != accepted["provider_fingerprint"], "Fingerprint did not change")
    expect_error(lambda: admit(new_guard), ValueError)
    expect_error(lambda: admit(guard_state(lock, "core-view-v2", "9007199254740994")), ValueError)


def verify_host_contract(registry, validator):
    descriptor_set = {name: row["descriptor"] for name, row in registry["capabilities"].items()}

    def match_registry(checker, runtime):
        domain = "luma.cad.registry"
        require(fingerprint(domain, checker) == fingerprint(domain, runtime), "HostRegistryMismatch")

    match_registry(descriptor_set, deepcopy(descriptor_set))
    different = deepcopy(descriptor_set)
    different["cad.plate"]["effects"] = ["Pure"]
    expect_error(lambda: match_registry(descriptor_set, different), ValueError)

    def resolve(mode, name, siblings):
        if mode == "cad" and name in registry["sealed_modules"]:
            require(name not in siblings, "ModuleNameCollision")
            return "host"
        return "sibling" if name in siblings else None

    require(resolve("cad", "cad", set()) == "host", "Host module resolution drift")
    expect_error(lambda: resolve("cad", "cad", {"cad"}), ValueError)
    require(resolve("legacy", "cad", {"cad"}) == "sibling", "Legacy resolver changed")
    execution = {
        "outputs": [{"key": "width", "value": {"type": "Quantity", "value": {
            "value_si": "0.1", "dimension": "Length", "quantity_kind": "linear"
        }}}],
        "diagnostics": [], "source_map": [], "view_intents": [], "task_intents": [],
        "consumed_resources": {"expression_steps": "9", "planner_memory_bytes": "1024", "prepare_deadline_ms": "30000"},
    }
    validator("wire-v1", "ExecutionResult").validate(execution)
    expect_error(lambda: validator("wire-v1", "ExecutionResult").validate({**execution, "outputs": "printed"}),
                 ValidationError)


def verify_domain_contracts(validator):
    prepare, _, receipt, _, _ = wire_examples()
    target = prepare["target"]
    entity = receipt["result"]["changed_ids"][0]
    pin = {"target": target, "revision": "8", "entity_id": entity, "type": "BodyRef"}
    core_state = {
        "schema": "luma.cad.recipe-instance-state/1", "schema_version": 1,
        "instance_id": prepare["context"]["recipe_instance_id"], "part_id": target["document_id"],
        "accepted_source": {"digest": prepare["source_bundle"]["source_digest"], "artifact_id": entity},
        "identity_registry": {"blank": {"entity_id": entity, "type": "BodyRef", "state": "live"}},
        "external_pins": [],
        "overrides": {"count": {"type": "Count", "value": "15"}, "scalar": {"type": "Scalar", "value": "15"}},
        "public_outputs": {"solid": pin},
    }
    validator("core-state-v1").validate(core_state)
    expect_error(lambda: validator("core-state-v1").validate({**core_state, "extension": {}}), ValidationError)

    frame = {"origin_m": ["0", "0", "0"], "x": ["1", "0", "0"],
             "y": ["0", "1", "0"], "z": ["0", "0", "1"]}
    boundary = {"schema_version": 1, "frame": frame, "length_unit": "m", "angular_unit": "rad"}
    sketch = {**boundary, "kind": "Sketch", "id": entity, "constraints": [],
              "curves": [{"id": entity, "kind": "circle", "center": ["0", "0"], "radius": "0.02"}]}
    profile = {**boundary, "kind": "Profile", "source": {**pin, "type": "SketchRef"},
               "tolerance_m": "0.00001", "minimum_web_m": "0.0001",
               "loops": [{"id": entity, "role": "outer", "edges": [{"curve_id": entity, "direction": "forward"}]}]}
    for value in (sketch, profile):
        validator("sketch-profile-v1").validate(value)
        expect_error(lambda: validator("sketch-profile-v1").validate({**value, "length_unit": "mm"}), ValidationError)


def verify_sketch_profile(validator, path):
    envelope = read(path)
    pair = envelope.get("result", envelope)
    sketch, profile = pair["sketch"], pair["profile"]
    validator("sketch-profile-v1", "Sketch").validate(sketch)
    validator("sketch-profile-v1", "Profile").validate(profile)
    source = profile["source"]
    require(source["type"] == "Sketch" and source["entity_id"] == sketch["id"], f"Sketch pin mismatch: {path}")
    require(sketch["frame"] == profile["frame"], f"Profile frame mismatch: {path}")
    curve_ids = {item["id"] for item in sketch["curves"]}
    require(len(curve_ids) == len(sketch["curves"]), f"Duplicate curve ID: {path}")
    loops = profile["loops"]
    require(len({item["id"] for item in loops}) == len(loops), f"Duplicate loop ID: {path}")
    outer = [item for item in loops if item["role"] == "outer"]
    require(len(outer) == 1 and "parent_id" not in outer[0], f"Expected one root loop: {path}")
    loop_edges = []
    for loop in loops:
        if loop["role"] == "hole":
            require(loop.get("parent_id") == outer[0]["id"], f"Hole lacks its outer loop: {path}")
        require(bool(loop["edges"]), f"Profile loop has no edges: {path}")
        loop_edges.extend(item["curve_id"] for item in loop["edges"])
    require(len(loop_edges) == len(set(loop_edges)) and set(loop_edges) <= curve_ids,
            f"Profile edge does not uniquely refer to an exported curve: {path}")
    for constraint in sketch["constraints"]:
        require(set(constraint["curve_ids"]) <= curve_ids, f"Unknown constraint curve: {path}")


def render(registry, locale):
    heading = {
        "en": ("CAD capabilities", "Runtime availability is scoped to the verified integration profiles.",
               "Operation | Stage | Backend | Adapter | Luma binding | Runtime | T0 / T1 | Reason"),
        "ru": ("Возможности CAD", "Доступность операций указана для проверенных профилей интеграции.",
               "Операция | Этап | Backend | Адаптер | Binding Luma | Runtime | T0 / T1 | Причина"),
    }
    title, intro, columns = heading[locale]
    lines = [f"# {title}", "", "<!-- Generated by tools/cad/validate.py --write-docs. -->", "", intro, "",
             "[CAD](README.md) · [Registry](../../conformance/cad/registry-v1.json) · "
             "[CAD-2](brep-project.md)", "",
             f"| {columns} |", "| --- | --- | --- | --- | --- | --- | --- | --- |"]
    for name, row in sorted(registry["capabilities"].items()):
        targets = row["target_verification"]
        lines.append(f"| `{name}` | {row['stage']} | {row['backend']} | {row['adapter']} | {row['binding']} | "
                     f"{row['availability']} | {targets['t0']} / {targets['t1']} | {row['reason'][locale]} |")
    lines.extend(["", "## " + ("Signatures" if locale == "en" else "Сигнатуры"), ""])
    for name, row in sorted(registry["capabilities"].items()):
        descriptor = row["descriptor"]
        parameters = ", ".join(f"{item['name']}: {item['type']}" for item in descriptor["parameters"])
        effects = ", ".join(descriptor["effects"])
        lines.append(f"- `{name}({parameters}) → {descriptor['result_type']}` · `{effects}`")
    return "\n".join(lines) + "\n"


def validate_links(files):
    for path in files:
        contents = path.read_text(encoding="utf-8")
        for link in re.findall(r"\]\(([^()\s]+)\)", contents):
            if "://" in link:
                continue
            target, _, anchor = link.partition("#")
            if target:
                require((path.parent / target).is_file(), f"Broken link in {path.name}: {link}")
            if anchor and not target:
                require(f'id="{anchor}"' in contents, f"Broken anchor in {path.name}: {link}")


def verify_cad2_profile(lock, registry):
    profile = read(FIXTURES / "cad2-profile-v1.json")
    require(profile["schema_version"] == 1 and profile["profile"] == "cad2-profile-v1",
            "Unsupported CAD-2 profile")
    require(re.fullmatch("[0-9a-f]{40}", profile["implementation_commit"]), "Invalid CAD-2 commit")
    require(profile["provider_commits"] == {
        name: lock["providers"][name]["commit"] for name in ("core", "ir-2d", "brep", "view")
    }, "CAD-2 provider pin mismatch")
    require(set(profile["targets"]) == set(lock["targets"]), "Missing CAD-2 target description")
    checks = profile["acceptance"]
    require(set(checks) == {f"AT-{number:03}" for number in (37, 44, 49, 54, 57, 58, 68)},
            "Incomplete CAD-2 acceptance")
    for path in checks.values():
        require((ROOT / path).is_file(), f"Missing CAD-2 test {path}")
    operations = profile["operations"]
    require(operations and set(operations) <= set(registry["capabilities"]), "Unknown CAD-2 operation")
    for name, row in registry["capabilities"].items():
        targets = row["target_verification"]
        require(set(targets) == set(lock["targets"]), f"Missing targets for {name}")
        if name in operations:
            entry = operations[name]
            require(row["stage"] == "CAD-2" and row["binding"] == "qualified_profile"
                    and row["adapter"] == "qualified_profile" and row["availability"] == "available"
                    and entry["target"] == "t0" and entry["entry_points"]
                    and entry["checks"] and set(entry["checks"]) <= set(checks)
                    and targets == {"t0": "passed", "t1": "not_run"}, f"Unqualified CAD-2 status for {name}")
        else:
            require(row["binding"] == "planned" and row["availability"] == "unavailable"
                    and set(targets.values()) == {"not_run"}, f"Unsupported status inflation for {name}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write-docs", action="store_true")
    parser.add_argument("--provider", action="append", default=[], metavar="ID=CHECKOUT")
    parser.add_argument("--sketch-profile", action="append", default=[], type=Path, metavar="JSON")
    args = parser.parse_args()
    repositories = dict(item.split("=", 1) for item in args.provider)
    schemas = {path.stem: read(path) for path in SCHEMAS.glob("*.json")}
    resources = Registry().with_resources(
        (schema["$id"], Resource.from_contents(schema)) for schema in schemas.values()
    )
    for schema in schemas.values():
        Draft202012Validator.check_schema(schema)

    def validator(name, definition=None):
        schema = schemas[name]
        if definition:
            schema = {"$ref": schema["$id"] + "#/$defs/" + definition}
        return Draft202012Validator(schema, registry=resources, format_checker=FormatChecker())

    registry = read(FIXTURES / "registry-v1.json")
    lock = read(ROOT / "luma-cad.lock.json")
    validator("lock-v1").validate(lock)
    validator("registry-v1").validate(registry)
    verify_lock(lock, repositories)
    verify_cad2_profile(lock, registry)
    operations = {}
    for name, row in registry["capabilities"].items():
        descriptor = row["descriptor"]
        require(descriptor["qualified_name"] == name, f"Descriptor name mismatch for {name}")
        require(name.split(".")[0] in registry["sealed_modules"], f"Unsealed module for {name}")
        operation = descriptor["operation_id"]
        require(operation not in operations, f"Duplicate operation ID {operation}")
        operations[operation] = descriptor["operation_version"]["major"]
        if row["provider"] != "luma":
            provider = lock["providers"][row["provider"]]
            require(descriptor["provider_contract"] == row["provider"] + "@" + provider["commit"],
                    f"Descriptor provider pin mismatch for {name}")
            require(set(row["evidence"]) <= set(provider["review_paths"]), f"Unreviewed evidence for {name}")
    verify_wire(validator, registry, operations)
    verify_host_contract(registry, validator)
    verify_domain_contracts(validator)
    for path in args.sketch_profile:
        verify_sketch_profile(validator, path)
    verify_guards(lock)
    verify_vectors()
    for locale in ("ru", "en"):
        path = DOCS / f"capabilities-{locale}.md"
        expected = render(registry, locale)
        if args.write_docs:
            path.write_text(expected, encoding="utf-8")
        require(path.read_text(encoding="utf-8") == expected, f"Stale capability reference {path.name}")
    validate_links([*DOCS.glob("*.md"), *SPEC.parent.glob("*.md")])
    print(f"Validated CAD contracts, lock, {len(operations)} capability entries, CAD-2 evidence and documentation")


if __name__ == "__main__":
    main()
