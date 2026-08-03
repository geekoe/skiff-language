use super::*;

impl Parser {
    pub(super) fn parse_function_signature_and_skip_body(
        &mut self,
        allow_static: bool,
    ) -> Result<InterfaceOperation> {
        let signature = self.parse_function_signature(allow_static)?;
        match self.parse_callable_body(
            signature,
            CallableParseOptions {
                exported: false,
                provider_without_body: CallableNoBodyPolicy::SignatureOnly,
                native_without_body: CallableNoBodyPolicy::SignatureOnly,
                native_with_body: NativeBodyPolicy::SkipAndKeepSignature,
                body_policy: CallableBodyPolicy::SkipAndKeepSignature,
            },
        )? {
            CallableParseResult::Signature(signature) => Ok(signature),
            CallableParseResult::Decl { .. } | CallableParseResult::None => unreachable!(),
        }
    }

    pub(super) fn parse_callable_body(
        &mut self,
        signature: InterfaceOperation,
        options: CallableParseOptions,
    ) -> Result<CallableParseResult> {
        if signature.is_provider {
            if self.check_symbol("{") {
                return Err(CompileError::syntax(
                    "provider function cannot have a body",
                    self.peek().span.start,
                ));
            }
            return Ok(self.finish_bodyless_callable(
                signature,
                options.exported,
                options.provider_without_body,
            ));
        }

        if signature.is_native && !self.check_symbol("{") {
            return Ok(self.finish_bodyless_callable(
                signature,
                options.exported,
                options.native_without_body,
            ));
        }

        if signature.is_native {
            return match options.native_with_body {
                NativeBodyPolicy::Error(message) => {
                    Err(CompileError::syntax(message, self.peek().span.start))
                }
                NativeBodyPolicy::FollowBodyPolicy => {
                    self.parse_callable_decl_body(signature, options.exported, options.body_policy)
                }
                NativeBodyPolicy::SkipAndKeepSignature => {
                    self.skip_callable_body_and_keep_signature(signature)
                }
                NativeBodyPolicy::SkipAndDrop => self.skip_callable_body_and_drop(),
            };
        }

        self.parse_callable_decl_body(signature, options.exported, options.body_policy)
    }

    pub(super) fn finish_bodyless_callable(
        &mut self,
        signature: InterfaceOperation,
        exported: bool,
        policy: CallableNoBodyPolicy,
    ) -> CallableParseResult {
        self.match_symbol(";");
        match policy {
            CallableNoBodyPolicy::EmptyDecl => {
                let end = self.previous().span.end;
                CallableParseResult::Decl {
                    decl: self.build_function_decl(
                        exported,
                        signature,
                        Block {
                            statements: Vec::new(),
                        },
                        end,
                    ),
                    spans: None,
                }
            }
            CallableNoBodyPolicy::SignatureOnly => CallableParseResult::Signature(signature),
        }
    }

    pub(super) fn parse_callable_decl_body(
        &mut self,
        signature: InterfaceOperation,
        exported: bool,
        policy: CallableBodyPolicy,
    ) -> Result<CallableParseResult> {
        match policy {
            CallableBodyPolicy::ParseStrict => {
                self.parse_callable_decl_body_strict(signature, exported)
            }
            CallableBodyPolicy::ParseTolerantKeepSignature => {
                self.parse_callable_decl_body_tolerant(signature, exported, true)
            }
            CallableBodyPolicy::ParseTolerantDrop => {
                self.parse_callable_decl_body_tolerant(signature, exported, false)
            }
            CallableBodyPolicy::SkipAndKeepSignature => {
                self.skip_callable_body_and_keep_signature(signature)
            }
        }
    }

    pub(super) fn parse_callable_decl_body_strict(
        &mut self,
        signature: InterfaceOperation,
        exported: bool,
    ) -> Result<CallableParseResult> {
        let body = self.parse_block(false)?;
        let end = self.previous().span.end;
        Ok(CallableParseResult::Decl {
            decl: self.build_function_decl(exported, signature, body.block, end),
            spans: Some(ExecutableSourceSpans {
                effects: Vec::new(),
                body: body.spans,
            }),
        })
    }

    pub(super) fn parse_callable_decl_body_tolerant(
        &mut self,
        signature: InterfaceOperation,
        exported: bool,
        keep_signature_on_failure: bool,
    ) -> Result<CallableParseResult> {
        let body_start = self.snapshot();
        match self.parse_block(false) {
            Ok(body) => {
                let end = self.previous().span.end;
                Ok(CallableParseResult::Decl {
                    decl: self.build_function_decl(exported, signature, body.block, end),
                    spans: Some(ExecutableSourceSpans {
                        effects: Vec::new(),
                        body: body.spans,
                    }),
                })
            }
            Err(_) => {
                self.restore(body_start);
                if self.check_symbol("{") {
                    self.skip_balanced_block("unterminated function body")?;
                } else {
                    return Err(CompileError::syntax(
                        "expected function body",
                        self.peek().span.start,
                    ));
                }
                if keep_signature_on_failure {
                    Ok(CallableParseResult::Signature(signature))
                } else {
                    Ok(CallableParseResult::None)
                }
            }
        }
    }

    pub(super) fn skip_callable_body_and_keep_signature(
        &mut self,
        signature: InterfaceOperation,
    ) -> Result<CallableParseResult> {
        if self.check_symbol("{") {
            self.skip_balanced_block("unterminated function body")?;
            Ok(CallableParseResult::Signature(signature))
        } else {
            Err(CompileError::syntax(
                "expected function body",
                self.peek().span.start,
            ))
        }
    }

    pub(super) fn skip_callable_body_and_drop(&mut self) -> Result<CallableParseResult> {
        if self.check_symbol("{") {
            self.skip_balanced_block("unterminated function body")?;
            Ok(CallableParseResult::None)
        } else {
            Err(CompileError::syntax(
                "expected function body",
                self.peek().span.start,
            ))
        }
    }

    pub(super) fn build_function_decl(
        &self,
        exported: bool,
        signature: InterfaceOperation,
        body: Block,
        end: SourceLocation,
    ) -> FunctionDecl {
        FunctionDecl {
            exported,
            name: signature.name,
            type_params: signature.type_params,
            params: signature.params,
            return_type: signature.return_type,
            body,
            is_native: signature.is_native,
            is_provider: signature.is_provider,
            is_static: signature.is_static,
            implicit_self: signature.implicit_self,
            span: SourceSpan {
                start: signature.span.start,
                end,
            },
        }
    }
}
