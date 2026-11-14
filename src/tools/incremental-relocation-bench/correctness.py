#!/usr/bin/env python3
"""Compare moved incremental runtime locations and decoded DWARF with clean output."""
import argparse
import collections
import json
from pathlib import Path
import re
import subprocess


SOURCE = '''use std::panic::Location;
#[derive(Debug)] struct Item { value: u32 }
#[inline(always)] fn generic<T: Copy>(x: T) -> T { x }
#[track_caller] fn caller() -> u32 { Location::caller().line() }
#[inline(never)] fn panic_here() { panic!("expected"); }
fn main() {
    std::panic::set_hook(Box::new(|info| println!("panic:{}:{}", info.location().unwrap().line(), info.location().unwrap().column())));
    let item = Item { value: generic(7) };
    println!("{}:{}:{}:{}:{}", line!(), column!(), caller(), item.value, file!());
    let _ = std::panic::catch_unwind(panic_here);
}
'''


def run(command, log):
    result = subprocess.run(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    log.write_text(result.stdout + result.stderr)
    if result.returncode:
        raise RuntimeError(f"failed command {command!r}; see {log}")
    return result.stdout


def dwarf(binary, output):
    decoded = run(["readelf", "--debug-dump=decodedline", str(binary)], output.with_suffix(".lines"))
    rows = collections.Counter()
    for line in decoded.splitlines():
        match = re.match(r"\s*locations.rs\s+(\d+|-)\s+0x[0-9a-f]+", line)
        if match:
            rows[match[1]] += 1
    info = run(["readelf", "--debug-dump=info", str(binary)], output.with_suffix(".info"))
    locations = collections.Counter(re.findall(r"DW_AT_(?:decl_line|call_line)\s*:\s*(\d+)", info))
    return rows, locations


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--rustc", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--flag", action="append", default=[])
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    source = output / "locations.rs"
    identity = run([args.rustc, "-Vv"], output / "identity.txt")
    reports = []
    for debug in (0, 1, 2):
        flags = ["--edition=2024", "-Copt-level=1", f"-Cdebuginfo={debug}", "-Ccodegen-units=4", "-Cembed-bitcode=no", *args.flag]
        incremental = output / f"incremental-debug{debug}"
        binary = output / f"moved-debug{debug}"
        clean = output / f"clean-debug{debug}"
        source.write_text(SOURCE)
        run([args.rustc, str(source), *flags, f"-Cincremental={incremental}", "-o", str(binary)], output / f"prime-debug{debug}.log")
        source.write_text("\n" + SOURCE)
        run([args.rustc, str(source), *flags, f"-Cincremental={incremental}", "-o", str(binary)], output / f"moved-debug{debug}.log")
        run([args.rustc, str(source), *flags, "-o", str(clean)], output / f"clean-debug{debug}.log")
        actual = run([str(binary)], output / f"moved-debug{debug}.stdout")
        expected = run([str(clean)], output / f"clean-debug{debug}.stdout")
        assert actual == expected, (debug, actual, expected)
        row = {"debug": debug, "runtime_matches_clean": True}
        if debug:
            actual_debug = dwarf(binary, output / f"moved-debug{debug}")
            expected_debug = dwarf(clean, output / f"clean-debug{debug}")
            assert actual_debug[0], "no fixture DWARF lines found"
            row["decoded_line_multiset_matches_clean"] = actual_debug[0] == expected_debug[0]
            row["declaration_and_inline_line_multiset_matches_clean"] = actual_debug[1] == expected_debug[1]
            assert all(row.values()), row
        reports.append(row)
    (output / "correctness.json").write_text(json.dumps({"compiler": identity, "flags": args.flag, "cases": reports}, indent=2) + "\n")
    print(json.dumps(reports, indent=2))


if __name__ == "__main__":
    main()
