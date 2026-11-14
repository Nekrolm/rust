# Integrated relocation validation

This is an LLM-assisted, personal-fork experiment. The b533 prototype demonstrably
skips LLVM optimization for some dirty CGUs. Ordinary owner/span-based reuse
already exists. The controlled full-debug Bevy comparison is slower with the
experiment on: top +15.5%, body +8.1%. Game configurations have lower line-edit
medians and higher body-edit medians, but also change Cargo crate identity/CGU
partitioning; those differences do not exclusively isolate backend reuse.
In the no-debug/optimization-z Bevy control, the top-edit median is 9.5% lower;
observed ranges overlap. The body median differs by only -1.7%, with no accepted
reuse in its diagnostic, and is not a verified benefit.

Validation also found a serious temporary-file retention bug: repeated builds
filled the disk after the primary timing matrix completed. The repaired compiler
and its independent controls are reported separately below.

The prototype skips LLVM optimization only: it still lowers IR, checks the proof,
generates objects and links. It requires -Clto=off and -Cembed-bitcode=no. Debug
refresh supports a proven uniform shift in one source file per CGU, with foreign
and synthetic files unchanged, a narrower scope than the complete per-entity
relocation design.

## Toolchains and measurement conditions

| Role | Rustc identity | Build provenance |
| --- | --- | --- |
| Official | 1.100.0, 0ef1bc386059596dada8fb65b50425aab2ea637a | Release distribution |
| Unpatched local | 1.102.0-dev, 0fd799ac1b46c54286261630672e1150f04d3649 | Matched optimized local build |
| Game prototype | 1.102.0-dev, b533f1464879063bc047d2c707e2184d41058827 | Matched optimized local build |
| Cleanup repair | 1.102.0-dev, 6dd88db2949b3173124de6f722f362678388e0d8 | Matched local build with cleanup repair |

All use fixed Cargo 1.100.0 (3acc79c0c). Rustc reports LLVM 23.1.5.
The local builds use the same shared LLVM binary and build configuration.
The unpatched 0fd799 revision differs from source base 5f4eff only by the design
document; its production compiler code is unpatched.
The official distribution differs in compiler version and build provenance;
official-versus-local gaps cannot be attributed solely to compiler version or
this experiment. No specific official PGO configuration is assumed.

Host: Linux 6.18.57, glibc 2.41, AMD EPYC 9V74; affinity exposes five logical CPUs
but the cgroup quota provides four CPUs. Game builds use four Cargo jobs.

Each measured sample restores and primes the exact source baseline outside the
timer, then runs one physical edit and an offline, locked Cargo build. Compiler
and profile target directories are separate. Trial order is shuffled with seed
1729. Timings include Cargo startup, compilation and linking. No-op measures Cargo
freshness. Five successful samples per cell are reported as median [minimum,
maximum], in seconds; these observed ranges are not confidence intervals.
Heavy build jobs paused throughout the primary timing windows.

Edit legend: noop measures Cargo freshness; touch changes only the source timestamp
with identical contents; top inserts a blank line before the file; type inserts
a line before the selected type and moves all following declarations; body is a
real body edit. Bottom moves the suffix after a configured late anchor.
Compensated moves one selected item and restores later declaration lines. The
type case is therefore broader than an isolated type movement.

## Primary workload: seabattles-rogue

This is the full native executable from a snapshot before the types-forward
layout change, with two independently edited source files. The native dependency
environment is identical across compilers. Source, lockfile, configuration,
commands and logs remain private; aggregate results are published with the user's
authorization.

Development profile: optimization z, debug 0, 64 CGUs; dependencies use optimization
z and one CGU. All compared configurations use incremental compilation,
-Clto=off and -Cembed-bitcode=no. The prototype additionally uses
-Zincremental-source-relocation. All 150 measured Cargo invocations succeeded.

Changing Rustc identity or Cargo-visible rustflags can change Cargo's -Cmetadata,
the crate disambiguator and CGU partitioning. These are real Cargo configuration
comparisons, with that confound explicitly retained. The supplemental same-binary
controls keep Cargo-visible identity constant.

### sb-game (UI source)

| Edit | Official | Local unpatched | Prototype | Prototype vs local median |
| --- | ---: | ---: | ---: | ---: |
| noop | 0.256 [0.251, 0.838] | 0.248 [0.227, 0.299] | 0.251 [0.241, 0.255] | +1.2% |
| touch | 6.434 [5.816, 7.832] | 12.589 [12.017, 13.165] | 12.174 [11.001, 12.331] | -3.3% |
| top | 20.946 [18.923, 21.200] | 29.651 [28.952, 31.704] | 26.945 [25.234, 28.376] | -9.1% |
| type | 20.365 [19.442, 21.069] | 31.274 [29.728, 32.250] | 27.144 [26.209, 28.704] | -13.2% |
| body | 7.746 [6.942, 7.895] | 13.153 [12.170, 14.086] | 14.516 [13.177, 15.692] | +10.4% |

