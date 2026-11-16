// SPDX-License-Identifier: Apache-2.0

//! A second interpreter for comparison: tinywasm, budgeted with its
//! resumable calls (a suspended tick is dropped, never resumed) and NaNs
//! canonicalised.

use std::cell::RefCell;
use std::rc::Rc;

use tinywasm::engine::{Config, StackConfig};
use tinywasm::{
    Engine, Error, ExecProgress, FuncContext, FunctionTyped, HostFunction, Imports, Memory,
    ModuleInstance, ResourceLimiter, Store, Trap,
};

use crate::host::HostState;
use crate::{Interpreter, Machine, VmError, VmSettings};

pub struct Tinywasm;

struct TinywasmMachine {
    store: Store,
    memory: Memory,
    tick: FunctionTyped<(), ()>,
    version: Option<FunctionTyped<(), i32>>,
    /// The Vm's host state, lent here for the length of a tick.
    host: Rc<RefCell<HostState>>,
}

struct Ceiling {
    memory_bytes: usize,
    table_elements: usize,
}

impl ResourceLimiter for Ceiling {
    fn memory_growing(&self, _: usize, desired: usize, _: Option<usize>) -> Result<bool, Trap> {
        if desired > self.memory_bytes {
            return Err(Trap::OutOfMemory);
        }
        Ok(true)
    }

    fn table_growing(&self, _: usize, desired: usize, _: Option<usize>) -> Result<bool, Trap> {
        Ok(desired <= self.table_elements)
    }
}

fn classify(e: Error) -> VmError {
    match e {
        Error::Trap(Trap::OutOfMemory) => VmError::Memory,
        other => VmError::Trap(other.to_string()),
    }
}

fn module_error(e: Error) -> VmError {
    match classify(e) {
        VmError::Trap(why) => VmError::Module(why),
        other => other,
    }
}

fn budgeted<R>(progress: tinywasm::Result<ExecProgress<R>>) -> Result<R, VmError> {
    match progress.map_err(classify)? {
        ExecProgress::Completed(r) => Ok(r),
        ExecProgress::Suspended => Err(VmError::Budget),
    }
}

fn fuel(fuel: u64) -> u32 {
    fuel.min(u32::MAX as u64) as u32
}

fn imports(host: &Rc<RefCell<HostState>>) -> Imports {
    let memory = |ctx: &FuncContext<'_>| ctx.memory("memory");
    let failed = |why: String| Error::Other(why);
    let mut imports = Imports::new();
    let h = host.clone();
    imports.define(
        "sariema",
        "submit",
        HostFunction::from(move |ctx: FuncContext<'_>, (ptr, len): (i32, i32)| {
            let data = memory(&ctx)?.data(ctx.store())?;
            h.borrow_mut()
                .submit(data, ptr as u32, len as u32)
                .map_err(failed)
        }),
    );
    let h = host.clone();
    imports.define(
        "sariema",
        "events",
        HostFunction::from(move |mut ctx: FuncContext<'_>, (dst, cap): (i32, i32)| {
            let m = memory(&ctx)?;
            let data = m.data_mut(ctx.store_mut())?;
            h.borrow_mut()
                .events(data, dst as u32, cap as u32)
                .map_err(failed)
        }),
    );
    let h = host.clone();
    imports.define(
        "sariema",
        "register",
        HostFunction::from(
            move |ctx: FuncContext<'_>, (kind, ptr, len): (i32, i32, i32)| {
                let size = memory(&ctx)?.len(ctx.store())?;
                Ok(h.borrow_mut()
                    .register(size, kind as u32, ptr as u32, len as u32))
            },
        ),
    );
    let h = host.clone();
    imports.define(
        "sariema",
        "log",
        HostFunction::from(
            move |ctx: FuncContext<'_>, (level, ptr, len): (i32, i32, i32)| {
                let data = memory(&ctx)?.data(ctx.store())?;
                h.borrow_mut()
                    .log(data, level as u32, ptr as u32, len as u32)
                    .map_err(failed)
            },
        ),
    );
    let h = host.clone();
    imports.define(
        "sariema",
        "button",
        HostFunction::from(move |_: FuncContext<'_>, name: i32| Ok(h.borrow().button(name as u32))),
    );
    imports
}

impl Interpreter for Tinywasm {
    const NAME: &'static str = "tinywasm";

    fn load(wasm: &[u8], settings: &VmSettings) -> Result<Box<dyn Machine>, VmError> {
        let module = tinywasm::parse_bytes(wasm).map_err(|e| VmError::Module(e.to_string()))?;
        let config = Config::new()
            .with_call_stack(StackConfig::fixed(settings.max_call_depth as usize))
            .with_resource_limiter(Ceiling {
                memory_bytes: settings.memory_bytes.min(usize::MAX as u64) as usize,
                table_elements: settings.table_elements as usize,
            });
        let mut store = Store::new(Engine::new(config));
        let host = Rc::new(RefCell::new(HostState::new(*settings)));
        let instance = ModuleInstance::instantiate(&mut store, &module, Some(&imports(&host)))
            .map_err(module_error)?;
        let memory = instance
            .memory("memory")
            .map_err(|_| VmError::Module("the module exports no memory".into()))?;
        let tick = instance
            .func::<(), ()>(&store, "sariema_tick")
            .map_err(|_| VmError::Module("the module exports no sariema_tick() function".into()))?;
        let version = instance
            .func::<(), i32>(&store, "sariema_batch_version")
            .ok();
        Ok(Box::new(TinywasmMachine {
            store,
            memory,
            tick,
            version,
            host,
        }))
    }
}

impl Machine for TinywasmMachine {
    fn tick(&mut self, budget: u64, host: &mut HostState) -> Result<(), VmError> {
        std::mem::swap(&mut *self.host.borrow_mut(), host);
        let result = self
            .tick
            .call_resumable(&mut self.store, ())
            .and_then(|mut run| run.resume_with_fuel(fuel(budget)));
        std::mem::swap(&mut *self.host.borrow_mut(), host);
        budgeted(result)
    }

    fn batch_version(&mut self, budget: u64) -> Result<Option<u32>, VmError> {
        let Some(f) = &self.version else {
            return Ok(None);
        };
        let result = f
            .call_resumable(&mut self.store, ())
            .and_then(|mut run| run.resume_with_fuel(fuel(budget)));
        budgeted(result).map(|v| Some(v as u32))
    }

    fn memory(&self) -> &[u8] {
        self.memory
            .data(&self.store)
            .expect("the memory belongs to this store")
    }
}
