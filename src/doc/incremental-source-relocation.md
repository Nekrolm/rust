# Incremental compilation after moving unchanged source

This is an LLM-assisted, fork-only experiment. It is not an upstream Rust proposal
or a claim that the optimization is correct or faster. The user intends to share
findings rather than contribute this generated implementation upstream.

## Question and scope

Can rustc reuse more work when an unchanged function, struct, or enum moves within
its source file, while preserving current diagnostics, runtime locations, and
debug information? A useful motivating edit inserts a blank line above a generic
function instantiated in many codegen units (CGUs).

The source investigation below uses commit
`5f4effd579500175af844241eb179e756190f1e7` (the checkout reports version 1.102.0).
Rust 1.100.0 is a requested external baseline, not this checkout's version.
This design predates the comparative timing runs. Current measurements and their
attribution limits are recorded in the [validation report](../tools/incremental-relocation-bench/REPORT.md).

The first implementation must be opt-in and fail closed. Unsupported or ambiguous
cases compile normally. Do not implement this by globally ignoring spans, disabling
incremental verification, or declaring a dirty CGU green without proving that its
non-debug inputs are unchanged.

## What rustc already does

The observation that every moved declaration necessarily invalidates all users is
too broad for this revision. Much of the declaration-hashing idea already exists:

| Responsibility | Existing implementation |
| --- | --- |
| Attach a definition owner to incremental spans | `compiler/rustc_ast_lowering/src/lib.rs`, `SpanLowerer::lower` |
| Stable span fingerprints | `compiler/rustc_middle/src/ich.rs`, `StableHashState::stable_hash_span` |
| Reconstruct cached spans at a moved definition | `compiler/rustc_middle/src/query/on_disk_cache.rs`, `CacheEncoder::encode_span` and `CacheDecoder::decode_span` |
| Track reads of absolute locations | `compiler/rustc_span/src/span_encoding.rs`, `Span::data`; `compiler/rustc_interface/src/callbacks.rs`, `track_span_parent` |
| Absolute definition position as a dependency | `compiler/rustc_middle/src/queries.rs`, `source_span` |
| Red/green dependency checking and result fingerprints | `compiler/rustc_middle/src/dep_graph/graph.rs`, `try_mark_green`, `try_mark_previous_green`, `alloc_and_color_node` |
| Decide whether a complete CGU can be reused | `compiler/rustc_codegen_ssa/src/base.rs`, `determine_cgu_reuse` |
| Record dependencies while producing LLVM IR | `compiler/rustc_codegen_llvm/src/base.rs`, `compile_codegen_unit` |
| Generate LLVM source metadata | `compiler/rustc_codegen_llvm/src/debuginfo/{mod,metadata}.rs` |
| Save and restore codegen artifacts | `compiler/rustc_incremental/src/persist/{work_product,save,load}.rs`; `compiler/rustc_codegen_ssa/src/back/write.rs` |

Eligible parented spans hash the stable identity of their parent and byte offsets
relative to its source span. The on-disk cache uses the same representation and
adds those offsets to the *current* parent position during decoding. Other spans
use source-file identity and absolute line/column information. Hashing and encoding
must agree; changing just one is incorrect.

Reading an absolute position registers a `source_span` dependency through
`SPAN_TRACK`. A move can therefore leave an HIR or MIR result reusable while
invalidating a later location consumer. A query that runs again can also produce
the same fingerprint and stop invalidation; execution and transitive invalidation
are different measurements.

Existing regression tests make the distinction explicit:

* `tests/incremental/spans_in_type_debuginfo.rs` expects reuse after moving types.
  Type declaration line numbers are normally disabled;
  `-Zdebug-info-type-line-numbers` is a separate case to measure.
* `tests/incremental/spans_significant_w_debuginfo.rs` expects code generation
  after moving a function with debug information enabled.
* `tests/incremental/spans_significant_w_panic.rs` expects code generation when
  the moved function contains a panic location, even without debug information.

The backend validation section below records execution of these tests with the
built experimental compiler.

## Separate semantic identity from placement

A declaration's text hash is a candidate filter, not sufficient authorization to
reuse its machine code. Types, resolution, attributes, cfg, macro expansion,
target options, upstream metadata, and monomorphization arguments remain inputs.
Keep rustc's existing dependency graph as the semantic authority.

Represent a relocation candidate with the equivalent of:

