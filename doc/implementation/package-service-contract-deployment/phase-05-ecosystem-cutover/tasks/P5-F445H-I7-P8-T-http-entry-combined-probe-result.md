# P5-F445H I7 P8 T HTTP entry combined probe result

状态：

```text
PASS
NO_PRODUCTION_CHANGE = YES
REAL_ISOLATED_ROUTER = YES
READY_FOR_INTEGRATION = YES
```

## 1. Candidate 与分支更新

最终冻结的 integration candidate：

- commit：
  `a5e484910b68784778c6de22ac0ca2a1fd893db2`
- tree：
  `0621766aefec0f91b42893380a1884ed88ea7f82`

T 最初基于 `9fd0fc003b8edd0bdb8fdd7626cfa5c7f6b1de22` 实现 fixture。generated test
service id hard cut 合入后，用下列方式只重放 T 自己的两笔提交：

```text
git rebase --onto \
  a5e484910b68784778c6de22ac0ca2a1fd893db2 \
  9fd0fc003b8edd0bdb8fdd7626cfa5c7f6b1de22
```

重放无冲突；fixture 不硬编码 generated service id，而是使用 runner 注入的动态 ingress URL，
selector 由普通 Router 路径产生。最终 implementation tip：

- commit：
  `6b52813f8c2f256e7d4f0d48f6ee742e400743ff`
- tree：
  `b0c1b44ecc8aa1e3e9d3f5214f95c2b4578d8bd8`

## 2. Hermetic 执行模型

probe 复用唯一的 `scripts/lib/isolated-test-runtime.mjs` owner：

- 动态租用 `46000`–`46999` 的四个 loopback 端口；
- 临时启动受管单节点 Mongo、真实 Router 和真实 Runtime；
- HTTP 请求经过 Router business port，不直接调用 handler，也不伪造 response sink；
- 不访问 stable instance、共享 Mongo、外网或 secret；
- owner 在成功和失败路径都停止进程、验证端口关闭、释放 lease 并删除临时目录；
- fixture 在 owner 返回后再次验证四个端口、临时目录和四个 lease 文件均已消失。

最初 dispatch 中“No Mongo”与“必须复用现有真实 isolated owner”冲突；主 Agent 明确修正为只允许
上述临时受管 Mongo，没有新增第二套 memory activation store 或 no-Mongo lifecycle。

## 3. Ancestry RED

在 ancestry floor
`3a87d37f81a04c249f308b311bd91dcfdf3a8aa3`
（tree `eafc29e952f6b5170e4f5faca4e5d181b3ace9f6`）临时应用同一 fixture，运行：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-test-runner \
  --test http_entry_test_service -- --nocapture
```

得到直接等价 RED：

```text
invalid canonical fixture: missing config binding skiff.test.ingressUrl
```

这证明旧 runner 尚不能把真实 business ingress 注入 test service，不是测试制造的失败。该次隔离栈
使用端口 `46011`–`46014`；失败后临时目录、Mongo、Router、Runtime、端口和 lease 均已清理。
用于 RED 的短期 worktree 与临时分支随后删除。

## 4. Candidate GREEN

在最终 candidate 上运行同一命令：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-test-runner \
  --test http_entry_test_service -- --nocapture
```

结果：

```text
1 passed; 0 failed
finished in 32.59s
```

Rust integration test只有在 Node orchestration 观察到下列全部 marker 后才通过：

- 第二个 active self-ingress 精确返回 canonical rejection；
- active-case 结束后 Router/Runtime assembly health 正常且无 pending activation；
- happy fixture 精确 `5 passed; 0 failed`；
- happy fixture 后 assembly health 正常且 Runtime capability connection 仍 connected；
- owner 完成后端口、临时目录和 lease 二次检查通过。

## 5. 覆盖矩阵

| 要求 | 真实路径与断言 |
| --- | --- |
| 显式 test HTTP entry | `http.yml` 绑定 `entry.__test.*` / `active.__test.*` wrapper |
| unary 完整 body | `POST /unary` 返回 request body 与 entry 内 outbound double body 的组合 |
| 父 inline effect 共享 | entry 内 `std.http.request` 消费父 case 声明的普通 HTTP double |
| raw HTTP stream | `std.http.stream` 经 Router/Runtime 消费 start/chunk/end，精确得到 `alpha|middle|omega` |
| consumer break/cancel | 读取 slow stream 首 chunk 后 `break`，随后同 case 顺序 self-ingress 成功 |
| 单 active 限制 | slow stream 未结束时第二个 self-ingress 返回 `already has an active self-ingress` |
| reserved headers | selector、Host、Content-Length、Transfer-Encoding、Connection 均捕获 `HttpError`，并核对 runtime-owned-header 精确消息 |
| 非 self origin | `https://example.test/direct` 仍命中普通 inline double，不被 self-ingress 接管 |
| Router 普通入口 | 所有 self-ingress 都使用动态 business URL 与普通 service/version selector |
| hard-cut service id | fixture 不读取或断言 generated service id 字面值；最终日志已显示新 `test.skiff/<pkg>/case-N` identity |

## 6. 验证与执行备注

```text
node --check test-runner/fixtures/http-entry-test-service/run.mjs
PASS

cargo fmt --all -- --check
PASS

git diff --check
PASS
```

祖先 build 曾让共享 Cargo target 留下旧 rmeta；最终 candidate 首次启动前表现为新 Host 源码看到旧
eval/request trait。确认源码完整且无并发 Cargo owner 后，只清理以下可再生产物：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo clean --locked \
  -p skiff-runtime-eval \
  -p skiff-runtime-request \
  -p skiff-runtime-native-contract \
  -p skiff-runtime-host \
  -p runtime
```

Cargo 报告删除 `2032 files, 1.0GiB`；重建后真实 GREEN。一次为提取额外输出而误用默认
worktree target 的尝试在启动隔离栈前中止，随后通过 task-local `cargo clean` 删除全部
`5.3GiB` 可再生产物，无遗留进程或目录。

未运行完整 workspace、stable/live、外网、OAuth、browser 或长压力 gate；本节点只负责聚焦的真实
Router/Runtime combined probe。

## 7. Actual write set

```text
test-runner/Cargo.toml
test-runner/tests/http_entry_test_service.rs
test-runner/fixtures/http-entry-test-service/run.mjs
test-runner/fixtures/http-entry-test-service/active/
  active.test.skiff
  api.yml
  config.skiff-test.yml
  http.yml
  package.yml
  service.yml
test-runner/fixtures/http-entry-test-service/happy/
  api.yml
  config.skiff-test.yml
  entry.test.skiff
  http.yml
  package.yml
  service.yml
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
  P5-F445H-I7-P8-T-http-entry-combined-probe.md
  P5-F445H-I7-P8-T-http-entry-combined-probe-result.md
```

未修改 compiler、Runtime、Router、std 或其它 production 文件。
