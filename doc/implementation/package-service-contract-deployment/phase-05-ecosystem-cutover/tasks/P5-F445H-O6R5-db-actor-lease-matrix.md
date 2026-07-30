# P5-F445H-O6R5 DB/Actor lease matrix

状态：Ready。共享fixture已经冻结。本节点只补lease claim/read/renew/lost/release矩阵，不修改fixture或
production。

## 直接父节点

- `P5-F445H-O6R2-db-actor-shared-fixture-checkpoint-result.md`

production prerequisite为integration commit `ceb73fbc`。父节点沿
`P5-F445H-O6R1-db-actor-fixture-owner-preflight-result.md` §5 F3冻结了完整矩阵。

## 唯一写集

- `runtime/eval/src/program_db/tests/lease.rs`
- `P5-F445H-O6R5-db-actor-lease-matrix-result.md`

不得修改共享fixture、`program_db.rs`、ordinary/transaction child、production、Cargo或lockfile。

## 必须覆盖

使用共享claim/read linked builder、phase script/gate/metrics、renew drop信号和真实Actor frame，至少
形成8个非零`db_actor_lease_*`测试：

1. claim `None`的Ready/Pending都不导入binding、不renew、不release；
2. claim success的Ready/Pending只在成功后导入binding，claim只启动一次；
3. normal success、body error、显式非法flow都按stop/join renew → lease-lost → release收尾；
4. lease-lost Ready/Pending与release error保持一次调用和既有可见优先级；
5. body DB Pending期间drop使真实renew future被abort/drop，之后poll不增长；release允许为零；
6. release真实Pending期间drop不重建release、无late terminal；
7. lease read覆盖Ready、Pending、None、store error与受限heap触发的decode error；
8. lease read真实Pending drop不重建、不materialize。

Renew tick次数不作精确合同；只断言first-poll、normal stop/join、outer-drop abort和drop后poll不增长。
Claim/read/lost/release每个实际method必须有非零constructed/poll证据，禁止phase必须为零。Pending
success用竞争Actor acquire证明单次切段。不得只测试`LeaseRenewOwner`helper或fake store。

当前checkpoint缺失selector就是RED；不得修改production制造失败。测试若发现实现错误，保留最小失败
证据并返回FAIL。

## 验证

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r5-lease/build/cargo-target \
  cargo test -p skiff-runtime-eval program_db::tests::lease:: -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r5-lease/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o6r5-lease/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录实际测试数，少于8或零测试不算完成。不运行完整eval/stage gate、stable/live/network/Mongo。

冻结fixture若缺少唯一机械helper，五分钟内返回`TASK_SCOPE_EXPANDED`和精确缺口；不得修改fixture、
放宽矩阵或复制fake。不得派子Agent。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o6r5-lease
branch   codex/p5-f445h-o6r5-lease
```

先提交tests，再单独提交result；worktree clean，不得merge/rebase/push。

