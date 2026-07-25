# P5-F295 Applied nominal model acceptance result

状态：`PASS`；`A1b-applied-nominal-model`验收通过，无blocking issue。

## Exact candidate

- production candidate commit：
  `e5c36178b35bbfa55d6ce4042053ebe87e1dd257`
- production candidate tree：
  `bc6faaa635e4d5e8542ade84bc6a930640b4d4a7`
- 实际验收任务基线：
  `f09add75a1be01a621abb2b382e3bd86eea7f7d6`
- 实际验收任务基线tree：
  `3c43124ec36a11849825d0640a09b7ad26a829f6`
- candidate merge：
  `e45517b4a198632e81d239a22078f820cd600273`
  （parents为`7184750d2dd28f5cb987d43c82138861502082d3`与exact candidate，
  tree与production candidate相同）

`git diff --name-status e5c36178..f09add75`只新增F294 implementation result与F295
acceptance task两份文档；没有candidate之后的production或test变化。因此本验收实际检查的代码状态与
exact production candidate一致。

## 独立验收矩阵

| 检查项 | 结论 | 独立证据 |
| --- | --- | --- |
| 唯一strict wire | PASS | `artifact-model/src/types.rs`只在`TypeRefIr`定义一个`AppliedNominal { base, arguments }`；base是独立closed `NominalTypeRefBaseIr`，只含Local、Publication、ServiceSymbol、PackageSymbol与PackageSchema。`arguments`只有non-empty deserializer，没有default、alias或skip；Vec保持declaration order。strict测试拒绝missing、null、empty、非法base、base unknown field和plain nominal附加arguments。 |
| plain/applied、kind、arity与scope | PASS | `artifact-model/src/file_ir.rs::validate_file_ir_type_refs`递归验证所有File IR type-ref surface；local/same-module publication ref按type table精确验证。plain ref只允许零type-param declaration；applied ref要求非零且arity精确；Record、Representation和named Union是唯一合法local declaration kind，Alias/Interface、unknown index和unbound `TypeParam`均fail closed。co-located admission负例覆盖empty、wrong arity、illegal kind、plain generic及out-of-scope parameter。 |
| named-union与nested traversal | PASS | `NamedUnionBranchIr::ConcreteNominal`只剩`nominal_type`，production搜索无旧`type_arguments`；`typeArguments`只存在于absence/rejection测试。`visit_type_ref`、descriptor visitor及`executable.rs` visitor递归覆盖descriptor、signature、construct、throw/catch/pattern、test-effect、DB、call type args、interface box与actor field等本任务owner；branch与construct测试证明nested argument进入admission。 |
| exact external owner / ABI | PASS | `cross_package_identity.rs`对symbol path只做exact equality，已删除trailing-segment匹配；applied PackageSymbol base与每个nested argument分别canonicalize，保持closed wrapper、exact package owner、symbol path与原ABI expectation。`applied_nominal_rebinds_exact_base_owner_and_nested_arguments`覆盖alias owner、nested owner和ABI expectation，不读取display或shape。 |
| applied PackageSchema边界 | PASS | File IR与PackageArtifact semantic admission显式拒绝Rust构造的empty arguments和applied PackageSchema；Actor ABI strict decode也拒绝empty wire与applied PackageSchema。candidate未修改`artifact-identity/src/contract.rs`、`artifact-model/src/contract_types.rs`或compiler projection/public contract owner；没有偷渡public generic schema、ServiceProtocol或`PublicTypedError`支持。 |
| identity与generation | PASS | File IR schema/format/prefix精确为v7/v5/v7；PackageArtifact schema为v5；Local ABI marker/prefix为v3/v5；Build marker/prefix为v4/v6。identity tests证明argument type、nested order及base/argument tamper改变或使旧identity失效，旧schema/prefix拒绝，non-generic writer也只产生新generation。PackageArtifact package-version mutation保持Local ABI/Build identity；既有ServiceContract projection继续排除contract version label。 |
| 保持矩阵 | PASS | opcode table v1、legacy Package Unit build/local ABI v2、PackageSchema Type/Index v1、ServiceContract v4、ServiceContractDefinition v3、ServiceProtocol v4、ContractOperation v1、Operation ABI v1与Publication ABI v1均保持；`canonical_generation_markers_bump_without_changing_legacy_package_domains`及contract/package focused suites通过。 |
| production scope | PASS | candidate的production/test diff只有`artifact-model`与`artifact-identity`共17个授权文件；没有compiler、runtime、deployment、router、test-runner、std、scripts或生态写入，没有compat reader、fallback或dual path。 |
| developer tests有效性 | PASS | 两个`--list`分别列出149与93个lib test，不是零selector。新增测试同时包含serde负例、contextual admission、cross-package rebind、actor拒绝、PackageArtifact admission、generation与identity mutation/tamper，不是只做serde round-trip。 |

