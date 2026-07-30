# P5-F420I Final N4 gate result

状态：`N4_PASS`。

```text
N4_PASS
F421_RELEASED
```

冻结候选上的全部 canonical 动态命令、identity checker、format/diff、反搜与 tracked
clean gate 均通过；唯一 test-runner ignored case 是任务预期保留的 I16/G16 shared-target
identity probe，不影响命令成功。F421 已解除。

## 1. Candidate、task checkout 与 ancestry

- executable candidate / tree：
  `0d33d26acf631184603d8bdc2c78a7ac67971392` /
  `e5961076f15a719bd755c8ac4e0445adf6eeae98`；
- task checkout / tree：
  `48021e1337ed5afa775caf4743cfa3d7f4129cd5` /
  `278e54fb1c155e48cd61ad938b53f203b21445c1`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

`git rev-parse HEAD^` 精确返回 executable candidate，candidate tree 与任务冻结值一致；
`git merge-base --is-ancestor <accepted-F415> <candidate>` 返回 0。task checkout 相对
candidate 只新增 `P5-F420I-final-n4-gate.md`。启动时 `git status --porcelain` 为空。

本 result 是 executable candidate 之后唯一新增的 tracked 交付；result-only final commit/tree
与提交后的 clean 状态由交付消息记录，避免在提交对象内声明无法自引用的 commit/tree。

## 2. Canonical N4 动态 gate

全部命令均在 task checkout 上实际执行，没有复用旧证据。

| gate | discovery / execution | fail / skip | 结果 |
| --- | --- | --- | --- |
| `node scripts/verify.mjs --only tooling` | `52/52` phases；其中 `50` 个 Node files 共 `543/543` tests，另有 package-store discovery 与 VS Code grammar | `0` fail；Node `0` skipped | PASS |
| Node 五文件单次 invocation | `36/36` tests | `0` fail，`0` skipped | PASS |
| identity single-source self-test | 1 checker | `0` fail / skip | PASS |
| identity single-source production check | 1 checker | `0` fail / skip | PASS |
| `node scripts/verify.mjs --only router` | `50/50` files，`608/608` tests | `0` fail，`0` skipped | PASS |
| test-runner `--list` | `24` tests，`0` benchmarks | 无执行 skip | PASS |
| test-runner execution | `23/24` passed | `0` failed，`1` ignored | PASS |
| `node scripts/run-skiff-tests.mjs` | `2` canonical source entries；内部三组 `11 + 6 + 4 = 21` tests | `0` failed，`0` skipped | PASS |
| `cargo fmt --all -- --check` | workspace | 无 diff | PASS |
| `git diff --check` | tracked tree | 无错误 | PASS |
| pre-result `git status --porcelain` | tracked tree | 空输出 | PASS |

Tooling 的 `52` phases 与 list gate 精确一致。全部 50 个 Node phase 都输出了非零实际 test
总数；package-store checker 输出 `Package store discovery check passed.`，grammar phase exit 0。

五文件组的实际拆分为：

- artifact identity validation：`7/7`；
- package/service authoring：`9/9`；
- I02 combined：`6/6`；
- runtime execution boundary checker：`4/4`；
- source-suite ownership self-test：`10/10`。

Test-runner listing 与 execution 使用任务指定的共享
`CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target`。
execution 的唯一 ignored test 是
`platform_source_identity_probe`，其输出明确标记
`I16/G16 shared-target identity probe only`；其余 `23` 个均通过，命令 exit 0。

Source suite 的 canonical runner 自己持有隔离 runtime 生命周期；两个 registry entries
全部完成，实际执行的 std、alias-return-catch-once 与 package-service-host test 分别为
`11/11`、`6/6`、`4/4`。没有访问 stable 或 live target。

## 3. Plan 与 retired-owner 反搜

只读 plan 命令：

```bash
node scripts/verify.mjs --only tooling --list
```

实际输出 `selectors: tooling`、`phases: 52`：50 个 Node test phase、一个
package-store discovery phase、一个 VS Code grammar phase。

两组任务指定反搜均为 0 matches（`rg` exit 1）：

```text
scripts:
runPackageServiceGenerationLifecycleSmoke
r05-generation-lifecycle
entrypoints[2]

router:
AssemblyWebSocketGateway
canonicalAssemblyWebSocketIngressIdentity
assemblyWebSocketGateway
```

因此旧 generation / third-entrypoint owner 与 Router Assembly WebSocket ingress owner 均为 0。

## 4. Router target 环境诊断

第一次额外诊断曾把仅供显式 Cargo gate 使用的共享 `CARGO_TARGET_DIR` 注入 Node-owned Router
命令。Router test helper 自己执行 Cargo build，但按其固定契约读取 checkout-local
`build/cargo-target/debug/skiff-artifact-identity`；额外环境把 build 输出改到共享 target，
因而该非-canonical 尝试在 `artifacts.test.ts` setup 以 `ENOENT` 结束（其余已完成部分为
`49` files、`523` tests passed、`85` skipped）。

只读核对任务命令与既有 F420B gate 后，按任务原样执行 canonical
`node scripts/verify.mjs --only router`，实际得到 `50/50` files、`608/608` tests PASS，
无 skip。期间没有修改或修复任何实现、test、fixture、manifest、lockfile或验证计划；前一次
额外环境诊断不属于 canonical gate，也不构成候选失败。

## 5. 环境、写入边界与 release verdict

主 Agent 预备的 frozen dependencies、ignored `router/node_modules` /
`vscode/node_modules`、共享 Cargo target，以及 Router canonical gate 产生的 ignored local
Cargo build output 均未改变 tracked tree。写入本 result 前再次确认：

```text
git diff --check        PASS
git status --porcelain  empty
```

没有修改实现或测试；没有派子 Agent，没有 merge、rebase、push、stable、live 或 watch
registry 操作，也没有手工启动 instance。任务要求的 source-suite isolated owner 仅由
canonical runner 在临时目录内创建并完成清理。

全部 canonical gate 通过，证据失效范围为空。因此最终 verdict 为 `N4_PASS`，解除
`F421_RELEASED`。
