#ifdef NDEBUG
#error RelocationTest requires assertions; compile with -UNDEBUG
#endif

#include "Relocation.cpp"
#include "llvm/AsmParser/Parser.h"
#include "llvm/BinaryFormat/Dwarf.h"
#include "llvm/Support/SourceMgr.h"
#include <cassert>

static std::unique_ptr<Module> fixture(LLVMContext &C, unsigned Line,
                                       unsigned BodyLine, StringRef Checksum,
                                       bool RuntimeChange = false,
                                       bool Inline = false, bool Foreign = false,
                                       bool ForeignChecksumChange = false,
                                       bool Ambiguous = false) {
  std::string IR;
  raw_string_ostream OS(IR);
  OS << "source_filename = \"source.rs\"\n"
        "define i32 @f() !dbg !4 {\nentry:\nret i32 "
     << (RuntimeChange ? 2 : 1) << ", !dbg !8\n}\n"
        "!llvm.dbg.cu = !{!0}\n"
        "!llvm.module.flags = !{!9}\n"
        "!0 = distinct !DICompileUnit(language: DW_LANG_Rust, file: !"
     << (Foreign ? 11 : 1) << ", producer: \"relocation-test\", isOptimized: false, runtimeVersion: 0, "
        "emissionKind: FullDebug)\n"
        "!1 = !DIFile(filename: \"source.rs\", directory: \"/\", "
        "checksumkind: CSK_MD5, checksum: \""
     << Checksum << "\")\n"
        "!2 = !DISubroutineType(types: !3)\n"
        "!3 = !{null}\n"
        "!4 = distinct !DISubprogram(name: \"f\", scope: !1, file: !1, line: "
     << Line << ", type: !2, scopeLine: " << Line
     << ", spFlags: DISPFlagDefinition, unit: !0)\n"
        "!8 = !DILocation(line: "
     << (Foreign ? 700 : BodyLine) << ", column: 1, scope: !"
     << (Foreign ? 12 : 4);
  if (Inline || Foreign)
    OS << ", inlinedAt: !10";
  OS << ")\n"
        "!9 = !{i32 2, !\"Debug Info Version\", i32 3}\n";
  if (Inline || Foreign)
    OS << "!10 = !DILocation(line: " << (Foreign ? BodyLine : Line)
       << ", column: 2, scope: !4)\n";
  if (Foreign) {
    OS << "!11 = !DIFile(filename: \"source.rs/@/cgu.0\", directory: \"/\")\n"
          "!12 = distinct !DISubprogram(name: \"core_inline\", scope: !13, "
          "file: !13, line: 600, type: !2, scopeLine: 600, "
          "spFlags: DISPFlagDefinition, unit: !0)\n"
          "!13 = !DIFile(filename: \""
       << (Ambiguous ? "source.rs" : "core.rs")
       << "\", directory: \"/\", checksumkind: CSK_MD5, checksum: \""
       << (ForeignChecksumChange ? "33333333333333333333333333333333"
                                 : "22222222222222222222222222222222")
       << "\")\n";
  }
  SMDiagnostic Diagnostic;
  auto M = parseAssemblyString(IR, Diagnostic, C);
  assert(M && !verifyModule(*M, &errs()));
  return M;
}

static std::unique_ptr<Module> run(Module &Old, Module &Current,
                                   Module &Optimized, uint32_t &Status) {
  auto A = serialize(Old), B = serialize(Optimized);
  return std::unique_ptr<Module>(unwrap(LLVMRustRelocationRefreshDebugInfo(
      wrap(&Current), reinterpret_cast<const uint8_t *>(A.data()), A.size(),
      reinterpret_cast<const uint8_t *>(B.data()), B.size(), &Status)));
}

