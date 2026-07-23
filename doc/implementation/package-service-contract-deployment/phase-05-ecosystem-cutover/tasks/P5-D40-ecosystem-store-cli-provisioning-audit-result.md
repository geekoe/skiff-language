# P5-D40：Ecosystem Store CLI Provisioning Audit Result

状态：complete。exact candidate为commit `bbd69ce11218f4d599ef694df3ec41d72db139fb`、tree
`211922136ac07c428ef3376429920c629c781799`、Cargo.lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。无代码、构建、测试或stable操作。

F03A已冻结`skiff-compiler __ecosystem-store`为唯一canonical store sidecar，compiler/build语义完整。缺口全部属于同一
scripts/config/install owner：

1. Router config仍接受`SKIFF_ECOSYSTEM_STORE_CLI`及dev-home/cwd ambient fallback，renderer又没有写显式path。
2. stable/isolated/dev本地路径与managed binary install没有`skiff-compiler`。
3. remote deploy build manifest虽包含compiler，但Router部署闭包不上传它。
4. remote PM2仍传Router已删除的`--release-mode`，ordinary dev-init仍生成F03B会拒绝的legacy rewrite。

冻结修复为F30A单节点：Router只接受YAML/显式CLI path；shared renderer写同一canonical path；local build/install原子安装
0755 compiler；remote上传manifest精确binary并写绝对path；删除unsupported flag与legacy rewrite；更新direct example。禁止
PATH/cwd fallback、Node store复刻或test-only默认值进入production。无需公共架构决策。

F30A会使F03B config/startup证据失效，只需窄刷新；F03B gateway/pin/dispatch与全部F03C证据不失效。
