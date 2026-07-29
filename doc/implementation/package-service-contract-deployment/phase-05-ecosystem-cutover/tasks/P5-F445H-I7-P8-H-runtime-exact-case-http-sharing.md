# P5-F445H I7 P8 H Runtime exact-case HTTP sharing

状态：

```text
READY_FOR_ZERO_WORKTREE_PREFLIGHT
PRODUCTION_CHANGE_REQUIRES_RED = YES
```

## 1. Parent, baseline, DAG

- 直接父节点：
  `P5-F445H-I7-P8-D0-http-entry-test-authority-result.md`
- baseline：
  `3a87d37f81a04c249f308b311bd91dcfdf3a8aa3`
  （tree `eafc29e952f6b5170e4f5faca4e5d181b3ace9f6`）
- DAG：`D0 -> H -> T`
- K拥有test-runner；R拥有Router。H不得修改compiler/std/File IR、test-runner或Router。
- integration owner：`/root/phase05_integration_steward`

## 2. Bounded preflight

先用现有`testEffectsEnabled`父dispatch、精确deployment/generation、Interpreter registry、
HTTP native dispatch与request finalization路径回答：

1. self-ingress是否已经在inline effect前可区分；
2. 普通HTTP client是否已经能从当前test case取得ingress origin与service/version；
3. Router回送的普通HTTP request是否已复用同一case registry；
4. 子请求是否错误触发`finalize_test_case`；
5. stream drop/break是否已释放active request并沿现有取消链收束。

每个production修改必须对应一个可复现RED。没有RED的表面保持不变。

## 3. Goal and write ownership

只闭合被RED证明的Runtime test-execution适配：

- origin精确等于runner注入ingress URL时，在double匹配前识别self-ingress；
- 自动加入当前case现有service/version selector，并在发送前拒绝用户覆盖selector、Host、
  content-length、transfer-encoding和hop-by-hop headers；
- 按exact case deployment/generation附着父inline-effect registry，不使用全局或task-local side
  channel；
- 子请求不finalize，父case唯一finalize；第一版同case一个active self-ingress；
- unary结束、stream EOF/error/drop/break均释放active slot并复用现有HTTP cancel/backpressure。

预期owner：

```text
runtime/request/**
runtime/eval/src/test_effect_registry.rs
runtime/eval/src/**http**
runtime/host/src/**http**
runtime/host/src/**request**
```

预检必须缩到最小文件。禁止新增公共API、wire/schema、header种类、Router路由或compiler/std/File IR改动。

## 4. Evidence

RED/GREEN聚焦测试必须覆盖：父double由entry内部消费、子不finalize、未消费double仍由父报错、第二个active
请求失败、顺序两个请求成功、stream break取消、非self origin仍走普通double、大小写变体保留header被拒绝。

建议命令由预检按实际crate收敛，最多包含受影响Runtime crates的聚焦`cargo test`、`cargo check`、
`cargo fmt --all -- --check`和`git diff --check`；不得先跑workspace/full gate。

## 5. Stop conditions

需要session header/token、Router test route、runtime frame字段、schema bump、并行case共享registry、
production威胁模型或第二套HTTP client时，返回`TASK_SCOPE_EXPANDED`并附RED链；不得继续实现。
