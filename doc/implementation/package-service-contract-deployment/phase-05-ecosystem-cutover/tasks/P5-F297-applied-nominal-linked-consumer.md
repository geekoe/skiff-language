# P5-F297 Applied nominal linked/type-plan consumer

状态：Ready。

## 直接父节点

- `P5-F295-applied-nominal-model-acceptance-result.md`

父结果继续引用F293 owner审计与唯一权威设计。启动时只读本任务；需要依据时沿父链向上读取。

## DAG位置与并行边界

- 节点：F293 `S3 linked-program + linker + linked-type-plan`。
- 与F296并行：本任务只修改runtime linked/linker/type-plan，禁止修改compiler。
- 输入是F295冻结的AppliedNominal与五种declaration/三种named-union branch DTO。
- 完成后解除runtime carrier/catch节点；不自行实现value carrier或service error channel。
- 当前是实现检查点，不是稳定候选。

## 唯一production范围

- `runtime/linked-program/**`
- `runtime/linker/**`
- `runtime/linked-type-plan/**`

允许co-located tests/fixtures。禁止修改runtime model/eval/boundary/request/loader/host/transport、
artifact-model/identity、compiler、router、std、生态仓库或权威文档。

## 完成标准

### 1. Linked canonical shape

新增唯一等价shape：

```text
LinkedTypeRef::AppliedNominal {
  base: LinkedNominalTypeRefBase,
  arguments: Vec<LinkedTypeRef>
}

LinkedNominalTypeRefBase
  = LocalType | PublicationType | ServiceSymbol | PackageSymbol |
    PackageSchema | Address
```

- pre-link完整保留exact locator与ordered non-empty arguments；
- link时递归link arguments，只把base解析成exact `Address`，不能把整个wrapper降成bare Address；
- plain zero-arg nominal仍使用existing Address/plain linked ref；
- missing/unresolved owner、ABI expectation不匹配、empty args、arity/kind错误fail closed；
- post-link executable/type-plan输入中AppliedNominal base必须是Address或允许的exact PackageSchema，
  不能残留unresolved symbol。

### 2. Declaration/branch parity

- linked descriptor精确区分Record、Representation、Union、Alias、Interface；
- named union保留concrete nominal、synthetic discriminator、literal branch及enclosing declaration context；
- interface不能伪装成empty record，representation不能展开成alias；
- union branch中`TypeParam`按enclosing applied owner substitution，branch不恢复旧map。

### 3. Substitution与type plan

- declaration `type_params[i]`与ordered arguments[i]建立唯一substitution；
- nested applied arguments递归substitute；完成执行plan后不得残留unbound TypeParam；
- `Box<string>`与`Box<number>`产生不同instantiated linked facts；generic representation保留外层owner；
- `U<string>`的每个branch保留同一applied union owner context；
- PackageSchema/public generic仍fail closed，不在本任务开放contract/wire；
- 不从display/source text/shape或bare address恢复arguments。

### 4. 全面 traversal

File IR到linked image的所有TypeRef-bearing surface必须保留AppliedNominal：

- declaration/implements、interface operation、const/executable signature；
- construct、pattern、throw/catch、test-effect、DB、actor field；
- call type args、container/record/union/nullable/function/interface nested ref。

任何unsupported surface必须在link/admission时报精确错误，不能静默丢参数。

## 最小测试与验证owner

至少覆盖：

- same declaration string/number args链接后不同；
- nested applied refs；
- representation；
- generic named union三类branch；
- cross-package exact owner/ABI；
- alias展开；
- empty/wrong arity/illegal kind/unresolved owner/unbound TypeParam；
- post-link wrapper仍存在且base为Address；
- tamper/shape/display不参与。

唯一owner：

```bash
cargo test -p skiff-runtime-linked-program --lib -- --list
cargo test -p skiff-runtime-linked-program --lib --no-fail-fast
cargo test -p skiff-runtime-linker --lib -- --list
cargo test -p skiff-runtime-linker --lib --no-fail-fast
cargo test -p skiff-runtime-linked-type-plan --lib -- --list
cargo test -p skiff-runtime-linked-type-plan --lib --no-fail-fast
git diff --check
```

若runtime下游旧consumer遮挡，使用能实际枚举的最窄owner tests并记录精确首错；不得修改范围外。
不运行workspace、runtime-eval、生态、stable、live或chat smoke。

## 风险与交付

- 风险：高；后续与runtime carrier/catch一起进入独立runtime验收。
- worktree：`/Users/geek/workspace/skiff-p5-f297-applied-nominal-linked`
- branch：`codex/p5-f297-applied-nominal-linked`
- 不push、不操作stable。
- 启动到第一次production修改不超过5分钟；不可执行时返回
  `TASK_NOT_EXECUTABLE`、精确缺口和最小前置。
- 完成后提交并返回commit、linked shape、反向搜索、自验收与任何owner/设计缺口；
  不自行承接runtime carrier、wire或S2。
