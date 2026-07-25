# P5-F335 Restricted service diagnostic acceptance

状态：Ready（独立只读验收）。

## 验收输入

- 唯一权威设计：
  `doc/architecture/package-service-contract-deployment.md`，重点 6.3；
- wire/observability owner：
  `P5-F333-wire-observability-delta-audit-result.md`；
- 被验收任务与实现证据：
  - `P5-F334-restricted-service-diagnostic-handoff.md`
  - `P5-F334-restricted-service-diagnostic-handoff-result.md`
- 已冻结的下层 error channel：
  `P5-F332-service-error-channel-a5-acceptance-result.md`。

## 精确候选与边界

- 候选 commit：`a4bd73be4fa59dd20937aabd9ccd6519cda1d138`
- 候选 tree：`fcac3e734101e7f085167b21a19f03e54f6a5639`
- 合流探针：候选上
  `restricted_service_diagnostic_ordinary_three_hop_preserves_bytes_and_local_stacks`，1/1 PASS。
- 风险：高，provider heap 生命周期、逐跳栈与受限信息边界。

只读 production/tests。唯一允许写入
`P5-F335-restricted-service-diagnostic-acceptance-result.md`并提交。不得修实现、fixture或设计；不得修改
task 状态，不运行完整 eval/workspace/root/stable/live，不 push，不承接 F336。

## 必须独立判断

1. typed diagnostic value只表达当前 provider owner、最终 correlation、typed source/local stack和有限 cause；
   没有 serde/generic JSON、payload/display、heap handle、`RuntimeValue`、`TypeAddr`或 external response
   surface。
2. sink seam与用户 `emit_native`彻底分离；默认 discard只用于 F336/H 尚未接线的实现检查点，不存在静默绕过
   production eval submit 的第二路径。
3. ordinary、async unary、server stream与 `ContractOperation`都在 provider heap销毁前调用同一个 wrapper；
   success/cancel/control/`PackageCallable`为零，且每 hop至多一次。
4. wrapper先取得最终 fixed envelope，再复制 correlation/kind并 best-effort submit；sink失败不能替换或修改
   原 error，不能按 message/code分类。
5. imported fixed/Internal继续 byte-for-byte forward；当前 B 的 diagnostic只用 B 的 imported local
   exception stack，不带 callee stack，不生成新 correlation或新 Internal。
6. private/nonclosed/encode failure的原 type、值、display/path/function不进入 fixed bytes或 safe
   diagnostic fields；完整本地 stack只留在 typed diagnostic。
7. 旧 ordinary spy与 lane-specific record shape已经删除；新增实现没有第二 error classifier、
   envelope codec、telemetry DTO或跨层 owner。
8. F336/H 尚未把默认 sink接到 host restricted telemetry必须列作边界内残余工作，不能把它误判为 F334
   blocker；若 typed seam本身无法支持该接线，则应 FAIL。

## 独立证据

- 逐段阅读 capability value/sink、R0 additive wrapper及四类 lane真实调用点；
- 反搜旧 spy、重复 record/classifier、generic JSON与 fixed bytes重编码；
- 不重复合流的 ordinary三跳探针；至少独立抽查一个 server-stream failure/cancel pair、一个
  `ContractOperation`/`PackageCallable` pair，以及一个 failing-sink exact-bytes负例；
- selector必须先列出且非零；只运行最小测试，不机械重跑 F334 全部命令；
- 核对候选相对 F334 implementation commit之后，受验 production无额外变化。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f335-restricted-diagnostic-acceptance`
- branch：`codex/p5-f335-restricted-diagnostic-acceptance`
- 新的一次性独立验收 Agent；
- 返回 PASS/FAIL、blocking、non-blocking、独立证据、残余风险和 result commit；
- PASS只冻结 RΔ并解除 F336 shared wire/telemetry checkpoint，不代表 W2-W/A6/Phase 5。
