# P5-F327 Service error core independent acceptance

状态：Completed。结果见
`P5-F327-service-error-core-independent-acceptance-result.md`。

## 验收输入

- 唯一权威架构：
  `doc/architecture/package-service-contract-deployment.md`，重点6.3 service error channel；
- 语言/runtime约束：
  `doc/reference/runtime.md`、`doc/reference/std-surface.md`、`doc/reference/static-semantics.md`；
- current owner/delta：
  `P5-F319-service-error-channel-delta-audit-result.md`；
- 开发节点与共享API：
  - `P5-F321-imported-service-exception-cause-result.md`
  - `P5-F322-selected-service-value-codec-result.md`
  - `P5-F324-canonical-service-error-channel-core-result.md`
- 合流探针：
  `P5-F326-service-error-core-combined-probe-result.md`。

先完整读取本任务和上述输入；它们冲突时以唯一权威架构为准。

## 精确候选

- production candidate commit：
  `49d9ab300f331f7662abfe8e6a0345f93c97f816`
- production candidate tree：
  `ec596389abb5583fcfffc198205a657df5d4f616`
- 验收worktree从包含F326 result的integration HEAD创建；必须证明从production candidate到验收HEAD在
  `runtime/model/**`、`runtime/boundary/**`、`runtime/eval/**`没有production diff，否则停止。

风险：最高，共享service error wire/value/identity/stack checkpoint。Verdict只针对R0 core；R1 ordinary、
R2 stream与R3 test effect尚未接线，不能把R0 PASS写成A5或Phase 5 PASS。

## 只读边界

只读检查production、tests和既有证据。唯一允许写入
`P5-F327-service-error-core-independent-acceptance-result.md`并提交。不得修改代码、fixture、设计，不运行
完整eval/workspace/root/stable/live，不push、不承接R1–R4。

## 必须独立判断

### 1. Canonical ownership

- fixed envelope、imported cause、selected codec、type index、platform identity、eval orchestrator各自恰好一个
  owner，依赖方向没有cycle；
- R0 API被production编译和真实调用父checkpoint，不是只在tests重新实现；
- core不会按operation error set、display/name/shape/static throw/message/code猜类型；
- ResourceError没有进入platform allowlist。

### 2. Export/import语义

- 任意自定义名义类型可local throw；package内部不要求可序列化；
- public + exact owner + SchemaClosed + encode成功保留实际Package owner，包括dependency-owned error；
- private/non-nameable/nonclosed/encode failure在第一次出界生成一个可序列化
  `std.service.InternalError`，不泄露原type/字段/display；
- exact local Internal进入fixed Internal；imported Internal/public/platform未处理时逐字节forward，不重复包装
  或换`traceId/errorId`；
- caller exact link恢复本地名义值并可catch；无exact edge时保持opaque且catch miss；不按assembly里其它build
  或同package id误materialize；
- named union exact ordinal、representation root、owner/key/type id/build/branch/payload mutation严格失败；
- malformed inbound、artifact/index损坏不能用Internal掩盖。

### 3. Stack与隐私

- 每个local throw有request-local source/stack/correlation；same-request rethrow保持；
- provider stack scope不继承caller local frame；
- remote import创建本service自己的新stack，只加入安全
  service/operation/errorId frame；callee stack/source/path/function不出界；
- B调用A收到Internal后不catch：B持同一错误值但有B的local stack；B向外原bytesforward；下一caller再建新栈。

### 4. Platform与Internal payload

- platform payload codec由enum identity选择并严格验证，不能从payload反推identity；
- fixed Internal message与correlation唯一、安全；用户新抛的exact Internal不能造成双层或信任任意
  display/private字段；
- std row缺失/错配失败关闭；Internal在linked caller中是普通exact可捕获名义值。

### 5. 结构质量

production core约1157行、tests约1435行。按workspace规范明确给出结论：

- 若职责边界清晰、内部helper无复制、只有一个canonical owner且物理拆分只是可读性改进，列为non-blocking；
- 若graph/index、codec、classifier、stack已形成相互独立且混杂的owner，或重复模式使R1–R3接入会继续复制，
  列为blocking并给出最小拆分边界；
- 不因行数本身机械FAIL，也不能忽略长文件审查。

## 独立证据

不得只复述F326。至少抽查production调用链、构造两个关键mutation或运行最窄现有selector；不重复所有昂贵
证据。可运行：

```bash
cargo test -p skiff-runtime-eval --lib assembly_execution::service_error_channel --no-fail-fast
cargo test -p skiff-runtime-model --lib imported --no-fail-fast
cargo test -p skiff-runtime-boundary --lib selected --no-fail-fast
git diff --check
```

selector必须非零。记录任何未运行证据及原因。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f327-service-error-acceptance`
- branch：`codex/p5-f327-service-error-acceptance`
- 返回`PASS`或`FAIL`、blocking issues、non-blocking follow-up、独立证据、结构判断和残余风险；
- PASS只冻结R0 API并解除R1/R2/R3，不代表A5；
- result提交，不push、不承接实现。