```text
stable definition identity
stable source-file identity
old and new start byte and start line
unchanged byte length and exact source-content fingerprint
old and new end position
syntax/expansion provenance or an explicit unsupported marker
```

Use a stable identity such as `DefPathHash`, not a session-local `DefId`, as an
on-disk key. Retain the exact content or verify equality after fingerprint matching
where a matching fingerprint is the only proof. Do not strip arbitrary whitespace:
comments, literals, macro tokens, and indentation can affect source consumers.

An initial accepted move is within one file, with unchanged source bytes inside
the entity and unchanged column alignment. All applicable line positions receive
a checked signed delta. Use byte offsets for source spans, line offsets for debug
locations, and character columns where the consumer requires them. They are not
interchangeable, especially with UTF-8, CRLF, and normalized source text.

Mapping must be piecewise by entity or proven unchanged source region. One CGU
can contain functions from several regions and files, inlined callees, generic
instances, closures, and macro expansions. A single delta for an entire CGU or
file is not generally valid. Overlapping or ambiguous ownership requires a normal
rebuild. Moves between files and internal edits are outside the initial scope.

## Where to insert the experiment

There are two separable layers. Implement the mapping/validation layer first, then
integrate a narrowly supported artifact-reuse path. Keep each in a separate PR
stacked on this document.

### 1. Relocation data and eligibility

Add a small tested module for unchanged-region matching, checked location
translation, and a versioned cache manifest. The manifest must bind its records
to the cached artifact, target, compiler/cache version, relevant options, and CGU
identity. Missing, truncated, corrupt, stale, or mismatched manifests are cache
misses. Bounds and integer overflow are explicit rejection cases.

Collect placement data at the source/debug boundary rather than adding a second
replacement for the semantic query engine. Preserve all semantic location reads.
Never suppress tracking around an entire codegen operation or arbitrary nested
queries. If separating debug-only reads cannot be proved, leave them tracked and
use the conservative backend-only approach below.

### 2. Artifact reuse and debug refresh

The full optimization would distinguish semantic dependencies from relocatable
debug placement dependencies in `compile_codegen_unit`, and extend
`determine_cgu_reuse` with a validated relocation outcome. The existing binary
green/red decision is insufficient: a location-dirty artifact needs repair before
being treated as usable. Preserve edges for all non-debug source consumers.

A practical first prototype may keep all current query dependencies and regenerate
LLVM IR, then compare a canonical non-debug representation of the old and current
module before reusing backend work. This is a smaller optimization: it does not
avoid rustc's MIR-to-LLVM lowering or claim to solve all monomorphization work.
Its benefit, if any, comes from avoiding LLVM optimization and/or object emission.
The report must state which stages were actually skipped.

For that prototype, the comparison must include instructions, constants, globals,
linkage, attributes, target/data layout, module flags, inline assembly, and all
non-debug metadata. Strip only recognized debug metadata from a temporary clone;
do not mutate the module that will be emitted. Location-bearing runtime constants
remain part of the comparison. Unsupported metadata is a miss, not permission to
erase it. Deterministic equality is preferable to relying on a textual heuristic.

There are two possible artifact strategies, with different engineering costs:

* Refresh debug metadata in cached LLVM bitcode and emit an object normally.
  This can avoid earlier work, but still pays LLVM emission costs. Every supported
  debug node, including inline chains, declaration locations and source-file
  checksums, must be refreshed consistently. An optimized artifact can only be
  reused under the same optimization configuration and after validating that the
  translation remains applicable to its transformed debug graph.
* Rewrite debug sections in a cached native object. This can avoid more work but
  needs an actual object/DWARF reader and writer. Start with a precisely declared
  ELF/DWARF subset and reject unsupported formats, encodings, compressed sections,
  split debug information, LTO, or unrecognized location forms.

The implementation PR must select and document its supported strategy. A prototype
that only measures eligibility is useful evidence but is not a completed compiler
optimization and must not be reported as one.

### Why literal in-place patching is insufficient

DWARF line numbers are often encoded in a state-machine program with variable-size
integers and special opcodes. Adding one can change encoded length. Declaration
and inline call-site lines also occur outside `.debug_line`; source checksums can
change even if all machine instructions stay the same. CodeView/PDB and split DWARF
have their own structures. Use checked reserialization or prove a fixed-width
encoding fits; otherwise rebuild.

