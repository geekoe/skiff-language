# P4-R03：Entry / Remote-Retirement Acceptance

## 角色与精确输入

高风险只读验收Agent。输入为权威设计§6.2、§7、§12、§14、§15，`phase-plan.md`，P4-T07/T08/T09任务合同，
三任务已合流exact clean commit，以及R01/R02与开发证据。不得修改或预设PASS。

## 必验 verdict

1. **UNIFIED_ENTRY**：active-generation context set原子发布；wire严格投影selector；ingress/internal命中同一
   dispatcher；request无legacy route/artifact I/O fallback。
2. **NO_REMOTE_SERVICE_RELAY**：runtime canonical service call不发router帧；router在registry/lazy/forward前拒绝
   service caller，同时gateway、actor/spawn不回归。
3. **STRUCTURE_GATE**：checker扫描真实owner，mutation覆盖rename/move/duplicate/omission/test-only、TLS、shared/
   recoverable callback、host fallback和router relay。

按需运行便宜聚焦抽查，不重复开发完整命令。确认T07/T08无跨owner隐式补丁，T09不通过allowlist掩盖production。

## 输出

首行总体`PASS`或`FAIL`，分别给出三个verdict、blocking issues、non-blocking follow-up、证据命令、动态缺口和残余
风险。PASS才允许T10冻结候选；结论锚定exact commit。
