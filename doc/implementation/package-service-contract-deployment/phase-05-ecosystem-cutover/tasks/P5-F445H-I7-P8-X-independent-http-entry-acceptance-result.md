# P5-F445H I7 P8 X Independent HTTP entry acceptance result

状态：

```text
PASS
READ_ONLY_ACCEPTANCE = YES
BLOCKING_ISSUES = NONE
PRODUCTION_CHANGE = NO
TEST_CHANGE = NO
T_EXPENSIVE_GATE_RERUN = NO
I_EXPENSIVE_GATE_RERUN = NO
```

## 1. Frozen candidates

- Skiff：
  `c79aa31c55db1de58ae9160eef0fcbcf650ea493`
  （tree `17e32b11fa82db115f418ebaed8c7e4dfdfeefa3`）
- Internals：
  `14d3d2d0a7171a57fde6c5dd19b8d7eb4903ccca`
  （tree `d58b732d45354f0b01efc24285a20ec3464f1b72`）
- Skiff packages：
  `1dd97eb0a2a6d129a912d578e2977469b86c34b4`
  （tree `b5c87af353891ad71294e99d2a104dafbaa32455`）

验收开始和聚焦抽查结束后，三个integration worktree均clean。Skiff冻结提交以
`7eb8690589a89d2c8d0be66198e4f02327fed6c7`为parent，唯一新增内容是I task/result账本；
I最终GREEN使用的Skiff candidate
`8e30f514caa3f219f4a77452684359d4a5ddbdd5`
是本次冻结提交的ancestor。

## 2. Acceptance matrix

| # | 结果 | 独立验收证据 |
| --- | --- | --- |
| 1 | PASS | P8写集只组合既有`std.http`、test service、Router/Runtime effect与stream能力。反向搜索和K/H/R/T、S2/S3写集核对未发现新增标准库、语言、File IR、wire/schema、test session或测试专用header机制。 |
| 2 | PASS | test-runner只投影test service显式声明的`http.yml`；精确聚焦用例证明不会自动复制subject ingress。 |
| 3 | PASS | T在隔离Router business port上以动态绝对URL、现有service/version和method/path完成真实wrapper入口；I以同一入口模型完成AIHub迁移。 |
| 4 | PASS | H与Host聚焦用例证明entry child借用parent case registry、child不finalize；重复begin不替换parent，parent保持唯一finalize责任。 |
| 5 | PASS | R证明Router production仍走普通business route，Host仅根据已经完成的目标解析识别精确self ingress；`Host`请求头不参与路由。 |
| 6 | PASS | runner/Host聚焦用例覆盖保留配置或header覆盖、缺失或非法business origin、非self origin、同一case重复或并发child；精确activation identity与单活动入口令cross-case请求fail closed。 |
| 7 | PASS | T覆盖raw HTTP stream break/cancel；I的consumer-break case覆盖provider ancestor取消。实现复用普通stream lease、cancel与backpressure，没有入口专用取消通道。 |
| 8 | PASS | T/H证据与runtime代码核对表明HTTP child使用独立stream registry；handle只在child当前runtime依据effect wire snapshot生成，不复用parent heap handle。 |
| 9 | PASS | S1三次保留普通wrapper到dependency `PackageDirect` return-stream的真实GREEN诊断；其结论继续是`TASK_NOT_EXECUTABLE`、`S1_COMPLETE=NO`，未伪造成production closure。 |
| 10 | PASS | S2在同一HTTP child中闭合overlay-local producer作为dependency `PackageDirect`参数时三个stream的registry/generation、create/lookup、normal/error/cancel/finish与executable轨迹；跨heap item只使用既有`StreamInternalItem`/wire。 |
| 11 | PASS | S3只把raw HTTP gateway既有response sink附到精确deferred producer环境；HTTP context外探针fail closed，service stream仍通过既有boundary materialization。 |
| 12 | PASS | I账本在冻结Internals candidate上报告default suite `51/51`；四条迁移case分别断言完整HTTP body、完整SSE event序列、post-start error前已发event及consumer break取消，不依赖网络chunk边界。 |
| 13 | PASS | 对P8新增写集逐层反事实核对：删除K会失去显式test-service入口与动态origin；删除H会失去case effect桥；删除T会失去真实入口证据；删除S2/S3会分别失去producer-argument闭环与deferred sink传递；删除I会失去AIHub真实迁移。没有一个额外协议或状态机制可删除而仍保持全部验收项，因为这类机制根本没有新增。 |

## 3. Focused checks

在冻结Skiff candidate上执行：

```text
cargo test --locked -p skiff-test-runner \
  --test package_service_contract_deployment http_entry -- --nocapture
```

结果：`1 passed; 0 failed`。覆盖单并发要求与保留配置覆盖拒绝。

```text
cargo test --locked -p skiff-test-runner \
  --test package_service_contract_deployment business_ingress -- --nocapture
```

结果：`1 passed; 0 failed`。覆盖缺失或非法business ingress在网络前拒绝。

```text
cargo test --locked -p skiff-test-runner \
  --test package_service_contract_deployment \
  explicit_test_service_http_entries_are_projected_per_case_without_subject_ingress \
  -- --exact --nocapture
```

结果：`1 passed; 0 failed`。

```text
cargo test --locked -p skiff-runtime-host \
  capability_context::test_http_entry::tests -- --nocapture
```

结果：`3 passed; 0 failed`。覆盖精确selector、顺序slot释放、错误origin、大小写不敏感的
保留header拒绝以及重复begin不替换parent。

```text
cargo test --locked -p skiff-runtime-host package_direct -- --nocapture
```

结果：`4 passed; 0 failed`。覆盖S1保留诊断、S2 producer-argument三stream链和S3 deferred
raw HTTP response sink。

上述聚焦抽查只访问本地源码和构建缓存；未启动stable instance、共享Mongo、browser、OAuth，
未读取secret、未访问外网。按X合同复用T的一次真实隔离入口证据和I的最终`51/51`账本，没有重复
运行两条昂贵gate，也没有运行J behavior gate。

## 4. Blocking issues and follow-up

Blocking issues：无。

Non-blocking follow-up：无。S1的`TASK_NOT_EXECUTABLE`是明确保留的诊断边界，不是P8-X残留任务。

## 5. Residual risk

本次没有在最终Skiff文档冻结提交上重跑T与I完整隔离gate；这是X合同要求的证据复用。残余风险限于
未被上述聚焦抽查捕获的跨组件回归。冻结提交相对其直接parent只有I账本变化，且相关Host、
test-runner与`PackageDirect`聚焦检查均GREEN，因此该风险不阻塞P8-X。
