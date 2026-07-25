# P5-F287 Std error surface migration

状态：Ready for contract review。

## 直接父节点与权威链

- 直接父结果：
  `P5-F280-open-service-error-channel-implementation-audit-result.md`
- 设计父结果：
  `P5-F279-open-service-error-channel-design-result.md`
- F279引用唯一架构、静态语义、std与runtime事实源。

启动时只读本任务；需要依据时沿父链向上读取。

## DAG位置、基线与并行边界

- 节点：F280 W2 language/std中的std、prelude与tooling consumer。
- 开发production base：`7045644f49c739510365aa33520f9da3d3f9e399`。它保留可编译的pre-A1 compiler
  consumer；本任务完成后合入当前已冻结A1 strict model的integration。
- 本任务不消费或修改F281 shared DTO，因而可以在pre-A1代码上独立验证。
- 并行节点：
  - F285只修改package-call type resolution与package_imports；
  - F288修改artifact/contract consumer；
  - 后续W2 language task不得修改本任务列出的prelude registry、marker与std/tooling owner。
- 完成后解除：W2 language combined compiler probe与runtime platform identity consumer。

## 唯一production写入范围

- `compiler/core/src/prelude_registry.rs`
- `compiler/source/src/semantic/interface.rs`，仅删除compiler-known`ErrorPayload` interface owner
- `prelude/config.skiff`
- `std/{bytes,db,file,http,json,number,resource,service,time}.skiff`
- `std/api.yml`
- `scripts/check-skiff-source-layout.mjs`
- `vscode/syntaxes/skiff.tmLanguage.json`
- `vscode/scripts/test-grammar.mjs`
- `runtime/live-tests/internal/db_live.live.test.skiff`，仅删除marker

允许迁移以下test-only native stub/marker fixture：

- `compiler/driver/authoring/package_publication/tests.rs`
- `compiler/driver/authoring/tests.rs`
- `compiler/driver/pipeline/tests/p5_f18a.rs`
- `compiler/input/src/package_sources/tests.rs`
- `compiler/input/src/platform_sources/tests.rs`
- `compiler/source/src/prelude_registry/tests.rs`
- `compiler/source/src/contract_type_resolution/tests/interface_signatures.rs`
- `compiler/tests/package_std_schema.rs`
- `test-runner/src/canonical_package/tests.rs`
- `test-runner/src/canonical_package/tests/combined.rs`

禁止修改compiler throw/catch checker/lowering、artifact model/identity/projection、runtime error registry/channel、
router、telemetry、internals或skiff-packages。

## 完成标准

1. 删除compiler-known/bare prelude `ErrorPayload -> std.error.ErrorPayload`及source注入的空marker interface。
2. 所有Skiff std/prelude声明删除`implements ErrorPayload`；原有错误仍是同一个名义`type`，字段与public
   path不因删marker改变。
3. 删除不存在的bare/prelude `InternalError -> std.error.InternalError`；不增加旧路径alias、root spelling或
   compatibility。
4. 在`std/service.skiff`新增普通名义record：

   ```skiff
   type InternalError {
     message: string,
     traceId: string,
     errorId: string,
   }
   ```

5. 在`std/api.yml`以`service.InternalError`显式公开，使它按普通Package public schema规则获得
   `PublicNameable + SchemaClosed` identity。不得把它声明成prelude native、marker或generic runtime
   `"InternalError"` code的alias。
6. `std.resource.ResourceError`保留为public普通名义错误；本任务不把它加入platform fixed allowlist。
7. source-layout checker、VSC grammar与测试示例不再要求/突出`ErrorPayload`；测试示例改用普通名义错误，
   不能用primitive/anonymous error绕过新语言规则。
8. Skiff production/fixture反向搜索只允许：
   - reference/architecture中明确说明“不存在ErrorPayload”的文字；
   - implementation历史记录；
   - TypeScript业务DTO名称如`ChatErrorPayload`；
   - generic Rust trait/type名`WirePayload`或函数局部命名，不是语言marker。
9. 不修改internals的十个marker声明；它们由独立跨仓consumer在语言consumer合流后迁移。

## 验证owner

本任务唯一拥有：

```bash
cargo test -p skiff-compiler-core prelude_registry
cargo test -p skiff-compiler-source semantic::interface
cargo test -p skiff-compiler --test package_std_schema
node scripts/check-skiff-source-layout.mjs
node vscode/scripts/test-grammar.mjs
git diff --check
```

先用`--list`确认Rust selector非零；实际module filter不同则使用最小真实等价命令并报告。若某个test因与本任务
无关的A1下游尚未迁移而不能运行，必须证明pre-A1 base上的真实原因或换用更窄owner test，不能扩大修改范围。
不得运行workspace/full compiler、生态publish、instance、live或chat smoke。

## 风险、非目标与交付

- 风险：中；验收组`A2-std-surface`，后续由W2 language combined probe覆盖。
- 不实现runtime InternalError转换、fixed envelope、catch registry、stack、trace或telemetry。
- 不改变任何错误的业务字段，只有新增`std.service.InternalError`与删除marker conformance。
- 不为历史artifact/source加compat。
- worktree：`/Users/geek/workspace/skiff-p5-f287-std-errors`
- branch：`codex/p5-f287-std-errors`
- 不push，不操作stable。
- 从启动到第一次production修改不超过5分钟；不可执行时返回`TASK_NOT_EXECUTABLE`。
- 完成后提交并返回commit、反向搜索、测试矩阵与设计缺口；不得自行承接runtime或internals节点。

