# P5-F445H I7 P8 K Test-runner HTTP entry closure

状态：

```text
IMPLEMENTED
READY_FOR_INTEGRATION
```

## 1. Parent, baseline, DAG

- 直接父节点：
  `P5-F445H-I7-P8-D0-http-entry-test-authority-result.md`
- 实际Skiff baseline：
  `45a89dc40dd2f4cffc19296acc9a31065fcc3a37`
  （tree `e67bfc6553b9a59797b04a4722768ee765529947`）
- DAG：`D0 -> K -> T`
- 并行兄弟：H拥有Runtime/Host/Eval；R拥有Router审计。K不得修改这些表面。
- integration owner：`/root/phase05_integration_steward`

## 2. Goal and ownership

K只拥有test-runner最短闭环：

1. 从现有isolated host的`--ingress-url`/等价显式输入取得真实business ingress URL，不猜固定端口；
2. 用现有resolved config机制注入保留只读`skiff.test.ingressUrl`，拒绝authored同名binding；
3. 每个case继续使用现有唯一synthetic service id与contract version；
4. 将test service显式`http.yml` entry及其`*.test.skiff` wrapper投影进每个case的普通deployment/
   assembly；不自动复制subject ingress；
5. 把当前case精确selector和ingress origin交给既有test execution context，不能新增runtime
   frame/schema/header。

预期写集：

```text
test-runner/src/canonical_package.rs
test-runner/src/package_test_assembly.rs
test-runner/src/runtime_execution.rs
test-runner/src/runtime_execution/**
test-runner/src/lib.rs
test-runner/src/main.rs
test-runner/tests/**
```

实际预检若证明更小写集即可，必须删减。compiler、std、File IR、Router和Runtime生产代码禁止修改。

## 3. RED / GREEN

RED必须在baseline证明至少一个真实缺口：显式test-service HTTP entry未进入case deployment、动态
business ingress没有进入resolved config，或runner未把exact case selector交给test execution。
已有行为若全部满足则返回`NO_PRODUCTION_CHANGE`，只记录证据。

GREEN最小证据：

```text
cargo check --locked -p skiff-test-runner --tests
cargo test --locked -p skiff-test-runner --no-fail-fast
git diff --check
```

必须有负例覆盖缺失/非法ingress URL、authored保留config覆盖、subject ingress自动投影不存在、
两个case的service id不相同。不得运行完整gate、stable/live/network/Mongo/OAuth/browser。

## 4. Stop conditions

若闭环要求compiler/std/File IR改动、第二套artifact、runtime wire/schema、特殊URL、session/token/header、
或与H/R共享production owner，返回`TASK_SCOPE_EXPANDED`。不得在K内发明替代机制。

实现与验证结果见
`P5-F445H-I7-P8-K-test-runner-http-entry-closure-result.md`。
