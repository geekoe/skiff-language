# P5-F445H I7 P8 D1 PackageDirect HTTP stream task refinement result

状态：

```text
PASS
S1_READY_FOR_ZERO_WORKTREE_PREFLIGHT = YES
DECISION_REQUIRED = NO
```

## 1. Zero-worktree facts

精确baseline为Skiff
`ff6418f5a43ee503608cf8f54512bd9f53a47a74`
（tree `a6ea21c20231e40db69960f70cc6850a7723f871`），工作区clean。只读探查确认：

- T已经分别覆盖真实raw HTTP handler stream和HTTP child当前runtime中的effect stream；
- F255覆盖`PackageDirect` stream，但consumer直接位于顶层调用内；
- 两者都没有覆盖raw HTTP wrapper迭代`PackageDirect` producer的交叉形状；
- `PackageDirect`使用当前`ProgramExecutionContext`/Interpreter；静态代码没有证明它新建了第二个
  `StreamRuntime`；
- producer与consumer heap可以不同，已有`StreamInternalItem`负责item搬运；
- HTTP parent/child各自有request-local stream registry；父子只共享`TestEffectCaseContext`的wire
  snapshots，child在自己的runtime生成effect stream；
- service stream有独立boundary materialization owner，不能改为共享package-local registry。

I的可恢复checkpoint观察到四条`unknown Stream value`，但现有日志没有create/lookup两侧的registry
identity、request generation与stream id。故当前根因仍未知，不能直接选择production修复点。

## 2. Minimal execution decision

新增S1作为唯一上游Runtime/Host闭合节点：

```text
T
↓
S1 concrete Host/raw HTTP × wrapper→PackageDirect stream
↓
I resume
↓
X
↓
J
```

S1必须先让交叉fixture稳定RED并记录身份，再只修既有association/lifetime。若无法得到稳定RED、证据落到
其它owner或仍有多个实现方向，S1停止并返回`TASK_NOT_EXECUTABLE`/`TASK_SCOPE_EXPANDED`，不做猜测性修复。

## 3. Authority clarification

`package-service-contract-deployment.md`只增加已有语义之间的关系：

- same request/assembly的wrapper→`PackageDirect` stream共享当前registry；
- heap可以不同，item通过`StreamInternalItem`搬运；
- service boundary仍rematerialize；
- HTTP parent/child registry隔离，只有effect wire snapshots在child当前runtime生成handle。

没有新增registry、协议、header、schema、compiler、Router、test-runner或标准库surface。

## 4. Gate refinement

J除既有T、AIHub、Agine和受影响Skiff selectors外，增加：

- Codex Relay全部default isolated tests，发现数不得下降；
- official packages全部default offline tests，发现计划与实际必须一致；
- Account继续只跑现有receipt/assembly checks，不伪造不存在的explicit test service。

冻结candidate后的独立矩阵可并行，但必须受磁盘、独立Cargo/artifact缓存、端口lease与isolated stack资源
约束；共享可变资源时串行。失败只分类和收集诊断，不在gate中修改候选。

这些独立矩阵也可在S1/I关键路径进行时针对精确pre-acceptance candidate提前做只读诊断；该证据不冒充
最终PASS。全部blocker批量闭合并冻结final candidate后，J唯一owner再建立最终验收结果。

## 5. Validation

本节点为docs-only；未运行build/test/live/network/stable/Mongo。执行：

```text
git diff --check
git grep（任务/DAG/禁止机制反向检查）
```

result提交与最终tree由handoff报告，不在本文自引用。
