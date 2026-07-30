# P5-F55D：Activation Facts Residual

只改runtime activation对已删除`LinkedImageActivationFacts`的残留导入/字段/测试，迁到canonical
AssemblyExecutionImage/assembly activation事实或删除已不可达旧语义；不得恢复linker type/alias。运行activation
及combined check、rustfmt/diff，提交单一commit。
