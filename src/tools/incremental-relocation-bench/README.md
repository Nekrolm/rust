# Fork-only relocation validation

These LLM-generated scripts support the personal-fork experiment; they do not
establish that an unbuilt compiler works. They benchmark a **headless `bevy_ecs`
consumer**, not the whole Bevy engine. The consumer instantiates query paths for
ten numeric component types. It uses Bevy v0.18.0 at
`5f8270f2e049f90139a503d1e930070d926f9427`, whose ECS crate declares Rust 1.89.0
as its minimum version. `consumer/Cargo.lock` pins the resolved dependencies.

Clone that Bevy revision next to the consumer directory, or update its path
dependency. Fetch dependencies using the fixed Cargo binary and a `CARGO_HOME`
under the work directory. Adjust `example-config.json` to the actual toolchain,
consumer, Bevy, and output paths. Compiler identities come from `rustc -Vv`;
they are never inferred from labels in the configuration.

```
python3 -m unittest -v test_bench
python3 bench.py example-config.json --repeats 5
python3 correctness.py --rustc /absolute/path/to/rustc --output /work/results
```

The benchmark uses physically edited `bevy_ecs/src/query/state.rs`. Each sample
restores the source and primes that exact baseline outside the measured interval.
It then performs one edit and times `cargo build --locked --offline`. Independent
target directories distinguish each compiler/debug/optimization configuration.
Trials are shuffled with a recorded fixed seed. Source restoration also happens
on failure. Run only in an exclusive window without compiler builds or other
benchmarks. Run one harness instance per source checkout; the output lock does
not coordinate separate output directories.

The cases cover no change, touch, top/generic/type/bottom line insertion, one
compensated movement, and an iterator-body change. The compensated case inserts
one line before the generic method and removes one after it, leaving subsequent
declarations at their original lines. Debug profiles cover 0, line tables, full,
and full with optimization level 1. The body case reverses table iteration; this
is an invalidation control rather than a behavior-preserving edit.

Raw JSONL includes failures, wall time, Linux `wait4` maximum RSS, and the sum of
dependency rlib sizes. RSS is the maximum reported for the waited process and its
children, not concurrent process memory summed across the build. Artifact size
does not include incremental caches or all linked outputs. Summaries report
median/min/max and sample count; fewer than five successful samples cannot
support a timing conclusion. Profiling and report collection are untimed runs.

For patched runs, add separate compiler entries for the same executable with the
experiment off/on. The enabled entry uses `-Zincremental-source-relocation`.
Add `-Zincremental-relocation-report` to its `diagnostic_flags`, and run
`python3 bench.py config.json --repeats 1 --diagnostics`. Parsed report records
preserve outcome/reason/skipped-stage counts, total CGU event denominators,
fallback fraction, and eligible candidates when the schema exposes eligibility.
An empty report is unavailable observability, not zero fallback. The report flag
must remain absent from primary timings.

The direct-rustc correctness runner compares runtime `line!`, `column!`, `file!`,
`track_caller`, and panic locations after a physical line insertion with a clean
build. For line/full debug modes it also compares decoded source-line multisets
and declaration/inline-line multisets using `readelf`. These summaries check
location consistency; they are not a complete DWARF graph or debugger-stepping
equivalence test. Run separate output directories for each compiler/flag set.
Use `--flag=-Zincremental-source-relocation` for the experiment and capture reuse
diagnostics separately. Unsupported outputs/options require additional tests;
this fixture does not establish their correctness.
