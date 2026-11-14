#include "LLVMWrapper.h"
#include "llvm/ADT/SmallPtrSet.h"
#include "llvm/Bitcode/BitcodeReader.h"
#include "llvm/Bitcode/BitcodeWriter.h"
#include "llvm/IR/Constants.h"
#include "llvm/IR/DebugInfo.h"
#include "llvm/IR/DebugInfoMetadata.h"
#include "llvm/IR/DebugProgramInstruction.h"
#include "llvm/IR/InstIterator.h"
#include "llvm/IR/IntrinsicInst.h"
#include "llvm/IR/Verifier.h"
#include "llvm/Support/MemoryBuffer.h"
#include "llvm/Support/raw_ostream.h"
#include "llvm/Transforms/Utils/Cloning.h"
#include <limits>

using namespace llvm;

namespace {
enum RelocationStatus : uint32_t {
  Success = 0,
  SemanticDifference = 1,
  UnsupportedMapping = 2,
  UnsupportedDebug = 3,
  MalformedModule = 4,
};

static SmallVector<char, 0> serialize(Module &M) {
  SmallVector<char, 0> Bytes;
  raw_svector_ostream OS(Bytes);
  WriteBitcodeToFile(M, OS);
  return Bytes;
}

static bool sameSemantics(Module &A, Module &B) {
  auto AC = CloneModule(A);
  auto BC = CloneModule(B);
  auto Strip = [](Module &M) {
    SmallVector<MDNode *, 4> Coverage;
    auto *CoverageNode = M.getNamedMetadata("llvm.gcov");
    if (CoverageNode)
      Coverage.append(CoverageNode->op_begin(), CoverageNode->op_end());
    StripDebugInfo(M);
    if (CoverageNode) {
      auto *N = M.getOrInsertNamedMetadata("llvm.gcov");
      for (auto *O : Coverage)
        N->addOperand(O);
    }
  };
  Strip(*AC);
  Strip(*BC);
  return serialize(*AC) == serialize(*BC);
}

struct DebugGraph {
  SmallPtrSet<Metadata *, 32> Seen;
  SmallVector<MDNode *, 64> Nodes;
  SmallVector<unsigned, 64> Lines;
  SmallVector<DIFile *, 4> Files;
  bool Supported = true;

  void visit(Metadata *MD) {
    if (!MD || !Seen.insert(MD).second)
      return;
    if (isa<ValueAsMetadata>(MD)) {
      if (!isa<ConstantAsMetadata>(MD) ||
          !isa<ConstantInt>(cast<ConstantAsMetadata>(MD)->getValue()))
        Supported = false;
      return;
    }
    auto *N = dyn_cast<MDNode>(MD);
    if (!N)
      return;
    Nodes.push_back(N);
    if (auto *L = dyn_cast<DILocation>(N))
      Lines.push_back(L->getLine());
    else if (auto *S = dyn_cast<DISubprogram>(N)) {
      Lines.push_back(S->getLine());
      Lines.push_back(S->getScopeLine());
    } else if (auto *L = dyn_cast<DILexicalBlock>(N))
      Lines.push_back(L->getLine());
    else if (auto *V = dyn_cast<DILocalVariable>(N))
      Lines.push_back(V->getLine());
    else if (auto *F = dyn_cast<DIFile>(N))
      Files.push_back(F);
    else if (auto *T = dyn_cast<DIType>(N)) {
      if (T->getLine() != 0 ||
          !(isa<DIBasicType>(T) || isa<DIDerivedType>(T) ||
            isa<DICompositeType>(T) || isa<DISubroutineType>(T)))
        Supported = false;
    } else if (!(isa<MDTuple>(N) || isa<DICompileUnit>(N) ||
                 isa<DILexicalBlockFile>(N) || isa<DIExpression>(N) ||
                 isa<DISubrange>(N) || isa<DIEnumerator>(N) ||
                 isa<DINamespace>(N) || isa<DITemplateTypeParameter>(N) ||
                 isa<DITemplateValueParameter>(N))) {
      Supported = false;
    }
    for (const MDOperand &O : N->operands())
      visit(O.get());
  }

  void attachments(GlobalObject &O) {
    SmallVector<std::pair<unsigned, MDNode *>, 8> MDs;
    O.getAllMetadata(MDs);
    for (auto &MD : MDs)
      visit(MD.second);
  }