The incremental cache uses `link_or_copy`, which can create hard links. Never
modify a reused cache file in place: that can mutate another session or output.
Read the old artifact, create a fresh temporary output, validate the complete
rewrite, and atomically install it in the new session. On any failure discard the
temporary output and compile normally. Carry manifests forward on no-op reuse as
well as on relocation; repeated moves must not accumulate an incorrect delta.

## Cases that must remain correct

* `line!()`, `column!()`, `file!()`, `Location::caller`, `#[track_caller]`, panic and
  bounds/overflow locations, and location-sensitive procedural macros.
* Diagnostics replayed from cached queries, including suggestions and spans.
* Inline call-site locations as distinct from callee declaration locations.
* Debug stepping, breakpoints, backtraces, type and member declarations, file
  checksums, and all requested output forms (`.o`, `.bc`, `.ll`, assembly, `.dwo`).
* Const evaluation, type/layout changes, attributes, cfg, generic arguments,
  closures and anonymous-definition identity changes, macro hygiene and expansion.
* Source content changes inside an entity, multiple independently moved entities,
  deleted/added definitions, mixed-file spans, and source remapping options.
* Cache interruption, invalid manifests, concurrent sessions, and repeated moves.

Do not use `-Zincremental-ignore-spans` as the proposed solution. It is at most a
diagnostic comparison, with explicit labeling and no correctness claim.

## Observability

Reserve an opt-in machine-readable reporting option alongside the experiment.
The report must measure how frequently real code reaches the optimization and why
it falls back, including when the optimization saves little time. Reporting must
not change eligibility or reuse decisions; keep it disabled in primary timing runs.

Use a versioned schema with crate/CGU scope and stable reason codes. Count ordinary
green reuse separately from relocation attempts. Record candidate/attempt counts,
accepted relocations, backend cache hits, normal recompiles, and rejected attempts
by reason: unsupported configuration or format, missing artifact/manifest, changed
semantic input, nonuniform or ambiguous source mapping, changed entity content,
invalid manifest/artifact binding, unsupported debug metadata, rewrite failure,
and output validation failure. Distinguish the stages actually skipped. Each
attempt has exactly one terminal outcome so rates have an unambiguous denominator.

Include compiler identity and relevant mode settings. Avoid recording source text;
aggregate counts are sufficient by default. The harness should collect reports
from all compiler invocations using separate files or a concurrency-safe stream,
then publish success/fallback rates and reason counts in untimed diagnostic runs.
Missing reports, disabled instrumentation, and zero candidates are distinct from a
measured zero fallback rate. Verify report accounting with deterministic tests.

## Test and measurement plan

Before changing reuse behavior, run existing incremental span tests and capture a
small reproducer that separates clean builds, no-op builds, physical source shifts,
and real code changes. Include an ordinary and a generic function, struct, enum,
inlined caller/callee, and multiple CGUs. Physically edit files for performance
measurements; cfg-selected copies are useful for compiletest but are not a timing
substitute for the user's edit.

Test mapping boundaries, negative deltas, overflow, non-ASCII source, CRLF,
ambiguous/overlapping matches, unchanged regions in changed files, and invalid cache
records. Integration tests must compare incremental results with clean patched
builds, execute runtime-location examples, and inspect decoded debug line/inline
information. An optimization counts as working only if a test observes actual
reuse and independently checks updated output. Test disabled and unsupported
configurations for normal fallback. Use `-Zincremental-verify-ich` where supported.

Benchmark three compiler identities:

1. Official Rust 1.100.0, recording `rustc -Vv` and Cargo version.
2. Unpatched `5f4effd579500175af844241eb179e756190f1e7`, built with the same compiler
   build configuration as the patch.
3. The patched revision, both with the experimental switch disabled and enabled.

If an exact toolchain or build prerequisite cannot be obtained, report the exact
failure. Never relabel a beta, a newer stable, or a check-only build as Rust 1.100
or a working patched compiler. A newer compiler alone is a confounder.

Use a pinned Bevy commit and lockfile compatible with Rust 1.100. Start with a
headless workload (`bevy_ecs` or another representative library) to avoid graphics
system dependencies. Include a consumer that instantiates generics from an edited
dependency. Record exactly which packages, targets, features and profile were built;
a single Bevy subcrate is not the whole engine.

