# P1-A01：Phase 01 独立验收

状态：`ready`
类型：只读阶段验收
依赖：P1-T12
执行者：未参与 T01–T12 的独立 Acceptance Agent

## 目标

从当前生产路径、任务契约和测试证据判断 Phase 01 是否形成可供 Phase 02依赖的 coherent
foundation。验收以方向成立、单一 owner、fail-closed和阶段可用为准，不追求更多美化。

## 只读约束

- 不修改代码、文档、测试或 git状态。
- 不替开发 Agent修 bug。
- 不只读 diff；沿真实 package compiler entrypoint追踪到 emitted PackageUnit，并抽查旧 service
  source path。
- 可以运行阶段 gate中的聚焦命令或更小 filter核实证据，不运行 live/full suite。

## 必查项

1. package `services` declaration进入单一resolver并产出无provider package/build address的typed
   requirement；service version/protocol identity与deployment revision分离，contract是具名
   operation surface。
2. 每个 public callable有 Local Code ABI、effect/link facts和显式 boundary状态。
3. mutable helper未被禁止，但 caller-reachable mutation/alias helper不可跨 boundary。
4. `any I`/native handle使用 request callback capability；ordinary boundary未被误要求
   recoverable。
5. identity owner唯一，diagnostic文本/path不影响 ABI identity。
6. effect analysis对递归/unknown call sound且 deterministic。
7. package build/test走真实生产 entrypoint；service dependency执行未被偷偷路由或 stub化。
8. 直接触碰路径没有新增重复实现、职责回并或超长文件继续膨胀。
9. 旧 service source path在 Phase 01仍 coherent，但不拥有第二套新规则。

## 宏观问题判定

以下为 blocking：

- 阶段产物无法被 Phase 02直接消费；
- source/projection/linker对关键语义各算一次；
- mutable local与boundary callable仍无法在artifact中区分；
- recoverable被错误用作普通service ABI门槛；
- package requirement依赖provider build/package identity寻址；
- Phase 01 requirement schema仍依赖Phase 02才决定的operation surface或deployment revision语义；
- 测试只覆盖builder/fixture，真实production path仍旧；
- 需要长期兼容/fallback才能工作。

以下默认 non-blocking：命名偏好、错误文案、更多边角case、未来remote wire细节、与当前数据流
无关的旧长文件。

## 输出格式

```text
Verdict: PASS | FAIL

Blocking findings:
- [文件:行] 事实、为何破坏阶段完成态、最小应回到哪个任务

Non-blocking notes:
- 仅记录确有价值且不阻止下一阶段的事项

Evidence checked:
- production path
- tests/commands
- single-owner and size audit
```

若 `FAIL`，只列会改变 verdict的阻塞项。协调 Agent把问题退回对应任务；不得让验收 Agent直接
修改。修复后重新做一次完整验收，仍有无法裁决的架构问题则询问用户，不无限循环。

## PASS 条件

`phase-plan.md` §1、§8 与上述九项均有可验证证据，且无 blocking finding。