  explicit DebugGraph(Module &M) {
    for (auto &N : M.named_metadata()) {
      if (N.getName().starts_with("llvm.dbg.") && N.getName() != "llvm.dbg.cu")
        Supported = false;
      for (MDNode *O : N.operands())
        visit(O);
    }
    for (GlobalVariable &G : M.globals())
      attachments(G);
    for (Function &F : M) {
      attachments(F);
      for (BasicBlock &B : F)
        if (B.getTrailingDbgRecords())
          Supported = false;
      for (Instruction &I : instructions(F)) {
        SmallVector<std::pair<unsigned, MDNode *>, 8> MDs;
        I.getAllMetadata(MDs);
        for (auto &MD : MDs)
          visit(MD.second);
        for (Use &O : I.operands())
          if (auto *V = dyn_cast<MetadataAsValue>(O.get()))
            visit(V->getMetadata());
        for (DbgRecord &R : I.getDbgRecordRange()) {
          visit(R.getDebugLoc().get());
          if (auto *V = dyn_cast<DbgVariableRecord>(&R)) {
            visit(V->getVariable());
            visit(V->getExpression());
            if (V->isDbgAssign()) {
              visit(V->getAssignID());
              visit(V->getAddressExpression());
            }
          } else {
            Supported = false;
          }
        }
      }
    }
  }
};

static std::optional<unsigned> shift(unsigned Line, int64_t Delta) {
  if (Line == 0)
    return 0;
  int64_t Result = int64_t(Line) + Delta;
  if (Result <= 0 || Result > std::numeric_limits<unsigned>::max())
    return std::nullopt;
  return unsigned(Result);
}

static TempMDNode copyNode(MDNode *N, int64_t Delta, bool Normalize,
                           DIFile *NewFile) {
  auto line = [&](unsigned L) { return Normalize ? 0 : *shift(L, Delta); };
  LLVMContext &C = N->getContext();
  if (auto *L = dyn_cast<DILocation>(N))
    return DILocation::getTemporary(C, line(L->getLine()), L->getColumn(),
                                   L->getRawScope(), L->getRawInlinedAt(),
                                   L->isImplicitCode(), L->getAtomGroup(),
                                   L->getAtomRank());
  if (auto *S = dyn_cast<DISubprogram>(N))
    return DISubprogram::getTemporary(
        C, S->getScope(), S->getName(), S->getLinkageName(), S->getFile(),
        line(S->getLine()), S->getType(), line(S->getScopeLine()),
        S->getContainingType(), S->getVirtualIndex(), S->getThisAdjustment(),
        S->getFlags(), S->getSPFlags(), S->getUnit(), S->getTemplateParams(),
        S->getDeclaration(), S->getRetainedNodes(), S->getThrownTypes(),
        S->getAnnotations(), S->getTargetFuncName(), S->getKeyInstructionsEnabled());
  if (auto *L = dyn_cast<DILexicalBlock>(N))
    return DILexicalBlock::getTemporary(C, L->getScope(), L->getFile(),
                                      line(L->getLine()), L->getColumn());
  if (auto *V = dyn_cast<DILocalVariable>(N))
    return DILocalVariable::getTemporary(
        C, V->getScope(), V->getName(), V->getFile(), line(V->getLine()),
        V->getType(), V->getArg(), V->getFlags(), V->getAlignInBits(),
        V->getAnnotations());
  if (auto *F = dyn_cast<DIFile>(N)) {
    if (Normalize)
      return DIFile::getTemporary(C, F->getFilename(), F->getDirectory());
    return NewFile->clone();
  }
  return N->clone();
}

static std::unique_ptr<Module> translated(Module &M, DebugGraph &G,
                                          int64_t Delta, bool Normalize,
                                          DIFile *NewFile) {
  for (unsigned L : G.Lines)
    if (!Normalize && !shift(L, Delta))
      return nullptr;
  ValueToValueMapTy VM;
  SmallVector<TempMDNode, 64> Temps;
  for (MDNode *N : G.Nodes) {
    Temps.push_back(copyNode(N, Delta, Normalize, NewFile));
    VM.MD()[N].reset(Temps.back().get());
  }
  for (auto &T : Temps)
    for (unsigned I = 0; I != T->getNumOperands(); ++I) {
      auto Found = VM.MD().find(T->getOperand(I).get());
      if (Found != VM.MD().end())
        T->replaceOperandWith(I, Found->second.get());
    }
  for (size_t I = 0; I != Temps.size(); ++I) {
    MDNode *N;
    if (G.Nodes[I]->isDistinct())
      N = MDNode::replaceWithDistinct(std::move(Temps[I]));
    else
      N = MDNode::replaceWithUniqued(std::move(Temps[I]));
    VM.MD()[G.Nodes[I]].reset(N);
  }
  for (auto &Entry : VM.MD())
    if (auto *N = dyn_cast_or_null<MDNode>(Entry.second.get()))
      if (!N->isResolved())
        N->resolveCycles();
  return CloneModule(M, VM);
}

static std::unique_ptr<Module> parse(const uint8_t *Data, size_t Len,
                                      LLVMContext &C) {
  auto Buffer = MemoryBuffer::getMemBuffer(
      StringRef(reinterpret_cast<const char *>(Data), Len), "relocation", false);
  auto Result = parseBitcodeFile(Buffer->getMemBufferRef(), C);
  if (!Result) {
    consumeError(Result.takeError());
    return nullptr;
  }
  bool BrokenDebug = false;
  if (verifyModule(**Result, nullptr, &BrokenDebug) || BrokenDebug)
    return nullptr;
  return std::move(*Result);
}
} // namespace

