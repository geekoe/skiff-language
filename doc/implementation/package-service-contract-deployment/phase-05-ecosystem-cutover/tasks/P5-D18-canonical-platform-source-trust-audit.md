# P5-D18：Canonical Platform Source Trust Audit

## 角色与结论

输入为F15A合流后的`40ed693ebadd5af4a84e6704c0f94918c272fddc` / tree
`01f6b8d14c9dd3486cd9215c63a13fa222c60d72`。R15及12项combined package-service test均已PASS，随后唯一
F04原样gate在编译std前失败：`package id skiff.run/std is reserved`。D18由两个只读owner独立复现，结论为
`DESIGN GO` / `BLOCKER CONFIRMED`；不改源码、不作F04 verdict。

权威设计仍为`doc/architecture/package-service-contract-deployment.md` §3、§9、§10、§14：package compile
只接受显式source input，identity不可按display/path猜测，trust boundary必须fail closed。本审计只冻结compiler
platform source的实现owner和transport，不新增domain object、用户CLI语义或兼容路径。

## 根因与排除项

- `compiler/input`用`env!("CARGO_MANIFEST_DIR")/../../std`识别唯一official std，并从同类编译期路径读取
  `prelude/error.skiff`；`compiler/source::prelude_registry`又独立从编译期`std/prelude`建全局registry和identity。
- A worktree编译出的rlib内嵌A绝对路径；共享`CARGO_TARGET_DIR`下B报告该crate为`Fresh`并复用同一产物，B的
  canonical std因此被当作普通用户package读取并正确触发reserved-id拒绝。强制从B重建后，A/B结果镜像反转。
- A/B同commit/tree；cwd、absolute input argv、Cargo manifest、source registry和package/registry内容均指向B且
  bit-identical。故不是Node caller错误，也不是F14A/F15A回归。
- 清cache、隔离target、touch源码或放宽reserved id只能掩盖问题；共享Cargo target是受支持输入，必须保留。

## 冻结修复设计

唯一trust owner为`compiler/input::platform_sources::CompilerPlatformSources`：

- 可信launcher显式传入绝对platform source root；context在运行时canonicalize并验证`std/registry.yml`、`std/`
  与`prelude/`，没有`Default`、cwd、环境变量、executable location或`CARGO_MANIFEST_DIR` fallback。
- official manifest/source owner只能由该context根据registry成员与canonical路径授予；普通package root即使声明
  `skiff.run/std`仍拒绝。official source reader必须消费同一context并复验provenance。
- package compile显式消费该context。`PreludeRegistry`只由同一context初始化：同canonical root幂等，不同root
  fail closed；identity从已初始化registry/context计算，不再重读默认目录。
- compiler binary、test-runner、source-suite及Host fixture的内部transport都携带同一个由模块位置确定的绝对
  `skiffRoot`；不向用户级`skiff package|contract|deployment|assembly`命令暴露新的trust开关。

执行DAG为`F16A -> (F16B || F16C)`与并行D19；只有D19 `AUDIT CLOSED`，或其冻结的F17 exact repair已合流且
无在途写入，才能进入`I16 -> R16`。I16是共享target与F04原样Host gate的唯一动态owner；候选不变时该完整证据
直接交F04 narrow receive，不重复昂贵gate。

## 次生teardown异常

`Node FileHandle ... ERR_INVALID_STATE`发生在reserved-id主错误之后的cleanup/runtime early-exit阶段，不能归为主
blocker。缺少原stack；静态最可疑是supervisor exit handler对日志handle的fire-and-forget close与立即exit竞态。
D19使用便宜失败注入探针独立审计，禁止为此重跑F04；若可复现，再冻结单一幂等close owner的窄repair。
