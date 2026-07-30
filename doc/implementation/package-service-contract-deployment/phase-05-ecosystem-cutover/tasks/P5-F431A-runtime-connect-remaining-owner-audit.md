# P5-F431A Runtime connect remaining-owner closure audit

状态：Ready。只读收敛审计；同一路径连续两次scope expansion后的强制闭合检查。

## 直接父节点

- `P5-F430A-runtime-websocket-connect-closure-result.md`

父节点记录已集成safe checkpoint、当前三个编译错误、已知D4遮挡、residual reverse search和建议
最小写集，并继续追溯到F429A/F426A及唯一权威设计。启动时只读本任务；需要依据时再沿引用向上。

## 输入、目的与DAG

精确候选：

| commit | tree |
| --- | --- |
| `64e5be20baa253c1b12f2cd2125b22888112e75d` | `7578f4449bd7f3331670007fc36f53ee0ceb848b` |

Runtime connect路径已连续两次因任务合同遗漏owner而停止。根据多Agent工作流，本审计必须在第三个
implementation closure前一次列全剩余production/fixture owner、关键跳点、遮挡和最小探针。

```text
F431A只读闭合审计
  -> 单一批量Runtime closure
  -> Runtime+Router combined probe
```

本节点不实现、不修编译、不运行完整gate。

## 只读审计范围

允许读取：

- `runtime/**`
- `artifact-model/**`
- F425A/F426A/F429A/F430A任务与result
- Cargo workspace metadata及直接compile diagnostics

唯一允许写入是本leaf result。禁止修改production、test/fixture、generated file、Router、
test-runner、Internals或skiff-packages。

## 必须回答

1. 从以下已知链逐字段列出全部definition、producer、projection、consumer和完整struct literal：

   ```text
   outbound_service::request_start_control
     -> RequestStartControl
     -> OutboundControlMessage::RequestStart
     -> control_mapper::request_start_frame_header
     -> RequestStartFrameHeader
   ```

   重点是待删除的legacy `business_identity`、`websocket_entry_id`、`websocket_adapter`；必须区分
   current `connection.send`中的同名业务字段，后者不能删。
2. 对已删除的`RequestEnvelope.websocket_adapter`、legacy
   request/response/receive/context/operation DTO执行全runtime反搜，列出所有仍会导致compile、
   reverse-search或行为残留的production和direct fixture owner；不能只列当前编译器首先报告的文件。
3. 执行最小compile诊断，确认在exact candidate上：
   - transport first blocker是否只剩父result三处；
   - 修复这些字段后，下游最可能暴露的所有静态owner是否已由反搜列出；
   - D4 test-runner optional-handler三个错误是否是独立、后置遮挡。
4. 复核F430A新增的ordinary provider capability rebinder链：caller/provider entry不同、provider
   零entry、caller零/provider有entry三个测试入口和最终`connection.send` owner是否齐全；列出任何
   尚未纳入授权的production/test owner。
5. 复核current connect执行关键跳点：
   assembly admission -> activation sole entry -> Host current request -> eval callable ->
   accept/reject mapper -> generation pin。逐点列owner、已有测试、上游失败遮挡下游的关系。
6. 给出第三个且应为最终的implementation任务：
   - exact production/test write set；
   - 每个文件的机械或行为变化；
   - 明确禁止面；
   - 便宜compile/test顺序，先暴露静态遗漏再运行被D4遮挡的suite；
   - completion reverse-search allowlist。
7. 若仍存在多个会改变实现方向的未知量，返回`TASK_NOT_EXECUTABLE`并指出需要的设计决策；不能由
   audit自行选择。

## 证据命令

使用`rg`/Cargo metadata和最小`cargo check`；不得为了重复已知事实运行完整13-package suite。
至少记录：

```bash
cargo check -p skiff-runtime-transport
cargo check -p runtime
git diff --check
```

若命令因首错停止，结合definition/literal全量反搜列出后续owner，不能等待编译器逐个报错。审计不
产生候选PASS，只冻结后继合同。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f431a-runtime-closure-audit`
- 分支：`codex/p5-f431a-runtime-closure-audit`

新增并提交`P5-F431A-runtime-connect-remaining-owner-audit-result.md`，返回commit/tree、owner矩阵、
命令结果和clean状态。不得修改production、merge、rebase、push、stable/live；完成后不得自行承接
implementation。
