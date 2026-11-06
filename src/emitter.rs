//! Tracks IL instruction positions so exception filters can be emitted with
//! `hao`'s public method-body API. No synthetic instructions are inserted.
use hao::dotnet::{
    entries::TypeDefOrRef,
    il::{EmitLabel, Emitter, HandlerKind, Instruction, Label, MemberToken, MethodBody, OpCode},
    signature::TypeSig,
};

#[derive(Default)]
pub(crate) struct TrackedEmitter {
    inner: Emitter,
    instructions: usize,
    filters: Vec<(usize, usize)>,
}

impl TrackedEmitter {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn label(&mut self) -> EmitLabel {
        self.inner.label()
    }
    pub(crate) fn mark(&mut self, label: EmitLabel) {
        self.inner.mark(label);
    }
    pub(crate) fn position(&self) -> usize {
        self.instructions
    }
    pub(crate) fn declare_local(&mut self, ty: TypeSig) -> u16 {
        self.inner.declare_local(ty)
    }
    pub(crate) fn emit(&mut self, instruction: Instruction) -> &mut Self {
        self.instructions += 1;
        self.inner.emit(instruction);
        self
    }
    pub(crate) fn op(&mut self, opcode: OpCode) -> &mut Self {
        self.instructions += 1;
        self.inner.op(opcode);
        self
    }
    pub(crate) fn branch(&mut self, opcode: OpCode, target: EmitLabel) -> &mut Self {
        self.instructions += 1;
        self.inner.branch(opcode, target);
        self
    }
    pub(crate) fn ldarg(&mut self, index: u16) -> &mut Self {
        self.instructions += 1;
        self.inner.ldarg(index);
        self
    }
    pub(crate) fn starg(&mut self, index: u16) -> &mut Self {
        self.instructions += 1;
        self.inner.starg(index);
        self
    }
    pub(crate) fn ldloc(&mut self, index: u16) -> &mut Self {
        self.instructions += 1;
        self.inner.ldloc(index);
        self
    }
    pub(crate) fn stloc(&mut self, index: u16) -> &mut Self {
        self.instructions += 1;
        self.inner.stloc(index);
        self
    }
    pub(crate) fn ldloca(&mut self, index: u16) -> &mut Self {
        self.instructions += 1;
        self.inner.ldloca(index);
        self
    }
    pub(crate) fn ldc_i4(&mut self, value: i32) -> &mut Self {
        self.instructions += 1;
        self.inner.ldc_i4(value);
        self
    }
    pub(crate) fn ldstr(&mut self, value: impl Into<String>) -> &mut Self {
        self.instructions += 1;
        self.inner.ldstr(value);
        self
    }
    pub(crate) fn ldnull(&mut self) -> &mut Self {
        self.instructions += 1;
        self.inner.ldnull();
        self
    }
    pub(crate) fn dup(&mut self) -> &mut Self {
        self.instructions += 1;
        self.inner.dup();
        self
    }
    pub(crate) fn pop(&mut self) -> &mut Self {
        self.instructions += 1;
        self.inner.pop();
        self
    }
    pub(crate) fn call(&mut self, method: impl Into<MemberToken>) -> &mut Self {
        self.instructions += 1;
        self.inner.call(method);
        self
    }
    pub(crate) fn callvirt(&mut self, method: impl Into<MemberToken>) -> &mut Self {
        self.instructions += 1;
        self.inner.callvirt(method);
        self
    }
    pub(crate) fn newobj(&mut self, method: impl Into<MemberToken>) -> &mut Self {
        self.instructions += 1;
        self.inner.newobj(method);
        self
    }
    pub(crate) fn ret(&mut self) -> &mut Self {
        self.instructions += 1;
        self.inner.ret();
        self
    }
    pub(crate) fn exception_handler(
        &mut self,
        catch_type: Option<TypeDefOrRef>,
        try_start: EmitLabel,
        try_end: EmitLabel,
        handler_start: EmitLabel,
        handler_end: EmitLabel,
    ) {
        self.inner
            .exception_handler(catch_type, try_start, try_end, handler_start, handler_end);
    }
    pub(crate) fn filter_handler(
        &mut self,
        try_start: EmitLabel,
        try_end: EmitLabel,
        filter_start: usize,
        handler_start: EmitLabel,
        handler_end: EmitLabel,
        handler_position: usize,
    ) {
        self.inner
            .exception_handler(None, try_start, try_end, handler_start, handler_end);
        self.filters.push((filter_start, handler_position));
    }
    pub(crate) fn finish(self) -> Result<MethodBody, String> {
        let mut body = self.inner.finish();
        if body.instructions.len() != self.instructions {
            return Err("exception filter instruction tracking mismatch".into());
        }
        for (filter, handler) in self.filters {
            let mut matches = body
                .exception_handlers
                .iter_mut()
                .filter(|entry| entry.handler_start.0 == handler);
            let entry = matches.next().ok_or("exception filter handler not found")?;
            if matches.next().is_some() || !matches!(entry.kind, HandlerKind::Finally) {
                return Err("ambiguous exception filter handler".into());
            }
            entry.kind = HandlerKind::Filter(Label(filter));
        }
        Ok(body)
    }
}
