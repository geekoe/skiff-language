# P5-F326 Service error core combined probe

状态：Completed。结果见
`P5-F326-service-error-core-combined-probe-result.md`。

## 直接父节点

- imported cause：
  `P5-F321-imported-service-exception-cause-result.md`
- selected codec：
  `P5-F322-selected-service-value-codec-result.md`
- canonical orchestrator：
  `P5-F324-canonical-service-error-channel-core-result.md`

三个开发节点共同组成F319定义的R0 checkpoint。本任务是合流后、独立验收前的唯一cheap combined owner，不是
A5 verdict。

## 精确候选与范围

- 候选：创建worktree时integration HEAD；result必须记录完整commit和tree。
- 只读production。唯一允许写入
  `P5-F326-service-error-core-combined-probe-result.md`并提交。
- 不修改代码、fixture、权威设计，不运行完整eval/workspace/root/stable/live。
- generic WebSocket两个既知失败与本探针无关。

## 必须证明

### 接线

- eval core真实编译并同时消费F321 imported cause和F322 selected codec，而不是测试内复制模型；
- `RuntimeError::FixedServiceFailure`、export/import API、provider stack reset只有一个production owner；
- `ServiceErrorEnvelope`、`ServiceErrorTypeIndex`和platform registry没有在eval新增第二份DTO/allowlist；
- R1/R2/R3 lane尚未接入是预期状态，不能误报A5完成。

### 正负矩阵

- public record/representation/union/dependency owner；
- linked/unlinked/opaque raw forward与三跳不变；
- private/nonclosed/encode failure/Internal once；
- exact local和imported Internal；
- platform exact与Resource package path；
- owner/key/id/build/ordinal/payload mutations；
- local rethrow、remote new stack、provider stack reset；
- private payload/source/display/message不进入fixed bytes。

### 证据命令

```bash
cargo test -p skiff-runtime-model --lib --no-fail-fast
cargo test -p skiff-runtime-boundary --lib --no-fail-fast
cargo test -p skiff-runtime-eval --lib assembly_execution::service_error_channel -- --list
cargo test -p skiff-runtime-eval --lib assembly_execution::service_error_channel --no-fail-fast
cargo check -p skiff-runtime-eval --lib
git diff --check
```

记录selector非零、测试计数、warnings与精确失败。不得重复完整eval。

## 结构与反搜

只做宏观结构检查：

- core production/test文件行数及职责分段；
- 是否存在可以独立拆出但当前造成重复classifier、循环依赖或多owner的明显职责混合；
- `rg`确认没有shape/display/static type/message/code fallback，没有Resource platform映射，没有callee stack
  序列化；
- 不做逐行shadow review，结构疑问交给F327独立验收。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f326-service-error-probe`
- branch：`codex/p5-f326-service-error-probe`
- 新的一次性只读Agent；提交result并返回candidate commit/tree、矩阵和PASS/FAIL；
- PASS只解除F327独立R0验收，不直接解除R1–R3；
- 不push、不承接验收。
