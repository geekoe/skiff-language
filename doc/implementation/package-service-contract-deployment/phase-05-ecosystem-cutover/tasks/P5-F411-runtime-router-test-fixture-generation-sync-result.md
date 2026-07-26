# P5-F411 Runtime, Router and test fixture generation sync result

状态：Complete。owned implementation 已迁到 PackageArtifact v8 / package build v9；声明为并行
前置的 F408 compiler consumer 与 F410 deployment consumer 尚未合流，因此依赖它们的 Rust 与
compiler-generated fixture 命令已实际执行但停在范围外编译错误，留给 integration gate 重跑。

## 1. 提交锚点与范围

| 锚点 | commit | tree |
| --- | --- | --- |
| 任务规定 start | `288a105fc87399c5e93228ee9f2ba2e58c4cd2b6` | `4688200acf69afe8778b06189c545e06d49d7212` |
| task definition / clean worktree start | `97f3f831b02507a8caa1f831c590ea044655f895` | `9a208b775f009a9795aaaf7714dae16e1f2d3d25` |
| implementation end | `329fdf586c994389440460b2117bce7f8e42eacf` | `e9b28c47a667b872581cb2f3350da6efb9703f68` |

实现提交：

```text
329fdf586c994389440460b2117bce7f8e42eacf
fix(runtime): sync package service consumers to v8
```

实现提交共修改 40 个文件，全部位于任务独占范围。没有修改 artifact model/identity、
compiler production、deployment、ecosystem repo 或权威设计；没有 merge、rebase 或 push。

## 2. 精确候选移植映射

开始前逐个执行 forward/reverse patch check；七个候选均不是当前 tree 的完整
patch-equivalent，且候选修改文件全部属于本节点 owner。按候选顺序使用 no-commit
cherry-pick，再只保留仍有效的 owned hunk：

| candidate | 新 model 中保留的 hunk |
| --- | --- |
| `89ffbeca41ef2c60ae754abd58a155fd2b72ac70` | `canonical_test_gateway`、package-test assembly、runtime execution/wire 与 overlay 的 T1 基础实现 |
| `4eccda2bea14fa2aa04bb3f2e0a2cdd3d788d85b` | orchestration/wire negatives、overlay checks 与 package-service contract deployment 覆盖 |
| `804f2f9cf74e1ec14fc89b9a18ab20939b7e7e4d` | HTTP smoke oracle/fixtures、新 Node fixture、四个 private wrapper、smoke fixture binary、zero-operation gateway 与 T2 路径 |
| `03e8192387016b82fd5af8e376b03bb90b1d7ff3` | canonical package/std、overlay 与 integration test 修复；旧 `api.yml {source, serviceCall}` hunk不保留，改为 scalar API + `service.yml.serviceCalls` |
| `a9de53ee6194703e429bc946920d4a2ef3db2d28` | bounded sanitized isolated evidence renderer与 4-case Node owner test，原样保留 |
| `8ad4db47b79581e4264148f61eadd084d00feb13` | 当前完整 startup mock seam，原样保留 |
| `593dcfe1edb741adf5afc66d965183b5a4712769` | F405 exact-coordinate resolver与 1 positive / 8 negative tests；只删除 v8 model 已不存在的 fixture field |

最终 implementation 与
`codex/p5-f386-package-test-http-gateway` 的上述候选-owned代码相比，只剩六个文件上的机械新模型差异：

- linker fixture 删除 `service_call_roots`；
- smoke oracle/helper 的 build prefix 改为 v9；
- provider `api.yml` 恢复 scalar，并在 `service.yml` 增加 `serviceCalls: [echo]`；
- canonical std exact build pin重算为
  `skiff-package-build-v9:sha256:8ac1d3ee235fb3f543df52430f1539610ca05c5631a09df22f7c4f4a7b6a8e17`。

