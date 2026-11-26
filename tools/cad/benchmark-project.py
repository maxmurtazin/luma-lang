"""Measure the headless CAD-1 project profile on Linux with a prebuilt CLI."""
import argparse
from contextlib import closing
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import resource
import secrets
import sqlite3
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


def read_optional(path):
    try:
        return Path(path).read_text().strip()
    except OSError:
        return None


class BenchmarkFailure(RuntimeError):
    def __init__(self, problem):
        self.problem = problem
        super().__init__(problem)


def snapshot(source, destination):
    with closing(sqlite3.connect(source)) as current, closing(sqlite3.connect(destination)) as copy:
        current.backup(copy)


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
    phases = ("prepare", "apply", "cancel_job", "publish_job")
    data = {phase: [] for phase in phases}
    output = {phase: [] for phase in phases}
    token_sizes = []
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
            if frame.get("protocol") != "luma.cad.core-project/1":
                raise RuntimeError("Unexpected CAD project response")
            if process.returncode:
                raise BenchmarkFailure(frame["error"])
            if measure:
                data[measure].append(elapsed_ms)
                output[measure].append(len(process.stdout))
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
        if call("work")["state"] != "published":
            raise RuntimeError("Could not prepare the benchmark fixture")
        seed = db
        for index in range(args.samples + args.warmup):
            measured = index >= args.warmup
            db = root / f"sample-{index}.db"
            snapshot(seed, db)
            token = root / f"{index}.token"
            call("preview", "command", PART, committed["project_revision"], commands[0], token,
                 measure="prepare" if measured else None)
            accepted = call("apply", token, f"{3:032x}", measure="apply" if measured else None)
            job = accepted["jobs"][0]["job"]
            if measured:
                token_sizes.append(token.stat().st_size)
            cancelled = call("job-cancel", job, measure="cancel_job" if measured else None)
            if cancelled["state"] != "cancelled":
                raise RuntimeError("Expected cancellation before the worker claims the job")
            call("job-retry", job, cancelled["attempt"])
            published = call("work", measure="publish_job" if measured else None)
            if published["state"] != "published" or job != published["job"]:
                raise RuntimeError("Unexpected definition job")

        peak_rss = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss * 1024
        db = root / "growth.db"
        snapshot(seed, db)
        growth = {"requested_edits": 35, "applied_edits": 0, "refusal": None}
        for index in range(growth["requested_edits"]):
            token = root / f"growth-{index}.token"
            operation = f"{index + 3:032x}"
            try:
                call("preview", "command", PART, committed["project_revision"], commands[index % 2], token)
                committed = call("apply", token, operation)
            except BenchmarkFailure as error:
                if error.problem.get("code") != "ResourceLimit":
                    raise
                if call("inspect")["project_revision"] != committed["project_revision"]:
                    raise RuntimeError("Budget refusal changed the accepted revision")
                growth["refusal"] = error.problem
                break
            growth["applied_edits"] += 1

    with executable.open("rb") as stream:
        cli_digest = hashlib.file_digest(stream, "sha256").hexdigest()
    lock = (ROOT / "luma-cad.lock.json").read_bytes()
    cpu = next((line.partition(":")[2].strip()
                for line in Path("/proc/cpuinfo").read_text().splitlines() if line.startswith("model name")), None)
    result = {
        "profile": "cad-1/t0-native-cli-plate-definition",
        "benchmark": "cold subprocess per phase against independent copies of the same accepted project",
        "timestamp_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(started)),
        "build": args.build,
        "cli_sha256": cli_digest,
        "lock_sha256": hashlib.sha256(lock).hexdigest(),
        "core_pin": json.loads(lock)["providers"]["core"]["commit"],
        "host": {"os": platform.platform(), "arch": platform.machine(), "cpu": cpu,
                 "logical_cores": os.cpu_count(), "gpu": None,
                 "cgroup_cpu_max": read_optional("/sys/fs/cgroup/cpu.max"),
                 "cgroup_memory_max_bytes": read_optional("/sys/fs/cgroup/memory.max")},
        "warmup_iterations": args.warmup,
        "child_peak_rss_bytes": peak_rss,
        "timings": {phase: percentiles(values) for phase, values in data.items()},
        "max_stdout_bytes": {phase: max(values) for phase, values in output.items()},
        "max_prepared_token_bytes": max(token_sizes),
        "growing_history": growth,
    }
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