### sb-scene (scene source)

| Edit | Official | Local unpatched | Prototype | Prototype vs local median |
| --- | ---: | ---: | ---: | ---: |
| noop | 0.238 [0.229, 0.247] | 0.267 [0.229, 0.644] | 0.244 [0.235, 0.265] | -8.5% |
| touch | 7.584 [6.827, 7.737] | 13.521 [12.141, 14.866] | 13.431 [12.570, 16.721] | -0.7% |
| top | 18.463 [17.815, 19.549] | 23.059 [21.684, 24.771] | 22.389 [21.497, 23.743] | -2.9% |
| type | 16.021 [15.392, 17.457] | 20.938 [17.421, 22.194] | 19.684 [18.525, 21.539] | -6.0% |
| body | 9.701 [8.635, 11.127] | 16.035 [14.953, 16.648] | 16.378 [15.458, 18.501] | +2.1% |

The prototype configuration's line-edit medians are 9.1–13.2% lower for the UI
file and 2.9–6.0% lower for the scene file. Body-edit medians are 10.4% and 2.1%
higher respectively. The official distribution remains faster for the non-noop
cases in this workload. These configuration differences are not universal or
exclusively causal speedup claims.

Full cell summaries, including RSS and rlib-size median/min/max, are in
[aggregate.json](results/seabattles-rogue/aggregate.json).
Linux wait4 maximum RSS is the maximum for the waited process and its children,
not their summed concurrent memory. Rlib size excludes other linked outputs,
incremental caches and temporary artifacts.

## Actual backend reuse evidence

One separate untimed diagnostic sample per case and file was collected with the
b533 compiler. Cargo replays cached rustc stderr even on no-op builds. The capture
wrapper streams and stores each actual rustc invocation's stderr; only those
capture files contribute CGU events. Cargo's combined stderr is ignored.
The known untracked report flag is injected through the wrapper so Cargo-visible
rustflags and artifact identity stay unchanged. Missing reports and no compiler
invocation are distinct from measured zero fallback.

Counts below are CGU events/attempts across all rebuilt crates, not unique source
entities. Cache-write lifecycle events are excluded. Supported dirty candidates
form the fallback denominator; ordinary green reuse is reported separately.

### sb-game (UI source) diagnostics

| Edit | Ordinary green | Accepted LLVM optimization reuse | Supported dirty candidates | Candidate fallback | Proof/fallback reasons |
| --- | ---: | ---: | ---: | ---: | --- |
| noop | 0 | 0 | n/a | n/a | no fresh rustc invocation |
| touch | 67 | 0 | 0 | n/a | none |
| top | 55 | 6 | 12 | 6/12 (50.0%) | debug_rewrite_unsupported: 1, semantic_ir_differs: 5 |
| type | 56 | 6 | 11 | 5/11 (45.5%) | debug_rewrite_unsupported: 1, semantic_ir_differs: 4 |
| body | 66 | 0 | 1 | 1/1 (100.0%) | semantic_ir_differs: 1 |
| bottom | 66 | 0 | 1 | 1/1 (100.0%) | semantic_ir_differs: 1 |
| compensated | 67 | 0 | 0 | n/a | none |

### sb-scene (scene source) diagnostics

| Edit | Ordinary green | Accepted LLVM optimization reuse | Supported dirty candidates | Candidate fallback | Proof/fallback reasons |
| --- | ---: | ---: | ---: | ---: | --- |
| noop | 0 | 0 | n/a | n/a | no fresh rustc invocation |
| touch | 195 | 0 | 0 | n/a | none |
| top | 179 | 13 | 16 | 3/16 (18.8%) | debug_rewrite_unsupported: 1, semantic_ir_differs: 2 |
| type | 189 | 3 | 6 | 3/6 (50.0%) | debug_rewrite_unsupported: 1, semantic_ir_differs: 2 |
| body | 194 | 0 | 1 | 1/1 (100.0%) | semantic_ir_differs: 1 |
| bottom | 195 | 0 | 0 | n/a | none |
| compensated | 195 | 0 | 0 | n/a | none |

