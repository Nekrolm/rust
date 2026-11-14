//@ only-x86_64-unknown-linux-gnu
//@ ignore-cross-compile
//@ ignore-backends: gcc

use run_make_support::{cmd, rfs, run, rustc};

const SOURCE: &str = "#![no_std]\n#[unsafe(no_mangle)]\n#[inline(never)]\npub extern \"C\" fn answer(x: u64) -> u64 { x.wrapping_mul(3) }\n";

fn build(incremental: &str, extra: &[&str]) -> String {
    let report = rustc()
        .input("sample.rs")
        .crate_type("lib")
        .emit("obj")
        .codegen_units(1)
        .opt_level("2")
        .debuginfo("2")
        .arg("-Clto=off")
        .arg("-Cembed-bitcode=no")
        .arg("-Zincremental-source-relocation")
        .arg("-Zincremental-relocation-report")
        .arg("-Zincremental-verify-ich")
        .incremental(incremental)
        .args(extra)
        .run()
        .stderr_utf8();
    assert_no_output_manifests();
    report
}

fn assert_no_output_manifests() {
    for entry in rfs::read_dir(".") {
        let path = entry.unwrap().path();
        assert!(
            !path.extension().is_some_and(|extension| extension == "relocation"),
            "temporary relocation manifest remains: {}",
            path.display(),
        );
    }
}

fn has_cached_manifest(directory: &std::path::Path) -> bool {
    rfs::read_dir(directory).any(|entry| {
        let path = entry.unwrap().path();
        if path.is_dir() {
            has_cached_manifest(&path)
        } else {
            path.extension().is_some_and(|extension| extension == "relocation")
        }
    })
}

fn debug_info() -> String {
    cmd("readelf").arg("--debug-dump=info,rawline").arg("sample.o").run().stdout_utf8()
}

fn assert_reason(report: &str, reason: &str) {
    assert!(report.contains(&format!("\"reason\":\"{reason}\"")), "{report}");
}

