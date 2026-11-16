// SPDX-License-Identifier: Apache-2.0

//! Gas metering by instrumentation, as wasm-instrument's mutable-global
//! mode: an exported `i64` global `sariema_gas`, charged at the start of
//! every instruction sequence (function body, block, loop body, if arm)
//! with that sequence's instruction count; below zero, the module traps.
//!
//!   inject <in.wasm> <out.wasm>

use walrus::ir::{
    BinaryOp, Binop, Const, GlobalGet, GlobalSet, IfElse, Instr, InstrLocId, InstrSeq, InstrSeqId,
    Value, Visitor, dfs_in_order,
};
use walrus::{ConstExpr, Module, ValType};

struct Seqs(Vec<(InstrSeqId, usize)>);

impl<'i> Visitor<'i> for Seqs {
    fn start_instr_seq(&mut self, s: &'i InstrSeq) {
        self.0.push((s.id(), s.instrs.len()));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output] = args.as_slice() else {
        panic!("usage: inject <in.wasm> <out.wasm>");
    };
    let mut m = Module::from_buffer(&std::fs::read(input).unwrap()).unwrap();
    let gas = m
        .globals
        .add_local(ValType::I64, true, false, ConstExpr::Value(Value::I64(0)));
    m.exports.add("sariema_gas", gas);
    let mut charged = 0usize;
    for (_, f) in m.funcs.iter_local_mut() {
        let mut v = Seqs(Vec::new());
        dfs_in_order(&mut v, f, f.entry_block());
        for (seq, n) in v.0 {
            if n == 0 {
                continue;
            }
            let trap = (f.builder_mut().dangling_instr_seq(None::<ValType>))
                .unreachable()
                .id();
            let none = f.builder_mut().dangling_instr_seq(None::<ValType>).id();
            let charge: [Instr; 8] = [
                GlobalGet { global: gas }.into(),
                Const {
                    value: Value::I64(n as i64),
                }
                .into(),
                Binop {
                    op: BinaryOp::I64Sub,
                }
                .into(),
                GlobalSet { global: gas }.into(),
                GlobalGet { global: gas }.into(),
                Const {
                    value: Value::I64(0),
                }
                .into(),
                Binop {
                    op: BinaryOp::I64LtS,
                }
                .into(),
                IfElse {
                    consequent: trap,
                    alternative: none,
                }
                .into(),
            ];
            let instrs = &mut f.block_mut(seq).instrs;
            instrs.splice(0..0, charge.into_iter().map(|i| (i, InstrLocId::default())));
            charged += 1;
        }
    }
    m.emit_wasm_file(output).unwrap();
    println!("{{\"charged_sequences\":{charged}}}");
}