Every accepted game event reported validated_identical_ir and skipped only LLVM
optimization. Lowering, proof work, object generation and linking still run.
semantic_ir_differs means the IR equality proof failed; identified LLVM type
renaming can cause conservative misses, so it does not prove a semantic source
change. Runtime source-location constants can also legitimately change.

The selected compensated item in each file produced entirely ordinary green
reuse, with no new optimization attempt. This demonstrates existing incremental
reuse for those selected items, not for every isolated move. Bottom and
compensated edits have diagnostic evidence only; no repeated timing conclusion
is drawn for them.

## Storage failure and repair

After normalized baseline rebuilds, allocated incremental-tree sizes were
819,372,032 bytes (official), 765,755,392 (local unpatched), and 1,161,879,552
(prototype). The prototype's incremental tree contained 441 relocation paths
summing 397,657,709 logical bytes. These figures alone understate total storage.

The prototype omitted cleanup of per-invocation temporary relocation sidecars in
the build output directories. At disk exhaustion, its full target occupied
36,543,651,840 allocated bytes. There were 15,317 temporary sidecar paths totaling
34,253,378,201 logical bytes. This accumulated across priming, primary samples and
diagnostics; it is not a normalized per-build or exclusively incremental-cache
overhead estimate.

Allocated tree sizes come from du; logical path sums can count hard-linked data
more than once and do not establish unique extra physical disk usage.
The temporary-file inventory was preserved privately before removing only known
unreferenced build scratch. Sources, incremental caches and measurement records
were retained.

All 150 primary samples completed before ENOSPC. Three later scene diagnostic
baseline primes failed; that entire diagnostic set was preserved and superseded
by a successful seven-case retry after cleanup. Primary samples were not removed
or rerun. The game timings describe the original retention-bug version; they must
not be presented as measured performance of the cleanup repair.

Repair 6dd88db2949b3173124de6f722f362678388e0d8 adds temporary-sidecar cleanup.
An isolated full native baseline plus repeated top/restore/green builds in both
files passed nine successful builds: zero output sidecars after every build and
441 incremental manifests retained. The forced identical-content touch generated
195 fresh ordinary-green CGU events and 195 carried manifests. Repaired top
smokes also retained accepted reuse: UI 6/12 dirty candidates, scene 13/16.
This validates lifecycle behavior, not repaired full-game performance. After
these nine builds the repaired full target occupied 2,745,049,088 allocated bytes,
including a 1,161,904,128-byte incremental tree; output sidecar count remained zero.

[Native cleanup evidence](results/patched-6dd88db2/cleanup-validation.json) records
per-stage flags and counters. The two top smokes additionally used untracked
report/time-passes instrumentation. For example, the repaired UI library reports
codegen_to_LLVM_IR 6.958 s and LLVM_passes 9.700 s; its native entrypoint reports
link_binary 7.328 s. These are overlapping, untimed, single-run phase observations,
not additive wall-time components or a decomposition of the b533 measurements.
[Selected phase observations](results/patched-6dd88db2/phase-observations.json)
retain numeric details without private command/source paths.

## Supplemental same-identity controls

The public workload is the headless Bevy v0.18.0 ECS consumer with ten numeric
generic query instantiations, using source revision
5f8270f2e049f90139a503d1e930070d926f9427. It is a narrow consumer, not a full
engine/game benchmark. Both profiles use one Cargo job and 16 CGUs.

### Debug 2 / optimization 1

All 60 measured invocations succeeded, five per cell. Seconds are median [min, max].

| Edit | Official | Local unpatched | Repaired off | Repaired on | On vs off median |
| --- | ---: | ---: | ---: | ---: | ---: |
| noop | 0.053 [0.047, 0.340] | 0.052 [0.047, 0.056] | 0.054 [0.054, 0.065] | 0.053 [0.052, 0.242] | -1.9% |
| top | 4.325 [4.111, 4.369] | 5.240 [5.074, 5.735] | 5.399 [5.064, 5.766] | 6.237 [5.451, 6.600] | +15.5% |
| body | 3.545 [3.427, 3.930] | 4.419 [4.256, 4.772] | 4.439 [4.184, 4.863] | 4.800 [4.663, 5.431] | +8.1% |

The matched on/off control shows slowdowns for top and body. The no-op difference
measures freshness/startup variation rather than backend optimization.

### Debug 0 / optimization z

All 30 measured invocations succeeded, five per cell. This profile matches only
the game's debug/optimization settings, retaining Bevy jobs/CGUs.

