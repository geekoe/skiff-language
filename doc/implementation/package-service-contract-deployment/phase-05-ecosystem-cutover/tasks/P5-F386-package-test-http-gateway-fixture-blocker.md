# P5-F386 Package-test HTTP gateway fixture blocker

状态：TASK_SCOPE_EXPANDED（T1 canonical type已切换；T2 binary仍消费旧字段）。

## 保留checkpoint

- worktree：
  `/Users/geek/workspace/skiff-p5-f386-package-test-http-gateway`
- branch：
  `codex/p5-f386-package-test-http-gateway`
- scoped T1 WIP：
  `89ffbeca41ef2c60ae754abd58a155fd2b72ac70`
- result/HEAD：
  `a2ea3cd40e1f1f262ed4526b319e110f48052e6b`
- final tree：
  `ee0dc40ff31ba7d016613060809e87894d65678b`
- worktree clean；未合流。

T1已经把`CanonicalPackageTestEntrypoint`改为deployment/gateway key/identity/selector/mode，并在scoped
production中删除contract operation与wire doubles旧字段；library check与反搜通过。

## 阻塞

`test-runner/src/bin/package_service_smoke_fixture.rs:237-238`仍读取已删除的
`fixture.package_test.contract`和`.operation`。因此`cargo check --bins`及integration test在编译阶段失败。

不能增加临时compatibility字段，因为那会恢复F384明确禁止的旧合同。F384已经冻结T1/T2共享同一canonical
entrypoint类型与helper，后继应从clean checkpoint直接完成T2 HTTP迁移，再一起跑T1/T2真实验证。
