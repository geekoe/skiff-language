# P1-T04：建立 Service Requirement 单一输入 Owner

状态：`ready`
类型：Compiler input / resolver
依赖：P1-T00、P1-T01、P1-T03
执行者：Compiler Input Agent，一份提交

## 目标

允许 `package.yml` 顶层声明 `services`，并把 package 与现有 service source manifest 的
declaration validation、alias binding、artifact-root resolution 和 protocol verification 收敛到
同一个 typed owner。

## Manifest 契约

Phase 01 冻结以下 shape：

```yaml
services:
  profile:
    id: skiff.run/profile
    version: 1.2.3
```

alias 是 map key；id/version 必填；version 必须精确。若现有 service manifest 已有语义等价的
canonical shape，应复用它，不另造 package-only 变体。最终拼写若与当前 parser 已冻结格式略有
差异，以“package/service 同一 declaration type”为最高优先级，并同步本文样例。

## 编译输入与 artifact 输出的区别

resolver从当前ServiceUnit或后续artifact中只抽取T00定义的 `ServiceProtocolContract` view。它可以
保留provider artifact path/build id作为本次编译的完整性证据，但必须分层：

```text
ResolvedServiceContract (compiler input evidence)
  = ServiceContractRequirement (可进入 PackageUnit)
  + resolved artifact location/build evidence（不可作为调用地址）
```

不得继续把现有含 build id 的 constraint 直接当作 PackageUnit service call requirement。

## 范围与 ownership

主要路径：

- `compiler/input-model/src/manifest.rs`、`dependencies.rs` 或新 shared manifest 模块
- `compiler/input/src/service_dependencies.rs`
- `compiler/input/src/package_config/`
- `compiler/input/src/service_config/` 中仅 declaration/resolution 共同部分
- compiler input tests/fixtures

若必须实质修改接近千行的 `service_config/validation.rs`，先把 service requirement validation
抽到 shared module，再让 package/service 调用；不能在长文件中再加 package 分支。

## 行为

- package/service declaration 使用同一 serde/input type 和 validator。
- alias 重复、保留 alias、非法 id、非精确 version、缺 artifact root、找不到版本、schema 不符、
  protocol/operation 不符均 fail closed。
- package source name resolution 获得 alias → typed service contract binding，但本任务不修改
  lowering/emission。
- artifact root 顺序和冲突处理保持 deterministic；多个不同 artifact 同时声称同一精确
  service version/protocol时不得静默选择。同一id/version出现不同protocol identity直接冲突。

## 非目标

- 不产生 effect 或 boundary projection。
- 不修改 runtime service resolution。
- 不支持 version range、optional service、动态 discovery 或 provider package id。
- 不执行 service call。

## 必须测试

- 同一 declaration fixture 被 package/service parser 接受并产生相等 typed declaration。
- package `services` 正例、未知字段、重复 alias、非法 id、非精确 version。
- 无 artifact root、缺 provider、冲突 provider、protocol mismatch。
- 编译输入 evidence 可以包含 build id，但导出的 `ServiceContractRequirement` 不包含它。
- 旧的“package 出现 services 必然失败”测试被明确删除并替换。

## 聚焦验证

```bash
cargo test --no-fail-fast -p skiff-compiler-input-model -p skiff-compiler-input
cargo test -p skiff-compiler-input service_depend
cargo test -p skiff-compiler-input package_config
git diff --check
```

## 验收标准

- `rg` 只能找到一个 service requirement declaration validator 和一个 artifact-root resolver owner。
- package/service 没有复制错误 taxonomy 或 alias/version 规则。
- PackageUnit 可用的 requirement 与编译 evidence 明确分型。
- 未增加兼容 reader 或 package-only manifest DTO。

## 停止条件

- package/service 当前语法无法共享而需要用户选择新 manifest shape；
- T00的具名operation surface或service version/deployment revision边界未同步到canonical文档；
- protocol expectation 必须依赖 provider package id/build id；
- resolver 冲突语义无法从现有 release/artifact identity 文档确定；
- 必须同时保留两套 production parser 才能通过阶段 gate。

## 提交

提交信息建议：`feat(compiler-input): support package service requirements`