int main() {
  LLVMContext C;
  const char *A = "00000000000000000000000000000000";
  const char *B = "11111111111111111111111111111111";
  auto Old = fixture(C, 10, 11, A);
  auto Current = fixture(C, 12, 13, B);
  uint32_t Status;
  auto OriginalBytes = serialize(*Old);
  auto CurrentBytes = serialize(*Current);
  auto Result = run(*Old, *Current, *Old, Status);
  assert(Result && Status == Success);
  assert(serialize(*Old) == OriginalBytes);
  assert(serialize(*Current) == CurrentBytes);
  DebugGraph RG(*Result);
  assert(RG.Files.front()->getChecksum() ==
         DebugGraph(*Current).Files.front()->getChecksum());
  assert(Result->getFunction("f")->getSubprogram()->getLine() == 12);
  assert(Result->getFunction("f")->front().back().getDebugLoc().getLine() == 13);
  assert(Old->getFunction("f")->getSubprogram()->getLine() == 10);
  auto InlineOld = fixture(C, 10, 11, A, false, true);
  auto InlineCurrent = fixture(C, 12, 13, B, false, true);
  auto InlineResult = run(*InlineOld, *InlineCurrent, *InlineOld, Status);
  assert(InlineResult && Status == Success);
  auto Loc = InlineResult->getFunction("f")->front().back().getDebugLoc();
  assert(Loc->getInlinedAt()->getLine() == 12);
  auto MultiOld = fixture(C, 10, 11, A, false, false, true);
  auto MultiCurrent = fixture(C, 12, 13, B, false, false, true);
  auto MultiResult = run(*MultiOld, *MultiCurrent, *MultiOld, Status);
  assert(MultiResult && Status == Success);
  auto MultiLoc = MultiResult->getFunction("f")->front().back().getDebugLoc();
  assert(MultiLoc.getLine() == 700);
  assert(MultiLoc->getInlinedAt()->getLine() == 13);
  assert(MultiLoc->getScope()->getSubprogram()->getLine() == 600);
  assert(serialize(*MultiResult) == serialize(*MultiCurrent));
  auto ForeignChanged = fixture(C, 12, 13, B, false, false, true, true);
  assert(!run(*MultiOld, *ForeignChanged, *MultiOld, Status));
  assert(Status == UnsupportedMapping);
  auto Ambiguous = fixture(C, 12, 13, B, false, false, true, false, true);
  assert(!run(*MultiOld, *Ambiguous, *MultiOld, Status));
  assert(Status == UnsupportedMapping);
  auto AmbiguousOld = fixture(C, 10, 11, A, false, false, true, false, true);
  assert(!run(*AmbiguousOld, *Ambiguous, *AmbiguousOld, Status));
  assert(Status == UnsupportedMapping);
  auto OptimizedChanged = fixture(C, 10, 11, A, false, false, true, true);
  assert(!run(*MultiOld, *MultiCurrent, *OptimizedChanged, Status));
  assert(Status == UnsupportedMapping);
  auto NoDebugOld = CloneModule(*Old);
  auto NoDebugCurrent = CloneModule(*Current);
  StripDebugInfo(*NoDebugOld);
  StripDebugInfo(*NoDebugCurrent);
  assert(run(*NoDebugOld, *NoDebugCurrent, *NoDebugOld, Status));
  assert(Status == Success);
  auto SemanticChange = fixture(C, 12, 13, B, true);
  assert(!run(*Old, *SemanticChange, *Old, Status));
  assert(Status == SemanticDifference);
  auto FlagsChanged = fixture(C, 12, 13, B);
  FlagsChanged->addModuleFlag(Module::Warning, "relocation-test-flag", 1);
  assert(!run(*Old, *FlagsChanged, *Old, Status));
  assert(Status == SemanticDifference);
  auto Overflow = fixture(C, 10, std::numeric_limits<unsigned>::max(), A);
  assert(!run(*Old, *Current, *Overflow, Status));
  assert(Status == UnsupportedDebug);
  auto Unknown = fixture(C, 12, 13, B);
  Unknown->getOrInsertNamedMetadata("relocation.unknown")
      ->addOperand(GenericDINode::get(C, dwarf::DW_TAG_variable, "unknown", {}));
  assert(!run(*Old, *Unknown, *Old, Status));
  assert(Status == UnsupportedDebug);
  auto Nonuniform = fixture(C, 12, 14, B);
  assert(!run(*Old, *Nonuniform, *Old, Status));
  assert(Status == UnsupportedMapping);
  auto Negative = fixture(C, 8, 9, B);
  assert(run(*Old, *Negative, *Old, Status));
  auto Unrepresentable = fixture(C, 1, 2, A);
  assert(!run(*Current, *Unrepresentable, *Old, Status));
  assert(!shift(1, -1));
  assert(!shift(std::numeric_limits<unsigned>::max(), 1));
  assert(shift(0, -100) == 0);
  auto Bytes = serialize(*Old);
  assert(!LLVMRustRelocationRefreshDebugInfo(
      wrap(Current.get()), reinterpret_cast<const uint8_t *>(Bytes.data()), 3,
      reinterpret_cast<const uint8_t *>(Bytes.data()), Bytes.size(), &Status));
  assert(Status == MalformedModule);
  assert(serialize(*Old) == OriginalBytes);
  assert(serialize(*Current) == CurrentBytes);
}
