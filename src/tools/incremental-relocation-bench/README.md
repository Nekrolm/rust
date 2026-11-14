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

For another Cargo workspace, set `repository`, `cwd`, and `cargo_args` to pin its
repository and select packages/targets. Optimization tokens such as `"z"` are
accepted. `--cases ... function ...` and `function_marker` cover an ordinary
function without labeling it generic. `target_root` can share primed targets
between sequential edit campaigns against the same baseline workspace; each
compiler/option set still has its own target directory. Never run those campaigns
concurrently. Keep private repository configurations and results outside public
source trees.

The cases cover no change, touch, top/generic/type/bottom line insertion, one
compensated movement, and an iterator-body change. The example configuration
targets the consumer's instantiated `QueryState::iter` method. The compensated case inserts
one line before that method and removes one after it, leaving subsequent
declarations at their original lines. Debug profiles cover 0, line tables, full,
and full with optimization level 1. The body case switches `query` to `query_manual`,
changing archetype refresh behavior as an invalidation control.

Raw JSONL includes failures, wall time, Linux `wait4` maximum RSS, and the sum of
dependency rlib sizes. RSS is the maximum reported for the waited process and its
children, not concurrent process memory summed across the build. Artifact size
does not include incremental caches or all linked outputs. Summaries report
median/min/max and sample count; fewer than five successful samples cannot
support a timing conclusion. Profiling and report collection are untimed runs.

For patched runs, add separate compiler entries for the same executable with the
experiment off/on. The enabled entry uses `-Zincremental-source-relocation`.
Set `-Clto=off -Cembed-bitcode=no` identically for every compared entry; default
local ThinLTO in optimized profiles is outside the supported prototype. Measure
that default separately as an unsupported/fallback case.
Add `-Zincremental-relocation-report` to its `diagnostic_flags`, and run
`python3 bench.py config.json --repeats 1 --diagnostics`. Parsed report records
preserve outcome/reason/skipped-stage counts, total CGU event denominators,
fallback fraction, and eligible candidates when the schema exposes eligibility.
An empty report is unavailable observability, not zero fallback. The report flag
must remain absent from primary timings.

The direct-rustc correctness runner compares runtime `line!`, `column!`, `file!`,
`track_caller`, panic, bounds, and overflow locations after physical line moves
of +1, +3, and +2 lines with a clean
build. For line/full debug modes it also compares decoded source-line multisets
and declaration/inline-line multisets using `readelf`. These summaries check
location consistency; they are not a complete DWARF graph or debugger-stepping
equivalence test. Run separate output directories for each compiler/flag set.
Use `--flag=-Zincremental-source-relocation` for the experiment and capture reuse
diagnostics separately. Unsupported outputs/options require additional tests;
this fixture does not establish their correctness.
Use `--crlf` to run the same checks with CRLF source. A negative move occurs in
the +3 to +2 transition, exercising repeated-cache placement updates.
