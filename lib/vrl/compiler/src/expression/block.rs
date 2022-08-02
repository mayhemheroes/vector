use std::fmt;

use crate::{
    expression::{Expr, Resolved},
    state::{ExternalEnv, LocalEnv},
    BatchContext, Context, Expression, TypeDef,
};

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    inner: Vec<Expr>,

    /// The local environment of the block.
    ///
    /// This allows any expressions within the block to mutate the local
    /// environment, but once the block ends, the environment is reset to the
    /// state of the parent expression of the block.
    pub(crate) local_env: LocalEnv,
    selection_vector_this: Vec<usize>,
    selection_vector_other: Vec<usize>,
}

impl Block {
    #[must_use]
    pub fn new(inner: Vec<Expr>, local_env: LocalEnv) -> Self {
        Self {
            inner,
            local_env,
            selection_vector_this: vec![],
            selection_vector_other: vec![],
        }
    }

    #[must_use]
    pub fn into_inner(self) -> Vec<Expr> {
        self.inner
    }
}

impl Expression for Block {
    fn resolve(&self, ctx: &mut Context) -> Resolved {
        // NOTE:
        //
        // Technically, this invalidates the scoping invariant of variables
        // defined in child scopes to not be accessible in parent scopes.
        //
        // However, because we guard against this (using the "undefined
        // variable" check) at compile-time, we can omit any (costly) run-time
        // operations to track/restore variables across scopes.
        //
        // This also means we don't need to make any changes to the VM runtime,
        // as it uses the same compiler as this AST runtime.
        let (last, other) = self.inner.split_last().expect("at least one expression");

        other
            .iter()
            .try_for_each(|expr| expr.resolve(ctx).map(|_| ()))?;

        last.resolve(ctx)
    }

    fn resolve_batch(&mut self, ctx: &mut BatchContext, selection_vector: &[usize]) {
        if self.inner.len() == 1 {
            self.inner[0].resolve_batch(ctx, selection_vector);
        } else {
            self.selection_vector_this.resize(selection_vector.len(), 0);
            self.selection_vector_this.copy_from_slice(selection_vector);

            for block in &mut self.inner {
                block.resolve_batch(ctx, &self.selection_vector_this);
                self.selection_vector_other.truncate(0);

                for index in selection_vector {
                    let index = *index;
                    if ctx.resolved_values[index].is_ok() {
                        self.selection_vector_other.push(index);
                    }
                }

                std::mem::swap(
                    &mut self.selection_vector_this,
                    &mut self.selection_vector_other,
                );
            }
        }
    }

    /// If an expression has a "never" type, it is considered a "terminating" expression.
    /// Type information of future expressions in this block should not be considered after
    /// a terminating expression.
    ///
    /// Since type definitions due to assignments are calculated outside of the "`type_def`" function,
    /// assignments that can never execute might still have adjusted the type definition.
    /// Therefore, expressions after a terminating expression must not be included in a block.
    /// It is considered an internal compiler error if this situation occurs, which is checked here
    /// and will result in a panic.
    ///
    /// VRL is allowed to have expressions after a terminating expression, but the compiler
    /// MUST not include them in a block expression when compiled.
    fn type_def(&self, (_, external): (&LocalEnv, &ExternalEnv)) -> TypeDef {
        let mut last = TypeDef::null();
        let mut fallible = false;
        let mut abortable = false;
        let mut has_terminated = false;
        for expr in &self.inner {
            assert!(!has_terminated, "VRL block contains an expression after a terminating expression. This is an internal compiler error. Please submit a bug report.");
            last = expr.type_def((&self.local_env, external));
            if last.is_never() {
                has_terminated = true;
            }
            if last.is_fallible() {
                fallible = true;
            }
            if last.is_abortable() {
                abortable = true;
            }
        }

        last.with_fallibility(fallible).with_abortability(abortable)
    }

    #[cfg(feature = "llvm")]
    fn emit_llvm<'ctx>(
        &self,
        state: (&LocalEnv, &ExternalEnv),
        ctx: &mut crate::llvm::Context<'ctx>,
    ) -> Result<(), String> {
        let block_begin_block = ctx.append_basic_block("block_begin");
        let block_end_block = ctx.append_basic_block("block_end");

        ctx.build_unconditional_branch(block_begin_block);
        ctx.position_at_end(block_begin_block);

        for expr in &self.inner {
            let type_def = expr.type_def(state);
            if type_def.is_fallible() {
                ctx.emit_llvm_for_ref(expr, state, ctx.result_ref())?;

                let is_err = ctx
                    .fns()
                    .vrl_resolved_is_err
                    .build_call(ctx.builder(), ctx.result_ref())
                    .try_as_basic_value()
                    .left()
                    .expect("result is not a basic value")
                    .try_into()
                    .expect("result is not an int value");

                let block_next_block = ctx.append_basic_block("block_next");
                ctx.build_conditional_branch(is_err, block_end_block, block_next_block);
                ctx.position_at_end(block_next_block);
            } else {
                ctx.emit_llvm_abortable(expr, state, ctx.result_ref(), block_end_block, vec![])?;
            }
        }

        ctx.build_unconditional_branch(block_end_block);
        ctx.position_at_end(block_end_block);

        Ok(())
    }
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("{\n")?;

        let mut iter = self.inner.iter().peekable();
        while let Some(expr) = iter.next() {
            f.write_str("\t")?;
            expr.fmt(f)?;
            if iter.peek().is_some() {
                f.write_str("\n")?;
            }
        }

        f.write_str("\n}")
    }
}
