"""Generate or verify a candidate manifest from one committed CAD source tree."""
import argparse
from functools import cache
import json
from pathlib import Path
import re
import subprocess
import tomllib

from validate import ROOT, load_json, read, sha, verify_inputs


LOCK = "luma-cad.lock.json"
REGISTRY = "conformance/cad/registry-v1.json"
EVIDENCE = {
    "CAD-0": ["docs/cad/evidence.md"],
    "CAD-1": [
        "docs/cad/core-project.md",
        "docs/cad/cad1-t0-benchmark.json",
        "docs/cad/cad1-t1-benchmark.json",
    ],
    "CAD-2": ["conformance/cad/cad2-profile-v1.json", "docs/cad/brep-project.md"],
    "CAD-3": ["docs/cad/voxel-project.md"],
    "CAD-4": ["docs/cad/box-heat.md", "docs/cad/box-transport.md"],
    "CAD-5": ["docs/cad/cad5-profile.md", "docs/cad/elements-catalog.md"],
    "CAD-6": ["docs/cad/studio-still.md"],
}


def git(*args):
    result = subprocess.run(
        ["git", "-C", ROOT, *args], capture_output=True, check=False
    )
    if result.returncode:
        raise ValueError(result.stderr.decode(errors="replace").strip())
    return result.stdout


def candidate(revision):
    commit = git("rev-parse", "--verify", "--end-of-options", f"{revision}^{{commit}}").decode().strip()

    @cache
    def source(path):
        return git("show", f"{commit}:{path}")

    lock = load_json(source(LOCK))
    inputs = lock["contract"]["inputs"]
    verify_inputs(inputs, source)
    registry = load_json(source(REGISTRY))
    package = tomllib.loads(source("Cargo.toml").decode())["package"]
    evidence = {stage: list(paths) for stage, paths in EVIDENCE.items()}
    for path in sorted(inputs):
        match = re.fullmatch(r"conformance/cad/cad([0-6])-.*\.json", path)
        if match:
            stage = f"CAD-{match[1]}"
            if path not in evidence[stage]:
                evidence[stage].append(path)

    def reference(path):
        return {"path": path, "sha256": sha(source(path))}

    operations = {
        name: {
            "stage": row["stage"],
            "provider": row["provider"],
            "api_version": row["descriptor"]["operation_version"],
            "backend_version": row["backend_version"],
            "adapter_version": row["adapter_version"],
            "availability": row["availability"],
            "binding": row["binding"],
            "profiles": row["numerical_profile"],
            "modes": row["descriptor"]["modes"],
            "recorded_target_verification": row["target_verification"],
            "limits": row["limits"],
        }
        for name, row in registry["capabilities"].items()
    }
    return {
        "protocol": "luma.cad.release-manifest/1",
        "status": "candidate",
        "commit": commit,
        "language": {
            "name": package["name"],
            "version": package["version"],
            "structural_schema": lock["language"]["structural_schema"],
        },
        "cad": {
            "wire_schema": lock["contract"]["wire_schema"],
            "descriptor_schema": lock["contract"]["descriptor_schema"],
            "canonical_encoding": lock["contract"]["canonical_encoding"],
            "core_state_schema": load_json(source("schema/cad/core-state-v1.json"))["$id"],
        },
        "storage_formats": {
            "core_checkpoint": {
                "format": "living-ir.multi-part-checkpoint/2",
                "provider": "core",
            },
            "elements_project": {
                "container": ".lirela",
                "schema_version": "0.1",
                "provider": "elements",
            },
            "studio_workspace": {
                "sqlite_user_version": 1,
                "presentation_schema": 1,
                "provider": "studio",
            },
        },
        "lock": {**reference(LOCK), "profile": lock["profile"], "bootstrap_language": lock["language"]},
        "registry": reference(REGISTRY),
        "contract_inputs": inputs,
        "providers": lock["providers"],
        "adapters": lock["adapters"],
        "operations": operations,
        "qualification": {
            "candidate_runtime": dict.fromkeys(("t0", "t1", "t2", "t3", "t4"), "not_run"),
            "recorded_targets": lock["targets"],
            "evidence": {
                stage: [reference(path) for path in paths]
                for stage, paths in evidence.items()
            },
        },
    }


def encode(document):
    return json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--revision", default="HEAD", help="Git revision of the candidate")
    parser.add_argument("--verify", type=Path, metavar="MANIFEST", help="compare with the selected Git tree")
    args = parser.parse_args()
    try:
        expected = candidate(args.revision)
        if args.verify:
            if encode(read(args.verify)) != encode(expected):
                raise ValueError(f"Manifest differs from candidate {expected['commit']}")
            print(f"Verified CAD candidate manifest for {expected['commit']}")
        else:
            print(encode(expected), end="")
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.exit(1, f"release-manifest: {error}\n")


if __name__ == "__main__":
    main()
