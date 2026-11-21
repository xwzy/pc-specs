#!/usr/bin/env python3
"""Run a release binary in the background and record bounded-resource checks.

macOS/Linux, standard library only. Never attaches to or stops an existing app.
"""

import argparse
import json
import math
import pathlib
import subprocess
import sys
import time


def positive(value):
    value = float(value)
    if not math.isfinite(value) or value <= 0:
        raise argparse.ArgumentTypeError("must be finite and positive")
    return value


def sample(pid, elapsed):
    fields = subprocess.check_output(
        ["ps", "-p", str(pid), "-o", "time=,rss="], text=True
    ).split()
    parts = fields[0].replace("-", ":").split(":")
    cpu_seconds = sum(float(part) * factor for part, factor in zip(reversed(parts), [1, 60, 3600, 86400]))
    if sys.platform == "darwin":
        threads = len(subprocess.check_output(
            ["ps", "-M", "-p", str(pid), "-o", "pid="], text=True
        ).splitlines())
    else:
        threads = len(list(pathlib.Path(f"/proc/{pid}/task").iterdir()))
    return {
        "elapsed_s": round(elapsed, 3),
        "cpu_s": cpu_seconds,
        "rss_mib": round(int(fields[1]) / 1024, 3),
        "threads": threads,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=pathlib.Path, default=pathlib.Path("target/release/pc-specs"))
    parser.add_argument("--duration", type=positive, default=300, help="seconds; use 86400 for a day")
    parser.add_argument("--warmup", type=positive, default=30)
    parser.add_argument("--interval", type=positive, default=5)
    parser.add_argument("--max-rss-growth", type=positive, default=32, help="MiB above the first post-warmup sample")
    parser.add_argument("--max-thread-growth", type=positive, default=4)
    parser.add_argument("--output", type=pathlib.Path, default=pathlib.Path("dist/background-check.json"))
    args = parser.parse_args()
    if sys.platform not in ("darwin", "linux"):
        parser.error("resource sampling currently supports macOS and Linux")
    if args.duration <= args.warmup + args.interval:
        parser.error("duration must exceed warmup plus one sampling interval")
    if not args.binary.is_file():
        parser.error("build the release binary first: cargo build --release --locked -p pc-specs")

    args.output.parent.mkdir(parents=True, exist_ok=True)
    report = {"platform": sys.platform, "samples": [], "errors": [], "passed": False}
    log_path = args.output.with_suffix(".log")
    with log_path.open("w") as log:
        process = subprocess.Popen([str(args.binary.resolve()), "--background"], stdout=log, stderr=log)
        started = time.monotonic()
        try:
            while True:
                elapsed = time.monotonic() - started
                if process.poll() is not None:
                    report["errors"].append(f"application exited early ({process.returncode}); close any existing instance before testing")
                    break
                report["samples"].append(sample(process.pid, elapsed))
                args.output.write_text(json.dumps(report, indent=2) + "\n")
                if elapsed >= args.duration:
                    break
                time.sleep(min(args.interval, args.duration - elapsed))
        except KeyboardInterrupt:
            report["errors"].append("check interrupted")
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            report["errors"].append(str(error))
        finally:
            # Stop only the child we started. A normal SIGTERM is tried before
            # force-killing it; neither operation affects an existing instance.
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()

    steady = [row for row in report["samples"] if row["elapsed_s"] >= args.warmup]
    if len(steady) < 2:
        report["errors"].append("insufficient post-warmup samples")
    else:
        first, last = steady[0], steady[-1]
        report["steady"] = {
            "duration_s": round(last["elapsed_s"] - first["elapsed_s"], 3),
            "average_cpu_percent_of_one_core": round(100 * (last["cpu_s"] - first["cpu_s"]) / (last["elapsed_s"] - first["elapsed_s"]), 3),
            "rss_first_mib": first["rss_mib"],
            "rss_last_mib": last["rss_mib"],
            "rss_peak_mib": max(row["rss_mib"] for row in steady),
            "threads_first": first["threads"],
            "threads_peak": max(row["threads"] for row in steady),
        }
        if report["steady"]["rss_peak_mib"] - first["rss_mib"] > args.max_rss_growth:
            report["errors"].append("memory growth exceeded the selected limit")
        if report["steady"]["threads_peak"] - first["threads"] > args.max_thread_growth:
            report["errors"].append("thread growth exceeded the selected limit")
    report["passed"] = not report["errors"]
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: value for key, value in report.items() if key != "samples"}, ensure_ascii=False, indent=2))
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
