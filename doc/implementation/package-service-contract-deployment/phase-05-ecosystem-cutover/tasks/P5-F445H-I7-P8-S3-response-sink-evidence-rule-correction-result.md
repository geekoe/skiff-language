# P5-F445H I7 P8 S3 Response sink evidence-rule correction result

状态：

```text
PASS
DOCS_ONLY = YES
S3_EVIDENCE_RULE_CORRECTED = YES
S3_IMPLEMENTATION_RESUME_UNBLOCKED = YES
S3_COMPLETE = NO
I_RESUME_UNBLOCKED = NO
DECISION_REQUIRED = NO
```

## 1. Input

本次纠偏锚定Skiff：

```text
baseline commit = 405cc99b3af136429a52ecf8b85dbf7d044e5438
baseline tree   = 30a74f00a971055abcbbc59b2f37f7fcca80ad17
fixture commit  = e45e81e8f37c1734fd25e16d72ddbf7ff68c757e
fixture tree    = efd1872329a86bae50b55289ead23d12cc296018
```

S3 fixture在未修改production的candidate上连续两次RED。最终Host terminal是
`unknown Stream value`，但task-local trace证明它不是首个偏离：

```text
stream-0 outer HTTP producer       same registry / generation 1 / exact create+lookup
stream-1 overlay source argument   same registry / generation 1 / exact create+lookup
stream-2 dependency producer       same registry / generation 1 / exact create+lookup

outer stream-0 creation:
  response sink = absent

native emitResponseStream enter:
  response sink = absent
  current stream sink = present
  native fails

cleanup:
  cancellation/drain later surfaces unknown Stream value
```

`runtime_http_gateway`的server-stream路径通过
`execute_runtime_assembly_addr_with_stream_defer(..., Env::new())`建立outer deferred producer；
`program_invocation`中另一个设置response sink的路径未经过。

## 2. Corrected classification

错误按首个可观测语义偏离分类，不按最终错误字符串分类：

| trace | owner |
| --- | --- |
| stream id、registry或request generation在create/register/lookup首先缺失或不一致 | 退回S2 stream association |
| 三个stream identity完整一致，response sink absent先于native失败 | 留在S3 response-sink propagation |
| native失败后cancel/cleanup才出现`unknown Stream value` | S3的secondary cleanup error |
| 两类偏离先后无法确定 | 停止并补trace |

因此当前RED仍是S3可执行输入。它不重新打开S2，不改变S1/S2状态，也不能写成S3 PASS。

## 3. Frozen minimal repair

production owner仅为：

```text
runtime/eval/src/program_stream.rs
runtime/eval/src/runtime_http_gateway.rs
```

修复只能在outer deferred stream已创建后，按其exact stream id、在drive前，把该producer已有的stream sink
以既有`TypedStreamSink` view附着到同一个parked producer env，然后沿现有drive路径执行。必须复用existing
sink、item plan、deferred registry和request scope。

禁止：

- 创建第二sink/channel、第二registry、全局或跨request状态；
- 新增public API、协议、schema、header或测试专用context；
- 修改`program_execution`、`program_invocation`或`env`来建立旁路；
- 只调整错误优先级、吞掉cleanup错误或伪造GREEN。

missing/wrong/taken stream id、重复附着或scope不一致必须fail closed。实现完成前，这只是冻结repair合同，
不是对代码状态的描述。

## 4. DAG and acceptance

DAG和blocker保持：

```text
S2 PASS -> S3 -> I resume -> X -> J
```

S3 result必须报告primary/secondary error分类、三个stream identity、sink presence、exact-id attach时序、实际
两文件production写集、无新增sink/state/public API，以及normal/error/cancel/outside-context和S1/S2回归。
只有完整GREEN后才可设置：

```text
S3_COMPLETE = YES
I_RESUME_UNBLOCKED = YES
```

本result只解除S3按修正规则继续实现，不解除I。

## 5. Validation

本节点为docs-only，未运行build/test/live/network/stable/Mongo/OAuth/browser。只执行：

```text
git diff --check
git grep（unknown classification、first divergence、owner、DAG与禁止机制）
```

result提交与最终tree由handoff报告，不在本文自引用。