没有移植候选的 compiler patch，也没有重复移植已经在 integration 中存在的 Router
patch-equivalent commit。

## 3. v8 / v9 consumer 结果

### 3.1 Runtime

- owned `PackageArtifact` literals全部删除 `service_call_roots` 与
  `PackageServiceCallRoot` import；
- `service_call_refs`、FileIR call-site dispatch、linked
  `package_callable_id` 与 call-site execution均保留；
- Host 三个 `ServiceDeploymentOperationInput` fixture从 public path改为 exact
  `PackageCallableId`，没有新增 path resolver、selection reader或 fallback；
- F405 `service_error_index.rs`与精确候选相比只有一行 v8 fixture field删除，wrong
  package/file/type、missing/ambiguous/tamper与 `boolean` strict negatives没有改写或弱化。

### 3.2 Router

- filesystem loader只接受 `skiff-package-build-v9:sha256`；
- package record显式要求 `schemaVersion === skiff-package-artifact-v8`；
- 新聚焦 test同时覆盖 v8/v9 success、v7 schema rejection与v8 build rejection；
- compiler compatibility fixture断言 v8、v9，并断言 `serviceCallRoots` 不存在，不做 dual-read。

### 3.3 Test runner、fixtures与 evidence

- provider API精确为 `echo: main.handle`，service manifest精确为
  `serviceCalls: [echo]`；已有 integration assertion仍要求 contract恰好一个 operation；
- 其余三个 owned test service manifest均缺失 `serviceCalls`，zero-operation contract、
  deployment operation bindings与HTTP gateway separation assertions原样保留；
- T1/T2 canonical gateway、strict control、inline setup、top-level package dependency、
  private HTTP wrapper与新 source fixture均保留，没有恢复旧 test-double写法；
- `run-skiff-tests` bounded sanitized evidence、4-case error evidence test与 3-case完整 startup
  mock test保留；
- package-service authoring Node fixture改为 scalar `api.yml`与
  `service.yml.serviceCalls: [ping]`，没有修改 F408 parser。

canonical std build pin不是仅替换前缀：用当前 canonical std source生成候选 v7 artifact，删除
empty selection并交给当前 v8 identity assignment重新计算，得到上述 v9 hash；Local ABI generation
仍为 v6。

## 4. 反向搜索

在任务全部 owned 路径执行：

```text
PackageServiceCallRoot | service_call_roots
=> 0

package_public_path | packagePublicPath | serviceCall:
=> 0

skiff-package-artifact-v7 | skiff-package-build-v8:sha256
=> 2 occurrences，均为 Router strict rejection test

skiff-package-artifact-v8
=> 3 files

skiff-package-build-v9:sha256
=> 6 files

service_call_refs | serviceCallRefs
=> 13 files
```

`test-runner/fixtures`共有 4 个 `service.yml`；只有 provider带 non-empty
`serviceCalls`，其余 3 个保持 missing selection。

## 5. 聚焦验证与实际计数

所有 Rust命令均使用共享 target：

