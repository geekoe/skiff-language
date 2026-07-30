# P5-F445H-I6J current-scope combined probe result

状态：

```text
PASS
MERGED_PROBE_FAIL = NO
TASK_NOT_EXECUTABLE = NO
READY_FOR_INDEPENDENT_I6_ACCEPTANCE = YES
```

所有 I6 parent implementation 与 service scope-reduction receipt 已在同一个 merged baseline
`f12ee51b3c77635d8d182e09152c995ae0ac35d0` /
`ea44d0e04f89b22573c6bd2dd63569ad20bdc808` 上合流。12 个非零 selector 的 listing 与 execution
精确一致，合计 `68 listed / 68 passed`；四包 locked check 与静态禁止面核对通过。

本节点没有实现或修改 production、tests、fixtures、Cargo manifests 或 lockfile，没有运行 full
gate，也没有访问 network、stable/live 或 MongoDB。

## 1. 合同恢复与候选身份

执行合同：

```text
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
P5-F445H-I6J-current-scope-combined-probe-resume.md
```

旧 I6R §8.6 要求父 merge 前新增独立 combined test并记录 RED；当前 baseline 已完成父 merge且该
test 不存在，本节点又禁止修改 tests。因此 resume 合同只把已提交的 hermetic parent receipt 在同一
merged tree 上聚合重建，不伪造历史 RED、不接受零 selector，也不替代后续独立四 crate acceptance。

| 项 | 值 |
| --- | --- |
| baseline commit | `f12ee51b3c77635d8d182e09152c995ae0ac35d0` |
| baseline tree | `ea44d0e04f89b22573c6bd2dd63569ad20bdc808` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i6j-combined-probe` |
| branch | `codex/p5-f445h-i6j-combined-probe` |
| result commit/tree | 由最终交付消息记录；避免 result 文件自引用 |
| Cargo target | worktree-local `build/cargo-target` |
| network mode | `CARGO_NET_OFFLINE=true` |

预检确认 E1、HTTP、WebSocket、time、file、Actor、response sink 与 service decision 的固定提交都
是 baseline 祖先；probe 前后 HEAD 未变化。

## 2. 非零 selector 结果

所有命令 exit `0`。Cargo 每条命令都使用离线模式和同一个 worktree-local target。

| parent / receipt | package / selector | list | run |
| --- | --- | ---: | ---: |
| E1 carrier delivery | eval `f445h_i6_carrier_delivery_receipt` | 5 | 5/5 |
| HTTP | host `f445h_i6_http_current_scope` | 11 | 11/11 |
| WebSocket registry | capability-context `f445h_i6_connection_request_scope` | 7 | 7/7 |
| WebSocket Host projection | host `f445h_i6_websocket_scope` | 6 | 6/6 |
| time native owner | native `f445h_i6_time_scope` | 7 | 7/7 |
| time Eval projection | eval `f445h_i6_time_projection_to_pending` | 1 | 1/1 |
| file Host owner | host `f445h_i6_file_scope` | 6 | 6/6 |
| file Eval projection | eval `f445h_i6_file_projection_to_pending` | 1 | 1/1 |
| Actor Eval prepared/spawn | eval `f445h_i6_actor_scope` | 4 | 4/4 |
| Actor Host owner | host `f445h_i6_actor_scope` | 15 | 15/15 |
| response sink | capability-context `f445h_i6_response_sink_scope` | 4 | 4/4 |
| canonical service current scope | eval `f445h_e4r7_stream_deadline_pending_unary_preserves_inherited_request_carrier` | 1 | 1/1 |
| **合计** | 12 selectors | **68** | **68/68** |

这 12 组 selector 共执行 24 条 Cargo test命令：12 条 `--list`、12 条真实执行。没有 ignored、
零 test、listing/execution不一致或测试失败。

## 3. Locked 接线

第 25 条动态命令：

```bash
cargo check -p skiff-runtime-capability-context -p skiff-runtime-native \
  -p skiff-runtime-eval -p skiff-runtime-host --locked
```

结果为 PASS；只有 baseline 已有的 dead-code、unused-import 与 unreachable-pattern warnings。没有
依赖解析、Cargo/lockfile写入或网络访问。

## 4. 静态禁止面

四条合同静态命令均通过或完成可解释分类：

1. `rg -n '\$/cancelRequest|-32800|CancelError' runtime router std`
   - `$/cancelRequest` 只命中 `router/tests` 的拒绝/profile/broker负例；
   - `CancelError` 只命中 Router tests、runtime model/eval 的 legacy spelling fail-closed tests
     与 test fixture；没有恢复 production peer cancel、`-32800` 或公开 cancel error。
2. `rg -n 'service_dispatch|outbound_service' runtime/host/tests`
   - 0 hits；combined证据没有依赖 legacy relay。
3. service timeout reduction搜索
   - 权威 architecture/reference各保留一处“第一版不定义 consumer dependency / callee operation
     timeout”条款；
   - production combined路径无新增字段或 policy复用；
   - `runtime/eval/src/test_support.rs` 的 `ServiceTimeoutConfig` 仅是 legacy test-support fixture，
     不是 canonical service production owner。
4. `git diff --check`
   - PASS。

## 5. 命令计数与写集

合同命令计数：

```text
24 Cargo selector commands
 1 four-package locked check
 3 static rg commands
 1 git diff --check
29 total contract commands
```

额外的 baseline ancestry、status 与 diff-name读取只用于身份/clean核对，不计入 combined probe。

实际 tracked 写集只有：

```text
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
  P5-F445H-I6J-current-scope-combined-probe-resume.md
  P5-F445H-I6J-current-scope-combined-probe-result.md
```

`build/cargo-target` 是 ignored、worktree-local、可再生缓存，不进入提交。production、tests、
fixtures、Cargo manifests 与 `Cargo.lock` 相对 baseline 都是零 diff。

## 6. 结论与失效条件

```text
READY_FOR_INDEPENDENT_I6_ACCEPTANCE = YES
```

这只解除独立 I6 acceptance，不宣称 I6/F445H/Phase 05 已验收完成。独立 owner仍需在同一候选上
按 I6R §8.7 各运行一次 capability-context、native、eval、host 完整 crate gate，再运行 locked
check、fmt、diff与冻结反向搜索。

baseline 后任何影响 I6 production/test/fixture、service decision、Cargo依赖或 probe/acceptance
工具的变化都会使本结果失效；此时必须重新建立 merged combined probe，不能沿用本结论。