| Edit | Purpose |
| --- | --- |
| No change | Cargo freshness and lower bound |
| Touch identical source | Cargo rebuild trigger without a semantic change |
| Blank line near the top of a populated module | Many declarations move |
| Blank line before a generic function | Monomorphization fan-out |
| Blank line before a struct or enum | Type-location behavior |
| Blank line near the bottom | A small movement set |
| Compensated move of one item | Distinguish per-entity and whole-file effects |
| Real function-body edit | Necessary invalidation control |
| Type/layout or location-sensitive edit | Correctness and fallback control |

Use isolated target/incremental directories per compiler and option set. Prime
each baseline once, restore source outside the timed interval, re-prime the same
baseline before every sample, and time just the requested edit's rebuild. Randomize
or alternate compiler order, use at least five repeated samples for reported
timing conclusions, and report median plus spread and raw records. Keep CPU/job
counts, environment, flags, lockfile and Cargo version fixed when compatible.
Do not overlap timed builds with compiler builds or other benchmarks.

Measure debug information off, line tables, and full debug information separately;
include a representative optimized development profile as well as unoptimized
development. Record unsupported combinations instead of silently dropping them.
Collect wall time, peak memory where available, CGU reuse/miss counts, artifact size,
and profiler breakdowns for a subset of runs. Keep profiling out of primary timed
samples. Distinguish Cargo freshness, frontend query reuse, LLVM work, and linking.

## PR stack and completion criteria

1. This design and source map, targeting the fork's `main`.
2. Relocation/eligibility primitives and tests, targeting the design branch.
3. Guarded backend integration and correctness tests, targeting the primitives.
4. Reproducible benchmark harness and measured report, targeting the integration.

Implementation sessions use Sol at medium effort. A separate Sol medium session
validates the combined changes and benchmarks them. Agents may narrow the initial
supported format after investigation, but must record that choice and preserve
fallback behavior. All PRs remain on `Nekrolm/rust` and disclose generated work.

Completion means actual compiler execution demonstrates reuse with correct updated
locations and a balanced benchmark report includes regressions and unsupported
cases. If build infrastructure blocks that evidence, label the affected PRs and
report as unvalidated rather than claiming completion or a speedup.

## Backend prototype in the fork

The opt-in switch is `-Zincremental-source-relocation`. This first backend path
retains all existing dependency tracking and regenerates the complete unoptimized
LLVM module for a dirty CGU. It stores that module and its optimized bitcode in a
versioned, checksummed manifest. The LLVM helper compares canonical modules after
removing only debug information, validates its supported debug mapping, and
refreshes a temporary optimized module. Success skips LLVM optimization; MIR to
LLVM lowering, object emission, and linking still run. There is no object-section
patching. Built-compiler correctness checks pass as described below; performance
claims require separate measurements.

Build the experimental compiler with `--set rust.omit-git-hash=false` so that
`rustc -Vv` reports its complete commit hash. Missing, unknown, empty, truncated,
or malformed compiler hashes cause `unsupported_compiler_identity` fallback;
compiler versions alone do not authorize cache reuse across development builds.

The initial configuration requires `-Clto=off -Cembed-bitcode=no`, ordinary object
or linked output, and no split debug information, save-temps, profiling, coverage,
sanitizers, extra passes, plugins, autodiff, or offload. The helper further rejects
unsupported debug graphs and mappings. In particular, this is a narrow uniform
movement experiment, not an implementation of the full per-entity map above.
One source file may move uniformly while foreign and synthetic source files remain
unchanged; inline callee and call-site lines are mapped by their respective files.
Requested debug outputs and runtime location constants remain inputs; rejected
cases take the normal backend path.

For modules genuinely containing no debug information, an additional path requires
exact equality of the entire old and current pre-optimization bitcode, retaining
all metadata. It returns the verified cached optimized module unchanged. This
path reports `validated_identical_ir`; it skips LLVM optimization only. An ordinary
no-debug source move can already be green under rustc's dependency checks, in
which case ordinary reuse takes precedence and this comparison never runs.
Location-bearing constants such as `line!()` remain part of the exact comparison.

A conservative comparison limitation affects identified LLVM struct types. Parsing
cached bitcode in the current LLVM context can rename those types, causing
`semantic_ir_differs` even for otherwise identical input. This loses an
optimization opportunity and takes the normal backend path. Literal LLVM struct
types do not require those identified names; the helper does not weaken equality
to compensate for this limitation.