extern "C" LLVMModuleRef LLVMRustRelocationRefreshDebugInfo(
    LLVMModuleRef CurrentRef, const uint8_t *OldData, size_t OldLen,
    const uint8_t *OptimizedData, size_t OptimizedLen, uint32_t *Status) {
  *Status = MalformedModule;
  Module &Current = *unwrap(CurrentRef);
  auto Old = parse(OldData, OldLen, Current.getContext());
  auto Optimized = parse(OptimizedData, OptimizedLen, Current.getContext());
  if (!Old || !Optimized)
    return nullptr;
  *Status = UnsupportedDebug;
  DebugGraph OG(*Old), CG(Current), PG(*Optimized);
  if (!OG.Supported || !CG.Supported || !PG.Supported)
    return nullptr;
  *Status = SemanticDifference;
  if (!sameSemantics(*Old, Current))
    return nullptr;
  if (OG.Files.empty() && CG.Files.empty() && PG.Files.empty() &&
      OG.Lines.empty() && CG.Lines.empty() && PG.Lines.empty()) {
    *Status = Success;
    return wrap(Optimized.release());
  }
  *Status = UnsupportedMapping;
  if (OG.Files.size() != 1 || CG.Files.size() != 1 || PG.Files.size() != 1 ||
      OG.Lines.size() != CG.Lines.size() || OG.Lines.empty())
    return nullptr;
  DIFile *OF = OG.Files.front(), *CF = CG.Files.front(), *PF = PG.Files.front();
  if (OF->getFilename() != CF->getFilename() ||
      OF->getDirectory() != CF->getDirectory() ||
      OF->getFilename() != PF->getFilename() ||
      OF->getDirectory() != PF->getDirectory() ||
      OF->getChecksum() != PF->getChecksum() || OF->getSource() != PF->getSource())
    return nullptr;
  std::optional<int64_t> Delta;
  for (size_t I = 0; I != OG.Lines.size(); ++I) {
    unsigned A = OG.Lines[I], B = CG.Lines[I];
    if (!A || !B) {
      if (A != B)
        return nullptr;
      continue;
    }
    int64_t D = int64_t(B) - int64_t(A);
    if (Delta && *Delta != D)
      return nullptr;
    Delta = D;
  }
  if (!Delta)
    return nullptr;
  auto ON = translated(*Old, OG, 0, true, nullptr);
  auto CN = translated(Current, CG, 0, true, nullptr);
  if (!ON || !CN || serialize(*ON) != serialize(*CN))
    return nullptr;
  *Status = UnsupportedDebug;
  auto Refreshed = translated(*Optimized, PG, *Delta, false, CF);
  bool BrokenDebug = false;
  if (!Refreshed || verifyModule(*Refreshed, nullptr, &BrokenDebug) || BrokenDebug ||
      !sameSemantics(*Optimized, *Refreshed))
    return nullptr;
  *Status = Success;
  return wrap(Refreshed.release());
}