## Identity/version evidence

| Domain | 验收值 |
| --- | --- |
| File IR schema / format / identity prefix | `skiff-file-ir-v7` / `skiff-file-ir-format-v5` / `skiff-file-ir-v7:sha256` |
| PackageArtifact schema | `skiff-package-artifact-v5` |
| PackageArtifact Local ABI marker / prefix | `skiff-package-artifact-local-abi-identity-v3` / `skiff-package-local-abi-v5:sha256` |
| PackageArtifact Build marker / prefix | `skiff-package-artifact-build-identity-v4` / `skiff-package-build-v6:sha256` |
| Preserved public/legacy domains | Package Unit v2；PackageSchema Type/Index v1；ServiceProtocol v4；ContractOperation/Operation ABI/Publication ABI v1 |

File IR identity preimage包含完整type table、declarations、actors、constants、executables与external refs；
PackageArtifact Local ABI preimage包含public symbols，Build preimage继续包含Local ABI与implementation
surface。因此ordered applied arguments与exact base owner不是仅靠测试命名声称，而是实际canonical
serialization输入。

## 独立探针

实际执行：

```text
cargo test -p skiff-artifact-model --lib -- --list
  149 tests, 0 benchmarks

cargo test -p skiff-artifact-model --lib --no-fail-fast
  149 passed; 0 failed

cargo test -p skiff-artifact-identity --lib -- --list
  93 tests, 0 benchmarks

cargo test -p skiff-artifact-identity --lib --no-fail-fast
  93 passed; 0 failed

git diff --check e5c36178^ e5c36178
  PASS（无输出）
```

补充只读反向检查：

- exact candidate的changed-path列表没有任何compiler/runtime/compat owner；
- `NamedUnionBranchIr`的旧`type_arguments` production field已删除；
  wire名`typeArguments`只剩strict rejection/absence测试；
- AppliedNominal `arguments`没有serde default、alias或skip；
- public contract identity owner `artifact-identity/src/contract.rs`相对candidate parent无diff；
- cross-package symbol path没有suffix/display/shape fallback。

没有运行workspace、compiler、runtime、stable instance、live、生态测试、fmt或chat smoke；没有修改
production/test，没有操作stable，也没有push。

## Blocking issues与consumer handoff

Blocking issues：无。

本结果只接收F293 DAG的S0 shared DTO + artifact identity checkpoint。以下是明确的残余consumer
handoff，不是本验收blocker：

1. language/compiler core、source与lowering必须生产并完整遍历structured applied arguments，删除
   source/display恢复路径，并完成fully-instantiated generic nominal consumer；
2. package/public compiler consumer必须保存package-local applied ref，同时让generic PackageSchema、
   public schema closure与`PublicTypedError`继续显式fail closed；
3. linked-program、linker与linked-type-plan必须保留applied wrapper、递归link/substitute arguments，
   不能退化为bare Address；
4. runtime value/catch lane必须把exact instantiated identity接入carrier、slot/container/call及
   throw/catch；不能用shape、display或static payload重建；
5. 上述consumer稳定后再由唯一机械owner刷新跨crate fixtures/goldens。

## Verdict

`PASS`。exact candidate满足F295八项验收条件，可以解除后续consumer节点；任何下游若要改变本结果冻结的
wire或identity generation，应退回shared checkpoint重新验收，不能引入legacy adapter、dual read/write、
fallback或display/shape inference。
