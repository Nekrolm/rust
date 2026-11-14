use std::fs;
use std::hash::Hasher;
use std::io::{Read, Write};

use rustc_codegen_ssa::back::source_relocation::{
    ArtifactManifest, CacheBinding, MAX_MANIFEST_SIZE,
};
use rustc_codegen_ssa::back::write::{
    BitcodeSection, CodegenContext, EmitObj, ModuleConfig, relocation_manifest_path,
    report_relocation_cache, report_source_relocation,
};
use rustc_codegen_ssa::traits::ModuleBufferMethods;
use rustc_codegen_ssa::{ModuleCodegen, ModuleKind};
use rustc_data_structures::stable_hash::StableHasher;
use rustc_hashes::Hash128;
use rustc_session::config::{Lto, Passes, SwitchWithOptPath};
use rustc_target::spec::SplitDebuginfo;

use super::lto::ModuleBuffer;
use crate::ModuleLlvm;

pub(super) struct RelocationAttempt {
    input: Option<Vec<u8>>,
    reused: bool,
}

fn checksum(bytes: &[u8]) -> [u8; 16] {
    let mut hasher = StableHasher::new();
    hasher.write(bytes);
    hasher.finish::<Hash128>().as_u128().to_le_bytes()
}

fn binding(cgcx: &CodegenContext, name: &str) -> CacheBinding {
    CacheBinding {
        compiler: cgcx.relocation_compiler.clone(),
        target: cgcx.relocation_target.clone(),
        options: cgcx.relocation_options.clone(),
        cgu: name.as_bytes().to_vec(),
    }
}

fn bitcode(module: &ModuleLlvm) -> Vec<u8> {
    module.with_module_refs(|refs| ModuleBuffer::new(refs.llmod, false).data().to_vec())
}

fn unsupported(cgcx: &CodegenContext, config: &ModuleConfig) -> Option<&'static str> {
    if !cgcx.source_relocation {
        return Some("disabled");
    }
    if cgcx.lto != Lto::No || cgcx.use_linker_plugin_lto {
        return Some("unsupported_lto");
    }
    if cgcx.split_debuginfo != SplitDebuginfo::Off {
        return Some("unsupported_split_debug");
    }
    if cgcx.save_temps
        || config.emit_bc
        || config.emit_ir
        || config.emit_asm
        || config.emit_obj != EmitObj::ObjectCode(BitcodeSection::None)
    {
        return Some("unsupported_output");
    }
    if config.pgo_gen != SwitchWithOptPath::Disabled
        || config.pgo_use.is_some()
        || config.pgo_sample_use.is_some()
        || config.instrument_coverage
        || config.debug_info_for_profiling
        || !config.sanitizer.is_empty()
        || !config.llvm_plugins.is_empty()
        || !config.passes.is_empty()
        || !config.autodiff.is_empty()
        || !config.offload.is_empty()
        || !matches!(cgcx.remark, Passes::Some(ref passes) if passes.is_empty())
        || cgcx.time_trace
    {
        return Some("unsupported_instrumentation");
    }
    None
}

pub(super) fn prepare(
    cgcx: &CodegenContext,
    module: &mut ModuleCodegen<ModuleLlvm>,
    config: &ModuleConfig,
) -> RelocationAttempt {
    if module.kind != ModuleKind::Regular {
        return RelocationAttempt { input: None, reused: false };
    }
    if cgcx.source_relocation {
        let _ = fs::remove_file(relocation_manifest_path(&cgcx.output_filenames, &module.name));
    }
    let mut attempt = RelocationAttempt { input: None, reused: false };
    let reason = if let Some(reason) = unsupported(cgcx, config) {
        reason
    } else {
        attempt.input = Some(bitcode(&module.module_llvm));
        match load(cgcx, module) {
            Ok(()) => {
                attempt.reused = true;
                "validated_debug_refresh"
            }
            Err(reason) => reason,
        }
    };
    report_source_relocation(
        cgcx.relocation_report,
        &module.name,
        if attempt.reused { "accepted" } else { "fallback" },
        reason,
        if attempt.reused { "llvm_optimization" } else { "none" },
    );
    attempt
}

fn load(cgcx: &CodegenContext, module: &mut ModuleCodegen<ModuleLlvm>) -> Result<(), &'static str> {
    let old_directory = cgcx.relocation_old_directory.as_ref().ok_or("no_prior_manifest")?;
    let file = fs::File::open(old_directory.join(format!("{}.relocation", module.name)))
        .map_err(|_| "no_prior_manifest")?;
    let limit = MAX_MANIFEST_SIZE as u64;
    if file.metadata().map_err(|_| "manifest_io_failure")?.len() > limit {
        return Err("manifest_oversize");
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).map_err(|_| "manifest_io_failure")?;
    if bytes.len() > MAX_MANIFEST_SIZE {
        return Err("manifest_oversize");
    }
    let manifest = ArtifactManifest::decode(&bytes, checksum).ok_or("malformed_manifest")?;
    if manifest.binding() != &binding(cgcx, &module.name) {
        return Err("manifest_binding_mismatch");
    }
    module.module_llvm.try_relocate(manifest.canonical_input(), manifest.artifact()).map_err(
        |status| match status {
            1 => "semantic_ir_differs",
            2 => "source_mapping_unsupported",
            3 => "debug_rewrite_unsupported",
            _ => "malformed_bitcode",
        },
    )
}

impl RelocationAttempt {
    pub(super) fn reused(&self) -> bool {
        self.reused
    }

    pub(super) fn save(self, cgcx: &CodegenContext, module: &ModuleCodegen<ModuleLlvm>) {
        let Some(input) = self.input else { return };
        let manifest =
            ArtifactManifest::new(binding(cgcx, &module.name), input, bitcode(&module.module_llvm));
        let Some(bytes) = manifest.encode(checksum) else {
            report_relocation_cache(cgcx.relocation_report, &module.name, "manifest_oversize");
            return;
        };
        let path = relocation_manifest_path(&cgcx.output_filenames, &module.name);
        let Some(parent) = path.parent() else { return };
        // Replace the directory entry, since an earlier output can be hard-linked into a cache.
        let written = tempfile::NamedTempFile::new_in(parent).and_then(|mut temporary| {
            temporary.write_all(&bytes)?;
            temporary.persist(path).map(|_| ()).map_err(|error| error.error)
        });
        report_relocation_cache(
            cgcx.relocation_report,
            &module.name,
            if written.is_ok() { "written" } else { "manifest_io_failure" },
        );
    }
}
