#!/usr/bin/env python3
"""Fork-only physical-edit benchmark. Results are JSON, including failed builds."""
import argparse
import collections
import fcntl
import hashlib
import json
import os
from pathlib import Path
import random
import statistics
import subprocess
import time


CASES = ["noop", "touch", "top", "generic", "type", "bottom", "compensated", "body"]


def edit(source, case, cfg):
    if case in ("noop", "touch"):
        return source
    if case == "top":
        return "\n" + source
    if case == "bottom":
        pos = source.index(cfg["bottom_marker"]) if "bottom_marker" in cfg else source.rfind("\n}")
        assert pos >= 0
        return source[:pos] + "\n" + source[pos:]
    marker = cfg["generic_marker" if case in ("generic", "compensated") else "type_marker"]
    if case == "body":
        old, new = cfg["body_replace"]
        assert source.count(old) == 1
        return source.replace(old, new, 1)
    assert source.count(marker) == 1, marker
    pos = source.index(marker)
    result = source[:pos] + "\n" + source[pos:]
    if case == "compensated":
        end = source.index(cfg["compensated_end_marker"], pos)
        assert source[end - 2:end] == "\n\n"
        result = result[:end] + result[end + 1:]
        assert result.count("\n") == source.count("\n")
    return result


def capture(command, **kwargs):
    return subprocess.check_output(command, text=True, **kwargs).strip()


def summarize(records):
    groups = collections.defaultdict(list)
    for row in records:
        if row["exit_code"] == 0:
            groups[(row["compiler"], row["debug"], row["opt"], row["case"])].append(row["wall_seconds"])
    return [{"compiler": k[0], "debug": k[1], "opt": k[2], "case": k[3],
             "n": len(v), "median_seconds": statistics.median(v),
             "min_seconds": min(v), "max_seconds": max(v),
             "sufficient_repeated_samples": len(v) >= 5} for k, v in sorted(groups.items())]


def diagnostics(log):
    events = []
    writes = collections.Counter()
    for line in log.read_text().splitlines():
        if line.startswith("rustc-relocation: "):
            event = json.loads(line.removeprefix("rustc-relocation: "))
            assert event["schema"] == 1
            if event["event"] == "cache_write":
                writes[event["reason"]] += 1
                continue
            assert event["event"] == "cgu"
            events.append(event)
    count = sum(event["count"] for event in events)
    outcomes = collections.Counter()
    reasons = collections.Counter()
    skipped = collections.Counter()
    eligible = 0
    candidates = 0
    eligibility_known = True
    candidates_known = True
    for event in events:
        outcomes[event["outcome"]] += event["count"]
        reasons[event["reason"]] += event["count"]
        skipped[event["skipped"]] += event["count"]
        eligibility_known &= "eligible" in event
        eligible += event["count"] if event.get("eligible") else 0
        candidates_known &= "candidate" in event
        candidates += event["count"] if event.get("candidate") else 0
    return {"total_cgu_events": count, "outcomes": dict(outcomes), "reasons": dict(reasons),
            "skipped": dict(skipped), "eligible_candidates": eligible if eligibility_known and count else None,
            "supported_dirty_candidates": candidates if candidates_known and count else None,
            "cache_writes": dict(writes),
            "fallback_fraction_of_all_cgus": outcomes["fallback"] / count if count else None}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("config", type=Path)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--cases", nargs="+", choices=CASES, default=CASES)
    parser.add_argument("--diagnostics", action="store_true")
    args = parser.parse_args()
    cfg = json.loads(args.config.read_text())
    out = Path(cfg["output"]).resolve()
    out.mkdir(parents=True, exist_ok=True)
    lock = (out / "benchmark.lock").open("w")
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    source_path = Path(cfg["edited_source"])
    original = source_path.read_bytes()
    source = original.decode()
    records = []
    metadata = {"config": cfg, "seed": cfg.get("seed", 1729),
                "source_sha256": hashlib.sha256(original).hexdigest(),
                "cargo_version": capture([cfg["cargo"], "-V"]),
                "bevy_commit": capture(["git", "-C", cfg["bevy"], "rev-parse", "HEAD"]),
                "lockfile_sha256": hashlib.sha256(Path(cfg["manifest"]).with_name("Cargo.lock").read_bytes()).hexdigest(),
                "compilers": {c["name"]: capture([c["rustc"], "-Vv"]) for c in cfg["compilers"]}}
    (out / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    trials = [(repeat, debug, opt, case, compiler) for repeat in range(args.repeats)
              for debug, opt in cfg["profiles"] for case in args.cases for compiler in cfg["compilers"]]
    random.Random(metadata["seed"]).shuffle(trials)
    try:
        for number, (repeat, debug, opt, case, compiler) in enumerate(trials):
            key = f'{compiler["name"]}-debug{debug}-opt{opt}'
            target = out / "targets" / key
            env = os.environ.copy()
            env.update(cfg.get("environment", {}))
            env.update({"RUSTC": compiler["rustc"], "CARGO_TARGET_DIR": str(target),
                        "CARGO_INCREMENTAL": "1", "CARGO_BUILD_JOBS": str(cfg["jobs"]),
                        "CARGO_PROFILE_DEV_DEBUG": str(debug), "CARGO_PROFILE_DEV_OPT_LEVEL": str(opt),
                        "CARGO_PROFILE_DEV_CODEGEN_UNITS": str(cfg.get("codegen_units", 16)),
                        "CARGO_ENCODED_RUSTFLAGS": "\x1f".join(compiler.get("flags", []))})
            if args.diagnostics:
                env.update(compiler.get("diagnostic_env", {}))
                env["CARGO_ENCODED_RUSTFLAGS"] += "\x1f" + "\x1f".join(compiler.get("diagnostic_flags", []))
            command = [cfg["cargo"], "build", "--locked", "--offline", "--manifest-path", cfg["manifest"]]
            stem = f'{number:05d}-{key}-{case}-r{repeat}'
            source_path.write_bytes(original)
            with (out / (stem + "-prime.log")).open("w") as log:
                prime = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT)
            if prime.returncode:
                raise RuntimeError(f"baseline failed: {stem}-prime.log")
            if case == "touch":
                os.utime(source_path, None)
            elif case != "noop":
                source_path.write_text(edit(source, case, cfg))
            with (out / (stem + ".log")).open("w") as log:
                start = time.perf_counter()
                build = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT)
                _, status, usage = os.wait4(build.pid, 0)
                build.returncode = os.waitstatus_to_exitcode(status)
                elapsed = time.perf_counter() - start
            row = {"compiler": compiler["name"], "debug": debug, "opt": opt,
                   "case": case, "repeat": repeat, "wall_seconds": elapsed,
                   "exit_code": build.returncode, "diagnostics_run": args.diagnostics,
                   "peak_rss_kib": usage.ru_maxrss, "log": stem + ".log",
                   "artifact_bytes": sum(p.stat().st_size for p in (target / "debug").rglob("*.rlib"))}
            records.append(row)
            if args.diagnostics:
                row["relocation"] = diagnostics(out / (stem + ".log"))
            with (out / ("diagnostics.jsonl" if args.diagnostics else "raw.jsonl")).open("a") as file:
                file.write(json.dumps(row) + "\n")
            print(json.dumps(row), flush=True)
    finally:
        source_path.write_bytes(original)
        if not args.diagnostics:
            (out / "summary.json").write_text(json.dumps(summarize(records), indent=2) + "\n")


if __name__ == "__main__":
    main()