Manifests bind compiler/cache version, target, tracked session options, and CGU
identity. They contain exact pre-optimization and optimized bitcode plus a stable
128-bit checksum for accidental corruption detection. They are trusted compiler
cache data, not an authenticated interchange format. The 256 MiB manifest limit
is an explicit miss. Cache writes install fresh temporary files; ordinary green
reuse carries the manifest into the next session. Repeated movements compare
against the latest unoptimized module and refreshed optimized module.

For applicability measurements, `-Zincremental-relocation-report` prints JSON
lines prefixed `rustc-relocation: `. This untracked switch does not change the
session configuration hash. Schema 1 `cgu` events include a hashed CGU scope,
compiler hash, mode, candidate/eligible booleans, outcome, stable reason, and
skipped stage. A candidate is a dirty CGU in a supported configuration; eligibility
is established only on acceptance. Ordinary green reuse has a separate outcome.
Non-LLVM backends report `unsupported_backend` for dirty CGUs in the shared
scheduler. Alternative backends with independent schedulers may emit no events.

Fallback reasons include `disabled`, `unsupported_compiler_identity`,
`unsupported_lto`, `unsupported_split_debug`,
`unsupported_output`, `unsupported_instrumentation`, `no_prior_manifest`,
`malformed_manifest`, `manifest_oversize`, `manifest_io_failure`, `manifest_binding_mismatch`, `semantic_ir_differs`,
`source_mapping_unsupported`, `debug_rewrite_unsupported`, and `malformed_bitcode`.
Acceptance uses `validated_debug_refresh` or `validated_identical_ir`, with
`skipped=llvm_optimization`.
Separate `cache_write` events report `written`, `carried`, `manifest_oversize`, or
`manifest_io_failure`; exclude those lifecycle events from CGU denominators.
Reporting is opt-in and contains no source text, paths, or per-item identifiers.

The run-make regression test passed with a compiler built from production commit
`b533f1464879063bc047d2c707e2184d41058827`. It exercises actual accepted debug
refresh, positive and negative repeated shifts, clean-object and decoded-DWARF
comparisons, semantic edits, runtime `line!` constants, unsupported output
fallback, and executed `line!`, `column!`, `file!`, track-caller and panic locations.
A separate no-debug `-Copt-level=z` case changes `1 + 1` to `2`, requires
`validated_identical_ir` with `skipped=llvm_optimization`, and compares the object
with a clean build. Three fresh direct runs of the complete test also passed.
The existing incremental tests `spans_in_type_debuginfo`,
`spans_significant_w_debuginfo`, and `spans_significant_w_panic` passed with the
compiler's default options and with relocation enabled, LTO disabled, embedded
bitcode disabled, and incremental hash verification enabled. These are correctness
results, not timing results.

One initial direct test run unexpectedly reported `no_prior_manifest` for the
second runtime-`line!` build. That miss did not reproduce in the subsequent fresh
runs or the bootstrap run. Its cause remains unexplained; the observed fallback
did not reuse a stale object. The checks above establish the exercised cases,
not complete cache lifecycle coverage.

A later repeated-build validation exposed a temporary-manifest leak in `b533f146`:
per-invocation output sidecars survived after their incremental-cache copies were
saved, including ordinary green carries. Repeated builds accumulated these files
until the filesystem ran out of space. Production repair
`6dd88db2949b3173124de6f722f362678388e0d8` unlinks temporary output sidecars in
`produce_final_output_artifacts`, after work-product persistence and before
linking. This covers object-only and linked outputs, ordinary green reuse,
accepted relocation, and fallback. Incremental-cache manifests remain available;
cleanup only unlinks the temporary path and respects `-Csave-temps`.

The repaired compiler and standard library built successfully with LLVM 23.1.5.
The extended run-make test failed on `b533f146` because a temporary sidecar
remained, then passed on `6dd88db2949b3173124de6f722f362678388e0d8`. It checks
output-sidecar absence after object and executable builds, retained cache
manifests, and linked rlib initial/green/accepted/fallback builds. The three
existing span tests named above passed again with default options and in a
forced rerun with relocation and incremental hash verification enabled, LTO off,
and embedded bitcode disabled. Existing benchmark timings remain attributed to
`b533f146`; this repair has no performance claim. The separate benchmark report
will distinguish the two compiler identities.
