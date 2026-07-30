# P5-F321 Imported service exception cause

状态：Completed。结果见
`P5-F321-imported-service-exception-cause-result.md`。

## 直接父节点

- current channel owner/delta audit：
  `P5-F319-service-error-channel-delta-audit-result.md`

父节点已追溯F299 local carrier、F298 index、F305 platform registry与唯一权威设计。本任务只实现父节点R0
识别出的第一个共享API事实，不实现service export/import。

## DAG位置与候选

- 节点：R0a；与R0b branch-aware codec并行，二者完成后解除R0c canonical orchestrator。
- 当前状态是implementation checkpoint，不是A5验收候选。
- 证据基线：worktree创建时的integration HEAD；任何
  `RequestExceptionCause`/`OpaqueServiceError`/`RuntimeValueCarrier`形状变化会使本证据失效。

## 唯一写入范围

- `runtime/model/src/service_error.rs`

禁止修改boundary/eval/linker/loader/capability/request/host/transport/router/std及权威设计。只允许同文件单测。

## 完成标准

将当前二选一原因：

```text
Local { value }
OpaqueService { error }
```

收敛为能表达以下事实的单一模型：

```text
Local { value }
ImportedService {
  error: OpaqueServiceError,
  local_value: Option<RuntimeValueCarrier>
}
```

具体Rust命名可调整，但必须满足：

- linked public/platform/Internal inbound可同时保存原始fixed bytes和caller-local carrier；
- unlinked合法public inbound保存同一raw envelope且`local_value=None`；
- local catch identity只读取`Local.value`或`ImportedService.local_value`，opaque-only必定miss；
- `map_local_value`移动/替换两类本地carrier，但绝不decode、重写或丢失raw bytes；
- 提供只读fixed service error accessor；仅imported cause返回原始`OpaqueServiceError`；
- local constructor/rethrow继续产生/保留`Local`；imported constructor不得伪装成本地throw；
- strict `OpaqueServiceError::decode`、原bytes保存、correlation与现有stack/source行为不变；
- 不增加legacy enum variant、optional raw bytes、dual representation、display/name/code fallback。

## 探针

同文件单测至少覆盖：

- linked imported exact catch，未捕获时fixed bytes逐字节不变；
- unlinked imported catch miss，map操作后raw bytes仍不变；
- imported `InternalError`和`PublicTypedError`均可持local carrier，fixed accessor返回同一envelope；
- local cause无fixed accessor，local rethrow的source/stack/correlation保持；
- `map_local_value`对`None`不凭空materialize；
- malformed envelope仍由现有strict decode拒绝。

```bash
cargo test -p skiff-runtime-model --lib -- --list
cargo test -p skiff-runtime-model --lib --no-fail-fast
cargo fmt -p skiff-runtime-model -- --check
git diff --check
```

selector必须非零。不运行eval/workspace/stable/live。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f321-imported-cause`
- branch：`codex/p5-f321-imported-cause`
- 风险：高，共享request-local模型；进入R0 independent acceptance；
- 新的一次性Agent，5分钟内修改；提交并返回API矩阵、raw-byte/catch/rethrow证据；
- 不push、不承接R0c。
