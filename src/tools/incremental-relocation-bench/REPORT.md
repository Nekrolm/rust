# Independent validation status

This is an LLM-generated, fork-only experiment report. It has **no measured
patched-compiler result and no measured speedup**. The official baseline executes;
the same-revision baseline and enabled/disabled patched compiler still require
building and running this matrix.

## Exact official baseline

* Download: https://static.rust-lang.org/dist/rust-1.100.0-x86_64-unknown-linux-gnu.tar.xz
* Published SHA256 verified: `98ee6bec240e1cfe944f17e7092412dc5ef6e008484bb668c6936d35a7d15acd`.
* `rustc 1.100.0 (0ef1bc386 2026-11-09)`; full commit
  `0ef1bc386059596dada8fb65b50425aab2ea637a`; LLVM 23.1.5.
* Fixed Cargo: `cargo 1.100.0 (3acc79c0c 2026-11-05)`.
* Bevy: https://github.com/bevyengine/bevy/tree/5f8270f2e049f90139a503d1e930070d926f9427
  (`v0.18.0`), `bevy_ecs`, default features disabled, `std` enabled.
* Workload: standalone binary consumer with ten numeric generic query
  instantiations; expected runtime output `10`. This is not the whole engine.
* Profiles measured: development, `debug=0`, `opt-level=0`, incremental on,
  16 CGUs, one Cargo job. Other profiles have correctness checks but no timings.

## Primary timings

Five successful samples per case; 40/40 builds succeeded. Source was restored
and the original baseline rebuilt before every sample outside the timer. Trial
order was shuffled with seed 1729. Compiler agents paused builds throughout the
exclusive timing window. The timing interval includes Cargo startup, compilation,
and linking. The no-op row measures Cargo freshness. These are baseline rebuild
times, with no attribution to a particular compiler stage.

| Physical edit | Median seconds | Minimum–maximum seconds | Median maximum RSS MiB |
| --- | ---: | ---: | ---: |
| body | 2.3081 | 2.1069–2.7109 | 630.7 |
| bottom | 2.5662 | 2.0466–2.7551 | 618.8 |
| compensated | 2.5359 | 2.0544–2.9469 | 629.0 |
| generic | 2.3644 | 2.2669–2.4182 | 637.0 |
| noop | 0.0494 | 0.0466–0.0504 | 39.1 |
| top | 2.3485 | 2.1513–2.4347 | 643.2 |
| touch | 1.9224 | 1.7299–3.3178 | 557.1 |
| type | 2.4984 | 2.2083–3.7151 | 641.1 |

The generic row inserts a line before `QueryState::matched_tables`, moving that
method and all subsequent declarations, including the consumer's instantiated
iteration methods. The compensated row confines movement to `matched_tables`;
the consumer does not call that method directly, so this row is a narrow
frontend/cache control, not evidence of monomorphization fan-out for that method.
The bottom row inserts a line before the final non-test `From<QueryBuilder>` impl.
The body control reverses the iterator returned by `matched_tables`.
The example configuration was subsequently strengthened to target the actually
instantiated `QueryState::iter` method, including compensated movement and a
`query` to `query_manual` body change. Those changed cases require new baseline
measurements before comparison; they are not the cases measured in this pilot.

Raw samples, compiler/configuration metadata, summaries, host details, and a
build-log hash manifest are in `results/official-1.100.0/`. Full build logs remain
in the local work directory `/workspace/work/validation-records/bevy-official-debug0`.
The initial artifact collector assumed the previous Cargo `debug/deps` layout;
**all raw `artifact_bytes: 0` entries mean unavailable**, not empty output.
The corrected recursive collector and separately recorded final rlib/executable
sizes address Cargo 1.100's `debug/build/.../out` layout. Timing and RSS collection
were unaffected. Maximum RSS comes from Linux `wait4`, not summed concurrent
process memory.

## Correctness executed

Official Rust 1.100.0, debug 0/line tables/full, optimization 1, four CGUs,
`embed-bitcode=no`: physical +1 line insertion followed by an incremental rebuild
matches a clean build's runtime `line!`, `column!`, `file!`, `track_caller`, and
panic locations. Line/full modes also match decoded source-line multisets and
`DW_AT_decl_line`/`DW_AT_call_line` multisets. These are limited location checks,
not a complete DWARF or debugger equivalence proof. Results and runtime stdout
are retained beside the timing samples. No patched result is implied.

An expanded untimed official-baseline run also passed repeated shifts of +1,
+3, and +2 lines (including backward movement), runtime bounds/overflow panic
locations, and CRLF input, in all three debug modes. The revised compensated
and instantiated-body workload edits compile and execute with output `10`.
`correctness-expanded.json` and `correctness-crlf.json` preserve these results.

The harness's three unit tests cover physical edit accounting, failed-sample
exclusion, minimum sample disclosure, report fallback denominators, and missing
report handling. The example configuration successfully constructs every edit
against the pinned source. Exact commands:

```
python3 -m unittest discover -s src/tools/incremental-relocation-bench -p test_bench.py -v
python3 src/tools/incremental-relocation-bench/bench.py /workspace/work/validation-records/official-debug0-config.json --repeats 5
python3 src/tools/incremental-relocation-bench/correctness.py --rustc /workspace/work/toolchains/1.100.0/bin/rustc --output /workspace/work/validation-records/correctness-1.100.0
```

Independent execution also passed the standalone LLVM helper binary
`/workspace/work/llvm-relocation-test` and all eight primitive tests from
`/workspace/work/source-relocation-tests --test-threads=1`. `readelf -Ws` confirms
the helper binary retains `__assert_fail` references, addressing the risk that
release-mode `NDEBUG` could erase its assertion-based checks. These component
checks do not exercise the complete patched compiler or its cache lifecycle.

## Still unvalidated

The unpatched `5f4effd579500175af844241eb179e756190f1e7` compiler and patched
compiler have no execution result from this validation session. All accepted
relocations, skipped LLVM work, fallback-frequency rates, updated cached debug
information, and comparative performance remain unmeasured. The parent session
is resolving compiler/LLVM build prerequisites; its exact infrastructure failures
belong in the final integration report. Baseline-only measurements cannot isolate
a compiler-version effect or demonstrate this experiment works.

Still required: same-revision compiler comparison, experiment disabled/enabled,
line/full/optimized development timing matrices, independent report-only CGU
counts, actual accepted-reuse observation, repeated/negative source moves,
unsupported configuration/output fallback tests, cache corruption/concurrency,
and deeper output/debug checks. Unsupported or missing combinations must stay
visible in the final report.
