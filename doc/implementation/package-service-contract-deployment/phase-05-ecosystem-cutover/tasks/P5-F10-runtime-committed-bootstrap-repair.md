# P5-F10：Runtime Committed Bootstrap Repair

## 输入、owner与限制

- 输入：D12完成；exact code integration `ff7a4dfbcabab23ba2f1f8f38e407cbf4d9655ee` / tree
  `aed0fdade3547322ea85b79e2676174a35aad6b4`，已包含F09/R10 PASS seam与F04A checkpoint。
- 独立worktree/branch，一个clean commit，不merge/push。R11 PASS只恢复F04真实Host probe，不提前解锁整个F03C。
- owner限于Runtime driver config、Runtime Host lifecycle/admission committed recovery/registration及直接tests；配置renderer
  只可把environment与singular canonical artifact root传给Runtime YAML。
- 不改Router、test-runner/fixture、compiler、shared activation/request codec、F05 ABI、manifest/Cargo.lock；不实现
  F03C request trust boundary、WS、drain或其余startup职责；不操作stable。

## 完成态

Runtime YAML严格要求`environment`与singular `artifactRoot`；旧`artifactRoots`不兼容读取。instance传
`config.environment`与canonical instance artifact root，本地CLI传`dev`，deploy传`prod`与远端canonical root；
所有renderer caller显式传值。

现有lifecycle成为唯一reconnect sequencing owner：每次建立Router session前，从canonical store的exact environment
path重读完整activation state，拒绝missing/tampered/partial/cross-environment/identity mismatch，只恢复committed。
Runtime用online prepare共享的exact resolve/load/link/validate/admit primitive加载assembly，并用online commit共享的
committed publication primitive原子发布active+committed；成功后才连接并发送capabilities、exact register。

durable pending只由Router连接后的原transaction重放；cold recovery不发送prepared/commit、不激活pending。断线丢弃
staged并在重连前重读，离线N→N+1后必须先admit N+1再register。generation-0只在committed recovery允许，online
candidate仍严格大于0。production direct `admit_runtime_assembly` bypass必须删除、私有化或限制为test-only。

## 写入边界与验证

最小写集：`runtime/driver/{config.rs,main.rs}`、`runtime/host/src/host/{runtime_host.rs,lifecycle.rs}`、admission
controller及聚焦recovery primitive、direct Runtime tests；以及`runtime-stack-config.mjs`、三个renderer caller与直接
Node tests。不得把reconnect状态机继续塞进`router_session.rs`或`control_plane.rs`。

```bash
node --test \
  scripts/tests/runtime-stack-config.test.mjs \
  scripts/tests/skiff-instance-config.test.mjs \
  scripts/tests/isolated-test-runtime.test.mjs
cargo test --locked --manifest-path runtime/Cargo.toml
cargo test --locked -p skiff-runtime-host --test active_runtime_assembly
cargo test --locked -p skiff-runtime-host committed_recovery
cargo clippy --locked -p skiff-runtime-host --all-targets --no-deps -- -D warnings
git diff --check
```

必须另跑真实ready-only isolated probe：canonical empty generation-0 store → Runtime exact recovery/admit → 同socket
capabilities后register → Router health同时出现capability connection与匹配environment/generation/assembly的healthy replica；
不执行F04 fixture、不作F04 verdict。

测试还必须覆盖non-empty N restart、pending只恢复committed、全类store/ref失败时零次连接、reconnect N→N+1，以及
recovered gen0后的prepare gen1→prepared但仍register gen0、abort保持gen0、commit后register gen1。回报配置矩阵、
recovery/online primitive复用、frame顺序、source/commit/tree、single commit/clean/lock、reverse-search与extra-review。

## R11 acceptance record

F10 candidate `47d92595cc346cdbbee184ebb467f3bc2aecb01d` / tree
`70d3c8d31c2a748ff642c99f2f3c29947bf181c2`由独立R11判定PASS并合流为`efb2bbbe`。strict config、每connect
exact committed recovery、共享admission/publication、pending/reconnect/gen0语义及真实ready-only probe均通过；
Node 24/24、Runtime 277/277、recovery 4/4、active 2/2、reconnect 1/1。Clippy仍为既有基线失败且候选错误数下降，
新增recovery/lifecycle文件无诊断；lock未变。
