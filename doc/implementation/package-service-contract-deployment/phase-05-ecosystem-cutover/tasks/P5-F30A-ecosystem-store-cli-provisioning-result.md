# P5-F30A：Ecosystem Store CLI Provisioning Result

状态：complete。commit `4a7b145396dc1359d0581d06e0bda1c31718504f`、tree
`e0202d962d2580a89871bf5066972d3787b70714`、Cargo.lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。

Router store adapter只接受YAML或CLI override的绝对path；local/isolated/build-dev-runtime统一在各自
`<devHome>/bin/skiff-compiler[.exe]`原子安装0755 binary；remote Router闭包消费build-manifest compiler unit并上传/chmod；
PM2 unsupported `--release-mode`与ordinary dev-init legacy rewrite已删除。relative/missing/non-executable/wrong-protocol
binary及manifest缺compiler均fail closed。

Router direct 29/29、scripts/fake lifecycle 49/49、13个syntax、Router type-check及diff-check全部PASS。未修改compiler
store/CLI、Runtime、F23E、Cargo manifests/lock或stable配置。
