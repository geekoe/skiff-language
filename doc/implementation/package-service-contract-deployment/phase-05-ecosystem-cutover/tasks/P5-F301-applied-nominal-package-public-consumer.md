# P5-F301 Applied nominal package/public compiler consumer

状态：Implemented checkpoint。结果见
`P5-F301-applied-nominal-package-public-consumer-result.md`。

## 直接父节点与权威链

- compiler producer检查点：
  `P5-F296-applied-nominal-compiler-consumer-result.md`
- shared DTO验收：
  `P5-F295-applied-nominal-model-acceptance-result.md`
- owner与public generic policy：
  `P5-F293-generic-nominal-type-ref-owner-audit-result.md`

父链继续引用唯一权威设计。启动时只读本任务；需要依据时沿父链向上读取。

## DAG位置与共享状态

- 节点：F293 S2 package-local ABI + projection + explicit public-schema fail-close。
- F296已产生并遍历唯一structured `AppliedNominal`；F295冻结wire/identity generation。
- 当前policy：package/internal fully-instantiated generic nominal可用；generic nominal不得进入
  `PackageSchema`、`ServiceProtocol`或public typed error envelope。
- 与F300 linked exception facts并行，production范围不重叠。
- 完成后解除compiler combined probe与`A2-language`独立验收。
- 当前是实现检查点，不是稳定候选。

## Production范围

允许：

- `compiler/compiled/**`
- `compiler/projection-input/**`
- `compiler/projection/**`
- `compiler/driver/source_compile/canonical_dependencies.rs`
- 仅作为package/public projection-input producer：
  - `compiler/lowering/src/entrypoint_abi.rs`
  - `compiler/lowering/src/entrypoint_abi_model.rs`

允许上述owner的co-located tests，以及本任务指定的`compiler/tests/**` fixture。禁止修改compiler
core/source、其它lowering、artifact-model/identity、runtime、router、std、生态仓库或权威文档。

## 完成标准

### 1. Package-local信息无损

- compiled projection input、canonical dependency binding、package callable signature、
  implementation link与visible-type normalization递归保留`AppliedNominal` wrapper、ordered
  arguments及exact base owner；
- `PackageSymbol` base与nested argument分别绑定正确package Local ABI expectation；
- local/publication/service/package base的可见化只做与plain ref相同的exact owner转换；
- 不把applied nominal退化成plain ref、anonymous shape、source/display string或bare schema ref；
- same symbol path/shape但不同package owner不合并，argument reorder不归一成同一类型。

### 2. Package ABI declaration DTO收敛

- package/public projection-input保留F295冻结的五种declaration kind：
  record、representation、named union branches、alias、interface；
- generic declaration保留ordered type parameters，named union保留三种branch及enclosing
  declaration context；
- 删除旧`Union { variants }`、discriminator/anonymous-union flattening与只覆盖
  record/alias/union的旧match；
- 不新增第二套wire或兼容DTO；该层只作为compiler内部typed handoff。

### 3. Public generic显式fail closed

- package link/local ABI仍可持有fully-instantiated applied nominal；
- 下列入口遇到generic declaration或`AppliedNominal`必须返回现有typed
  projection/boundary unavailable错误，不panic、不丢arguments后继续：
  - local/dependency `PackageSchema` projection与closure；
  - package public schema record/index；
  - service boundary/`ServiceProtocol`类型投影；
  - 可成为public typed error schema的类型；
- `ResolvedPackageSchema`拒绝手工或store输入中non-empty canonical `type_params`，防止绕过producer；
- 当前generation的applied `PackageSchema`继续由strict admission拒绝，本任务不改变shared DTO；
- public schema/index不写入generic declaration的部分记录或伪closed shape。

### 4. 旧路径删除

- production中不再匹配`TypeDescriptorIr::Union { variants }`或读取`.variants`；
- 所有授权owner对`TypeRefIr`穷举`AppliedNominal`，语义walk递归arguments；
- source text仅可用于诊断/authoring，不参与owner、schema或ABI恢复；
- 不新增legacy adapter、dual path、fallback或public generic wire/identity。

## 最小测试与验证owner

至少覆盖：

- package-local `Box<string>`与`Box<number>`完整保留且identity不同；
- cross-package same path/shape不同owner不合并，nested arguments分别绑定ABI expectation；
- record、representation、named union三类generic declaration及三种union branch在typed handoff中无损；
- package callable/implementation link允许fully-instantiated applied nominal；
- local/dependency public schema、service boundary及public error候选遇generic显式失败；
- forged generic `ResolvedPackageSchema`拒绝且不产生partial index/records；
- non-generic schema/contract/package callable行为保持。

唯一owner：

```bash
cargo test -p skiff-compiler-compiled --lib -- --list
cargo test -p skiff-compiler-compiled --lib --no-fail-fast
cargo test -p skiff-compiler-projection-input --lib -- --list
cargo test -p skiff-compiler-projection-input --lib --no-fail-fast
cargo test -p skiff-compiler-projection --lib -- --list
cargo test -p skiff-compiler-projection --lib --no-fail-fast
cargo test -p skiff-compiler --test test_artifact_identity -- --list
cargo test -p skiff-compiler --test test_artifact_identity --no-fail-fast
git diff --check
```

先确认selector非零。若runtime等旧consumer遮挡compiler integration，记录精确首错；不得越界修复。
不运行workspace、runtime、生态、stable、live或chat smoke。

## 风险与交付

- 风险：高；完成后与F296进入`A2-language`独立验收。
- worktree：`/Users/geek/workspace/skiff-p5-f301-package-public`
- branch：`codex/p5-f301-package-public`
- 从包含F296 result的当前integration checkpoint创建；不push、不操作stable。
- 启动到第一次production修改不超过5分钟；不可执行时返回
  `TASK_NOT_EXECUTABLE`、精确缺口与最小前置。
- 提交后返回commit、package-local preservation/public fail-close矩阵、旧路径反搜、自验收与遮挡；
  不承接combined probe、A2或fixture-wide refresh。