```text
/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

| 命令 | 实际结果 |
| --- | --- |
| `cargo test --locked -p skiff-runtime-linker service_error_index` | compile blocked；0 tests executed；F410 `deployment/**`仍读取 `package_public_path` |
| `cargo test --locked -p skiff-runtime-loader` | compile blocked；0 tests executed；同一 F410 blocker |
| `cargo test --locked -p skiff-test-runner --lib runtime_execution -- --test-threads=1` | compile blocked；0 tests executed；F408 compiler selection readers + F410 deployment |
| `cargo test --locked -p skiff-test-runner --test package_service_contract_deployment -- --test-threads=1` | compile blocked；0 tests executed；同一 F408/F410 blockers |
| `node --test scripts/tests/run-skiff-tests-error-evidence.test.mjs` | 4 passed / 0 failed |
| `node --test scripts/tests/isolated-test-runtime-log-evidence.test.mjs` | 3 passed / 0 failed |
| `node --test scripts/tests/package-service-ecosystem-http-fixture.test.mjs` | 4 passed / 0 failed |
| `pnpm --filter @skiff/router exec vitest run tests/filesystem-runtime-assembly-snapshot-loader.test.ts` | 26 passed / 0 failed |
| `pnpm --filter @skiff/router test:manifest-compatibility` | 0 passed / 1 failed；在 assertions前被 F408/F410 compile blocker截断 |
| Router generation-file strict `tsc` | PASS；loader + 两个 generation tests及其 import closure |
| `pnpm --filter @skiff/router type-check` | FAIL；diagnostics全在未修改的 WebSocket/ingress files，且这些 files与精确候选 tree无差异 |
| authoring Node `service package build returns one stable API receipt with operation identities` | 0 passed / 1 failed；在 fixture执行前被 F408/F410 compile blocker截断 |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

Node owner tests合计 11 passed / 0 failed。Router可执行的 generation loader tests为
26 passed / 0 failed。没有把 compile blocker、零测试或 F405父节点的旧 9/9证据冒充当前
implementation通过。

包级 Router typecheck的既有错误位于
`assemblyWebSocketGateway.ts`、`assemblyRuntimeRegistry.ts`及相应 WebSocket tests；本节点未修改
这些文件，精确候选 branch中的内容也相同。聚焦 generation `tsc`使用与 package
`tsconfig.json`相同的 strict、`noUncheckedIndexedAccess`和
`exactOptionalPropertyTypes`选项，执行成功。

## 6. 尚待 integration 解除的遮挡

- F408需删除 compiler的 `PackageServiceCallRoot` /
  `service_call_roots` reader，并把 manifest selection切到 `serviceCalls`；
- F410需把 deployment projection及fixtures从 `package_public_path`切到
  `package_callable_id`；
- 两者合流后必须重跑四条 Rust命令、Router manifest compatibility与authoring focused test；
- F409及最终 F408/F409/F410/F411组合 gate仍由 integration owner执行；
- 本节点按任务规定没有运行完整 `node scripts/run-skiff-tests.mjs`，也没有操作 stable/live。

## 7. 自验收矩阵

| 条款 | 代码 / 搜索证据 | 验证 | 结论 |
| --- | --- | --- | --- |
| owned artifact literals切 v8/v9 | roots exact identifier为0；v9分布6 files | fmt + diff check | PASS |
| Runtime不新增selection reader | roots/path search为0；call refs仍13 files | candidate exact diff | PASS |
| exact deployment binding保留 | Host三个 input均为 `package_callable_id` | Rust gate待F410 | IMPLEMENTED / UPSTREAM-BLOCKED |
| Router v8/v9 only、无dual-read | source schema/prefix gate + legacy negatives | filesystem 26/26；focused tsc PASS | PASS |
| F405 exact coordinate语义不放宽 | candidate文件仅删除1行roots fixture | current Rust gate待F410；父证据不复用 | IMPLEMENTED / UPSTREAM-BLOCKED |
| provider manifest selection | scalar API + `serviceCalls: [echo]`；exact-one assertion | integration test待F408/F410 | IMPLEMENTED / UPSTREAM-BLOCKED |
| zero-operation gateway不进contract | 其余manifest missing；empty operation assertions保留 | HTTP Node 4/4 | PASS |
| T1/T2与evidence实现保留 | 七个精确候选hunk映射 | evidence Node 7/7 | PASS |
| authoring fixture使用新选择 | scalar API + service manifest selection | focused test待F408/F410 | IMPLEMENTED / UPSTREAM-BLOCKED |
| ownership与禁止项 | 范围外改动0；无compat/fallback；无stable/live | clean implementation commit | PASS |

结论：P5-F411 owned implementation完成；节点提交可进入 integration，不能在 F408/F410与最终
组合 gate完成前宣称全阶段 green。
