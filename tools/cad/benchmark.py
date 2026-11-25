"""Measure the existing Luma checker for the T0 bootstrap baseline on Linux."""
import argparse
import json
import math
import resource
import subprocess
import tempfile
import time
from pathlib import Path

SOURCE_BYTES = 32 * 1024
WARMUPS = 5
SAMPLES = 30


def source_bytes():
    assignments = "".join(f"v{index} = {index}\n" for index in range(500)).encode("ascii")
    return assignments + b"#" + b" " * (SOURCE_BYTES - len(assignments) - 2) + b"\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()

    with tempfile.TemporaryDirectory(prefix="luma-cad-baseline-") as directory:
        source = Path(directory) / "bootstrap.luma"
        source.write_bytes(source_bytes())

        def check():
            start = time.perf_counter_ns()
            completed = subprocess.run([binary, "check", source], check=True, capture_output=True)
            return (time.perf_counter_ns() - start) / 1_000_000, len(completed.stdout)

        first_process, output_size = check()
        for _ in range(WARMUPS):
            check()
        durations = sorted(check()[0] for _ in range(SAMPLES))
        print(json.dumps({
            "profile": "T0 Luma 0.1.3 release CLI check",
            "source_bytes": SOURCE_BYTES,
            "assignments": 500,
            "warmups": WARMUPS,
            "samples": SAMPLES,
            "first_process_ms": round(first_process, 3),
            "p50_ms": round(durations[math.ceil(SAMPLES * 0.50) - 1], 3),
            "p95_ms": round(durations[math.ceil(SAMPLES * 0.95) - 1], 3),
            "peak_child_rss_kib": resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss,
            "first_stdout_bytes": output_size,
        }, indent=2))


if __name__ == "__main__":
    main()
