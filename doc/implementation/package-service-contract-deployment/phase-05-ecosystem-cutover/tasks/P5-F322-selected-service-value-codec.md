# P5-F322 Selected service value codec

状态：Ready。

## 直接父节点

- current channel owner/delta audit：
  `P5-F319-service-error-channel-delta-audit-result.md`

父节点已追溯F298 index与唯一权威设计。本任务只实现父节点R0识别出的第二个共享API事实，不实现
service error分类、index lookup或caller materialization。

## DAG位置与候选

- 节点：R0b；与R0a imported cause并行，二者完成后解除R0c canonical orchestrator。
- 当前状态是implementation checkpoint，不是A5验收候选。
- 证据基线：worktree创建时integration HEAD；`ServiceValuePlan`、binary union format或
  `PayloadBoundaryKind`变化会使本证据失效。

## 唯一写入范围

- `runtime/boundary/src/service_value_plan.rs`
- `runtime/boundary/src/service_value_plan_tests.rs`
- `runtime/boundary/src/lib.rs`仅导出本任务新增的public selection/result DTO

禁止修改model/eval/linker/loader/capability/request/host/transport/router/std、artifact generation及权威设计。

## 完成标准

在现有`ServiceValuePlan`上增加一个严格的selected binary API，语义等价于：

```text
encode_binary_selected(value, Root | NamedUnionBranch(index), boundary, heap)
decode_binary_selected(bytes, boundary, heap)
  -> { value, Root | NamedUnionBranch(index) }
```

具体名字可调整，但必须满足：

- record/representation/non-union root只接受`Root`，拒绝branch selection；
- named union编码必须显式给出branch index，不允许shape-based first match；
- index必须在compiled root branches范围内，且只按选定branch plan编码；
- named union解码保留binary payload中实际branch ordinal并返回，不丢失；
- same-shape branches以不同ordinal round-trip后仍可区分；
- decoded ordinal越界、trailing bytes、payload不符、selection/root不匹配全部严格拒绝；
- nested union只返回root selection；nested payload继续由原计划递归验证；
- `PayloadBoundaryKind::ServiceResponse`及其它已有boundary限制照常执行；
- 现有普通成功值API可保持其当前调用语义，但新service error路径不得经旧shape matcher；
- 不改变binary wire generation、不加legacy/default/dual format。

## 探针

至少覆盖：

- record和representation `Root`正例；
- same-shape named-union两个branch分别选择、编码、解码，ordinal和payload精确；
- wrong branch index、union+Root、record+branch、branch payload mismatch、ordinal tamper、trailing bytes负例；
- boundary kind限制仍有效；
- 旧ordinary encode/decode回归不变。

```bash
cargo test -p skiff-runtime-boundary --lib -- --list
cargo test -p skiff-runtime-boundary --lib --no-fail-fast
cargo fmt -p skiff-runtime-boundary -- --check
git diff --check
```

selector必须非零。不运行eval/workspace/stable/live。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f322-selected-codec`
- branch：`codex/p5-f322-selected-codec`
- 风险：高，共享binary codec API；进入R0 independent acceptance；
- 新的一次性Agent，5分钟内修改；提交并返回selection/ordinal/negative矩阵；
- 不push、不承接R0c。