| Edit | Repaired off | Repaired on | On vs off median |
| --- | ---: | ---: | ---: |
| noop | 0.055 [0.049, 0.067] | 0.054 [0.050, 0.062] | -1.2% |
| top | 4.325 [4.060, 4.373] | 3.913 [3.472, 4.318] | -9.5% |
| body | 4.081 [3.572, 4.550] | 4.012 [3.849, 4.156] | -1.7% |

The top-edit median is lower with the experiment on, with overlapping observed
ranges. The small body difference also overlaps and its diagnostic accepted no
reuse; it should not be promoted as a confirmed optimization win. Five samples
do not establish statistical significance or general performance.


The repaired off/on pair uses the same Rustc path, Cargo-visible flags,
environment and streaming capture wrapper. The tracked experiment flag is
injected only in on mode, with strictly separate target/incremental directories.
Actual command captures verify matching -Cmetadata/extra-filename identities
for all 59 compiled unit identities in each profile.
Wrapper startup and capture overhead are included symmetrically. Report and
time-passes flags remain absent from these timing samples.

Public raw timing/RSS rows, summaries, compiler identities and normalized identity
audits are available for [debug 2 / opt 1](results/bevy-controlled/bevy-comparison-debug2opt1.json)
and [debug 0 / opt z](results/bevy-controlled/bevy-controlled-debug0optz.json).
All six retained consumer executables produced the expected output 10;
[runtime checks](results/bevy-controlled/runtime-checks.json) record the results.

### Bevy fresh-invocation diagnostics

One untimed sample per case/profile, using the repaired on-mode compiler:

| Profile | Edit | Ordinary green | Accepted | Dirty candidates | Fallback | Reason |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| debug 2 / opt 1 | noop | 0 | 0 | n/a | n/a | no fresh invocation |
| debug 2 / opt 1 | top | 28 | 0 | 4 | 4/4 | debug_rewrite_unsupported: 4 |
| debug 2 / opt 1 | body | 30 | 0 | 2 | 2/2 | debug_rewrite_unsupported: 2 |
| debug 0 / opt z | noop | 0 | 0 | n/a | n/a | no fresh invocation |
| debug 0 / opt z | top | 30 | 1 | 2 | 1/2 | semantic_ir_differs: 1 |
| debug 0 / opt z | body | 30 | 0 | 2 | 2/2 | semantic_ir_differs: 2 |

The full-debug case accepted no reuse: all four top candidates and both body
candidates fell back as debug_rewrite_unsupported. The no-debug top case accepted
one validated_identical_ir candidate and rejected one equality proof; its body
case rejected both proofs. These are CGU-event observations from one diagnostic
sample each, not unique functions or repeated fallback-frequency estimates.

## Correctness and scope

The built b533 compiler passed the integrated run-make regression and the three
existing incremental source-span cases, including forced experiment/verification
runs. Actual no-debug exact-IR reuse and full-debug refresh fixtures produced
native objects byte-identical to independent clean current-source builds.

Independent LF and CRLF fixtures passed 18 runtime/location cases in debug 0/1/2,
with repeated +1, +3 and +2 shifts, incremental verification, LTO off and
optimization 1. Runtime line/column/file, track_caller and panic/bounds/overflow
locations matched clean builds. Debug modes also matched decoded-line and
declaration/inline-line multisets. These are limited DWARF checks, not complete
debugger-stepping or DWARF-graph equivalence. Their dirty location-dependent CGUs
fell back rather than being counted as accepted reuse.

[LF results](results/patched-b533f146/correctness-lf.json) and
[CRLF results](results/patched-b533f146/correctness-crlf.json) retain the generic
fixture evidence. The repaired compiler independently passed the same 18 LF/CRLF cases:
[LF](results/patched-6dd88db2/correctness-lf.json) and
[CRLF](results/patched-6dd88db2/correctness-crlf.json).
All three physically edited source files match their recorded baseline hashes
after the campaigns, and both workload source worktrees are clean. Separate
consumer/native-dependency/toolchain/target/configuration/log directories are
intentional benchmark setup, outside those source worktrees.
The harness has seven passing regression tests, including
Cargo replay filtering, streaming stderr, control identity and cache separation.

This is a narrower matrix than the complete design: two game files in one native
profile, selected Bevy controls, and small correctness fixtures. It does not
establish universal correctness or performance, full debugger equivalence, or
every unsupported configuration/concurrency case. Repaired full-game performance
has not been remeasured.

The earlier official-only 40-sample debug-0/optimization-0 Bevy pilot remains in
[results/official-1.100.0](results/official-1.100.0/). It used older edit anchors and
is not pooled with the current comparisons. Its historical artifact_bytes: 0
entries mean unavailable collection, not empty output.
