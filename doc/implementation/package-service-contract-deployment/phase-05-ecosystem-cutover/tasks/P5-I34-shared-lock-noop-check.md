# P5-I34：Shared Lock No-op Check

权威设计为
`doc/architecture/package-service-contract-deployment.md` §6.2、§7、§12及§14；执行顺序来自phase plan的Wave 2
shared-lock串行收口。

DAG节点I34，依赖D43 COMPLETE。exact production commit为
`c59b4baf9752147cc49c141d89642d8b7f5aa507`，Cargo.lock必须保持blob
`f3ce5457138c58aec4c84abda431afa96013e3fd`。这是I02前唯一shared-lock no-op/locked compiler evidence owner。

全新只读Agent在当前docs descendant先确认无production diff，然后各运行一次：

```bash
test "$(git rev-parse c59b4baf9752147cc49c141d89642d8b7f5aa507:Cargo.lock)" = \
  f3ce5457138c58aec4c84abda431afa96013e3fd
cargo metadata --no-deps --format-version 1 --locked --offline >/dev/null
git diff --exit-code -- Cargo.lock
test "$(git hash-object Cargo.lock)" = \
  f3ce5457138c58aec4c84abda431afa96013e3fd
CARGO_TARGET_DIR="$(mktemp -d /tmp/skiff-p5-lock-compiler.XXXXXX)" \
  cargo check --locked --offline -p skiff-compiler
```

临时target由owner在命令结束后清理。禁止generate/update、编辑、提交、真实probe、instance/stable或其他gate。
PASS且lock无diff只解除I02环境准备，不作I02/R02 verdict；任何lock变化或locked compiler失败都停止并返回唯一owner。
no-op不使既有证据失效。
