"""Measure the headless CAD-1 project profile on Linux with a prebuilt CLI."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import resource
import secrets
import statistics
import subprocess
import tempfile
import time

import lcan

ROOT = Path(__file__).resolve().parents[2]
PROJECT = "00000000-0000-4000-8000-000000000001"
PART = "00000000-0000-4000-8000-000000000002"


def bundle(root, filename, origin, transform):
    frame = json.loads((ROOT / "examples/cad" / origin).read_text(encoding="utf-8"))
    entry = frame["entry"]
    frame["files"][entry] = transform(frame["files"][entry])
    typed = {"bundle_version": {"$u64": "1"}, "entry": entry, "files": frame["files"]}
    frame["source_digest"] = lcan.digest("luma.cad.source", lcan.encode(typed))
    path = root / filename
    path.write_text(json.dumps(frame), encoding="utf-8")
    return path


def percentiles(samples):
    return {
        "p50_ms": statistics.median(samples),
        "p95_ms": sorted(samples)[math.ceil(0.95 * len(samples)) - 1],
        "samples": len(samples),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cli", type=Path)
    parser.add_argument("--samples", type=int, default=30)
    parser.add_argument("--warmup", type=int, default=5)
    parser.add_argument("--build", required=True, help="Build identifier and configuration of the CLI")
    args = parser.parse_args()
    if platform.system() != "Linux":
        parser.error("This target records Linux child peak RSS")
    if not 1 <= args.samples <= 100 or not 0 <= args.warmup <= 20:
        parser.error("Use 1–100 samples and 0–20 warmup iterations")
    executable = args.cli.resolve(strict=True)
    data = {"prepare": [], "apply": [], "publish_job": []}
    started = time.time()
    with tempfile.TemporaryDirectory(prefix="luma-cad-benchmark-") as directory:
        root = Path(directory)
        db = root / "profile.db"
        env = dict(os.environ, LUMA_CAD_PROJECT_KEY=secrets.token_hex(32))
        env.pop("LUMA_CAD_TEST_CRASH_AT", None)
        env.pop("LIVING_IR_CHECKPOINT_FAULT", None)

        def call(*arguments, measure=None):
            before = time.perf_counter_ns()
            process = subprocess.run(
                [executable, db, PROJECT, *map(str, arguments)], env=env,
                capture_output=True, timeout=90,
            )
            elapsed_ms = (time.perf_counter_ns() - before) / 1e6
            frame = json.loads(process.stdout)
            if process.returncode or frame.get("protocol") != "luma.cad.core-project/1":
                raise RuntimeError(frame.get("error", "Unexpected CAD project response"))
            if measure:
                data[measure].append(elapsed_ms)
            return frame["result"]

        recipe = bundle(root, "recipe.json", "plate-geometry.json",
                        lambda text: 'use jobs\n' + text + '\njob = jobs.submit("definition", jobs.task("part_definition"))\n')
        commands = [bundle(root, f"width-{mm}.json", "plate-width-command.json",
                           lambda text: text.replace("units.mm(120)", f"units.mm({mm})"))
                    for mm in (120, 100)]
        call("init", PART, f"{1:032x}")
        initial = root / "initial.token"
        call("preview", "recipe", PART, 1, recipe, initial)
        committed = call("apply", initial, f"{2:032x}")
        call("work")
        for index in range(args.samples + args.warmup):
            measured = index >= args.warmup
            token = root / f"{index}.token"
            call("preview", "command", PART, committed["project_revision"], commands[index % 2], token,
                 measure="prepare" if measured else None)
            committed = call("apply", token, f"{index + 3:032x}", measure="apply" if measured else None)
            published = call("work", measure="publish_job" if measured else None)
            if published["state"] != "published" or committed["jobs"][0]["job"] != published["job"]:
                raise RuntimeError("Unexpected definition job")
            token.unlink()

    peak_rss = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss * 1024
    with executable.open("rb") as stream:
        cli_digest = hashlib.file_digest(stream, "sha256").hexdigest()
    lock = (ROOT / "luma-cad.lock.json").read_bytes()
    cpu = next((line.partition(":")[2].strip()
                for line in Path("/proc/cpuinfo").read_text().splitlines() if line.startswith("model name")), None)
    result = {
        "profile": "cad-1/t0-native-cli-plate-definition",
        "benchmark": "cold subprocess per preview, apply and publish",
        "timestamp_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(started)),
        "build": args.build,
        "cli_sha256": cli_digest,
        "lock_sha256": hashlib.sha256(lock).hexdigest(),
        "core_pin": json.loads(lock)["providers"]["core"]["commit"],
        "host": {"os": platform.platform(), "arch": platform.machine(), "cpu": cpu,
                 "logical_cores": os.cpu_count(), "gpu": None},
        "warmup_iterations": args.warmup,
        "child_peak_rss_bytes": peak_rss,
        "timings": {phase: percentiles(values) for phase, values in data.items()},
    }
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
