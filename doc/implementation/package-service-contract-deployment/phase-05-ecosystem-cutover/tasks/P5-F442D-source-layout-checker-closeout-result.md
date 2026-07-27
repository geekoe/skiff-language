# P5-F442D Source-layout checker closeout result

状态：`PASS / K_CLOSED / NON_LIVE_VALIDATED`。

default source-layout checker已收敛到current compiler builtin与`std/api.yml`/source surface。
它现在要求`Actor`并明确拒绝`ActorRef`和`CancelError`，完整覆盖
`std.service.InternalError`、current WebSocket surface和current HTTP surface，同时保留既有
file inventory检查。未发现source/api不一致，因此未触发`TASK_SCOPE_EXPANDED`。

## 1. 基线与提交

| 状态 | Commit | Tree |
| --- | --- | --- |
| 任务声明的implementation baseline | `0303fe5d5d32ab2eacf80dc539a31d2a89b5e806` | `363e2dc885b2555125a745016c7c9c98002ae008` |
| worktree任务起点 | `2989ddf97391e97b99c3c1dd8c3d9468de0d28f7` | `33a82301064a3d8fccfa33b777a90074a522785a` |
| implementation | `07ed30fdd8042a5aac220dc2b276b63df5c7520f` | `58a83147305772337ef7ef89bea2a90403fa3851` |

`2989ddf9`在implementation baseline之上只增加本轮调度文档。Implementation与本文result分离
提交；result commit/tree由最终交付消息记录。

## 2. Test-first RED

在修改checker前，于task-start HEAD执行：

```text
node scripts/check-skiff-source-layout.mjs
```

真实结果为exit `1`：

```text
FAIL compiler builtin registry must own CancelError
```

这证明失败来自checker仍正向要求已删除的public builtin，而不是预造fixture或模拟错误。

## 3. 实现与事实源核对

### 3.1 Compiler builtin

- 正向inventory新增current `Actor`；
- 保留`ActorRef`负向拒绝；
- 删除`CancelError`正向要求，并与`ActorRef`一起作为明确负向拒绝；
- 其它既有builtin inventory不变。

### 3.2 Canonical std surface

checker只读取current `std/api.yml`与对应source，并用同一份required expectation同时校验API映射和
source declaration kind：

| Module | Current required surface |
| --- | --- |
| `std.service` | `ProviderUnavailableError`、`ProtocolError`、`InternalError`三个type |
| `std.websocket` | 4个type；4个direct send native；`requestJsonToConnection` native；2个JSON source helper |
| `std.http` | 10个public type；20个public native/helper |

事实核对确认`std/api.yml`的每个映射与`std/service.skiff`、`std/websocket.skiff`、
`std/http.skiff`的声明名和native/source kind一致。checker另外同时拒绝API或WebSocket source
重新导出旧`receive`、`sendText`、`sendBinary`、`sendJson`名字。

既有`std/file.skiff` type/native inventory、prelude文件inventory及其它source guard均原样保留。
没有新增service contract、receive执行路径、compatibility alias、dual path或旧JSON send API。

## 4. 自验收矩阵

| 任务条款 | 代码证据 | 反向证据 | 结果 |
| --- | --- | --- | --- |
| `Actor`正向；`ActorRef`/`CancelError`负向 | `scripts/check-skiff-source-layout.mjs` builtin inventory及removed loop | compiler registry不存在`name: "ActorRef"`或`name: "CancelError"` | PASS |
| `std.service.InternalError` | shared `requiredStdSurface.service.types`同时驱动API/source检查 | 删除API映射或source type都会使checker失败 | PASS |
| WebSocket 4 type、5 native、2 source helper | `requiredStdSurface.websocket`及source declaration-kind检查 | exact旧`receive/sendText/sendBinary/sendJson` API/source export为0 | PASS |
| HTTP current公开surface | `requiredStdSurface.http`覆盖10 type与20 native/helper | API映射与source declaration共用同一required expectation | PASS |
| 保留file inventory | `std/file.skiff`既有4 type与7 native检查未修改 | implementation commit只改checker，未改std source/API | PASS |
| 默认checks可达 | verify list展开`checks:skiff-source-layout`，命令为当前checker | 仅list，未执行完整checks或live selector | PASS |
| 唯一写集 | implementation只有checker；result提交只有本文 | `git show --stat 07ed30fd`为1 file changed | PASS |

反向搜索使用：

```text
rg -n 'name: "(ActorRef|CancelError)"' compiler/core/src/prelude_registry.rs
rg -n '^\s*(?:native\s+)?function\s+(receive|sendText|sendBinary|sendJson)\b|^  (receive|sendText|sendBinary|sendJson):' \
  std/websocket.skiff std/api.yml
```

两项均为0命中。

## 5. 规定的non-live验证

| 命令 | 结果 |
| --- | --- |
| `node scripts/check-skiff-source-layout.mjs` | PASS：`Skiff source layout checks passed.` |
| `node scripts/verify.mjs --only checks --list` | PASS：展开16个phase；其中`checks:skiff-source-layout`属于`default verify` |
| `git diff --check` | PASS |

任务给定的verify list形式可直接使用，无需替代命令。没有运行stable、network、live、完整checks或
完整verify；没有启动服务或修改本地instance。

## 6. 写集与操作边界

Implementation写集：

- `scripts/check-skiff-source-layout.mjs`

Result写集：

- `doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-F442D-source-layout-checker-closeout-result.md`

未修改compiler registry、prelude/std source、`std/api.yml`、其它checker/test、README或
production。未派sub-agent，未merge、rebase或push。