fn main() {
    rfs::write("sample.rs", SOURCE);
    assert_reason(&build("incremental", &[]), "no_prior_manifest");
    assert!(has_cached_manifest(std::path::Path::new("incremental")));
    assert_reason(&build("incremental", &[]), "green_post_lto");
    assert!(build("incremental", &["-Zincremental-relocation-report=no"]).is_empty());
    assert_reason(&build("incremental", &[]), "green_post_lto");

    for shift in [1, 3, 2] {
        rfs::write("sample.rs", format!("{}{SOURCE}", "\n".repeat(shift)));
        let report = build("incremental", &[]);
        assert_reason(&report, "validated_debug_refresh");
        assert!(report.contains("\"skipped\":\"llvm_optimization\""));
        let incremental_debug = debug_info();
        let incremental_object = rfs::read("sample.o");
        build(&format!("clean-{shift}"), &[]);
        assert_eq!(incremental_debug, debug_info());
        assert_eq!(incremental_object, rfs::read("sample.o"));
    }

    rfs::write("sample.rs", SOURCE.replace("wrapping_mul(3)", "wrapping_mul(4)"));
    assert_reason(&build("incremental", &[]), "semantic_ir_differs");

    let runtime_line = SOURCE.replace("x.wrapping_mul(3)", "line!() as u64");
    rfs::write("sample.rs", &runtime_line);
    build("runtime-line", &[]);
    rfs::write("sample.rs", format!("\n{runtime_line}"));
    assert_reason(&build("runtime-line", &[]), "semantic_ir_differs");
    let incremental_object = rfs::read("sample.o");
    build("runtime-line-clean", &[]);
    assert_eq!(incremental_object, rfs::read("sample.o"));

    rfs::write("sample.rs", SOURCE);
    build("no-debug", &["-Cdebuginfo=0"]);
    rfs::write("sample.rs", format!("\n{SOURCE}"));
    let no_debug = build("no-debug", &["-Cdebuginfo=0"]);
    assert!(
        no_debug.contains("\"reason\":\"validated_identical_ir\"")
            || no_debug.contains("\"reason\":\"green_post_lto\""),
        "{no_debug}",
    );
    let incremental_object = rfs::read("sample.o");
    build("no-debug-clean", &["-Cdebuginfo=0"]);
    assert_eq!(incremental_object, rfs::read("sample.o"));

    let identical_ir =
        SOURCE.replace("answer(x: u64)", "answer()").replace("x.wrapping_mul(3)", "1 + 1");
    let no_debug_options = ["-Cdebuginfo=0", "-Copt-level=z"];
    rfs::write("sample.rs", &identical_ir);
    build("identical-ir", &no_debug_options);
    rfs::write("sample.rs", identical_ir.replace("1 + 1", "2"));
    let report = build("identical-ir", &no_debug_options);
    assert_reason(&report, "validated_identical_ir");
    assert!(report.contains("\"skipped\":\"llvm_optimization\""));
    let incremental_object = rfs::read("sample.o");
    build("identical-ir-clean", &no_debug_options);
    assert_eq!(incremental_object, rfs::read("sample.o"));

    rfs::write("sample.rs", &runtime_line);
    build("no-debug-line", &["-Cdebuginfo=0"]);
    rfs::write("sample.rs", format!("\n{runtime_line}"));
    assert_reason(&build("no-debug-line", &["-Cdebuginfo=0"]), "semantic_ir_differs");
    let incremental_object = rfs::read("sample.o");
    build("no-debug-line-clean", &["-Cdebuginfo=0"]);
    assert_eq!(incremental_object, rfs::read("sample.o"));

    assert_reason(&build("unsupported", &["--emit=obj,llvm-ir"]), "unsupported_output");
    assert_reason(&build("disabled", &["-Zincremental-source-relocation=no"]), "disabled");

    let second = "#[unsafe(no_mangle)]\n#[inline(never)]\npub extern \"C\" fn second(x: u64) -> u64 { x.wrapping_add(7) }\n";
    rfs::write("sample.rs", format!("{SOURCE}{second}"));
    build("nonuniform", &[]);
    rfs::write("sample.rs", format!("{SOURCE}\n{second}"));
    assert_reason(&build("nonuniform", &[]), "source_mapping_unsupported");

    rfs::write("sample.rs", SOURCE);
    build("corrupt", &[]);
    alter_manifests(std::path::Path::new("corrupt"), false);
    rfs::write("sample.rs", format!("\n{SOURCE}"));
    assert_reason(&build("corrupt", &[]), "malformed_manifest");

    rfs::write("sample.rs", SOURCE);
    build("oversize", &[]);
    alter_manifests(std::path::Path::new("oversize"), true);
    rfs::write("sample.rs", format!("\n{SOURCE}"));
    assert_reason(&build("oversize", &[]), "manifest_oversize");

    rfs::write("sample.rs", SOURCE);
    assert_reason(&build("linked", &["--emit=link"]), "no_prior_manifest");
    assert!(has_cached_manifest(std::path::Path::new("linked")));
    assert_reason(&build("linked", &["--emit=link"]), "green_post_lto");
    rfs::write("sample.rs", format!("\n{SOURCE}"));
    assert_reason(&build("linked", &["--emit=link"]), "validated_debug_refresh");
    rfs::write("sample.rs", SOURCE.replace("wrapping_mul(3)", "wrapping_mul(4)"));
    assert_reason(&build("linked", &["--emit=link"]), "semantic_ir_differs");

    let executable = "#[track_caller]\nfn caller() -> u32 { std::panic::Location::caller().line() }\nfn main() { println!(\"{}:{}:{}:{}\", line!(), column!(), file!(), caller()); std::panic::set_hook(Box::new(|info| println!(\"panic:{}\", info.location().unwrap().line()))); let _ = std::panic::catch_unwind(|| panic!(\"location\")); }\n";
    rfs::write("sample.rs", executable);
    build_executable("runtime");
    let before = run("sample").stdout_utf8();
    rfs::write("sample.rs", format!("\n{executable}"));
    build_executable("runtime");
    let after = run("sample").stdout_utf8();
    assert_ne!(before, after);
    build_executable("runtime-clean");
    assert_eq!(after, run("sample").stdout_utf8());
}

fn build_executable(incremental: &str) -> String {
    let report = rustc()
        .input("sample.rs")
        .codegen_units(1)
        .opt_level("2")
        .debuginfo("2")
        .arg("-Clto=off")
        .arg("-Cembed-bitcode=no")
        .arg("-Zincremental-source-relocation")
        .arg("-Zincremental-relocation-report")
        .arg("-Zincremental-verify-ich")
        .incremental(incremental)
        .run()
        .stderr_utf8();
    assert_no_output_manifests();
    report
}

fn alter_manifests(directory: &std::path::Path, oversize: bool) {
    for entry in rfs::read_dir(directory) {
        let path = entry.unwrap().path();
        if path.is_dir() {
            alter_manifests(&path, oversize);
        } else if path.extension().is_some_and(|extension| extension == "relocation") {
            rfs::remove_file(&path);
            if oversize {
                std::fs::File::create(path).unwrap().set_len(256 * 1024 * 1024 + 1).unwrap();
            } else {
                rfs::write(path, b"invalid manifest");
            }
        }
    }
}
