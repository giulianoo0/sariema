// SPDX-License-Identifier: Apache-2.0

//! The default interpreter: wasmi, compiled eagerly at load, with fuel
//! metering and NaN canonicalisation (its `deterministic` feature).

use wasmi::{
    Caller, CompilationMode, Config, Engine, Error, ExternType, Linker, Memory, Module, Store,
    StoreLimits, StoreLimitsBuilder, TrapCode, TypedFunc,
};

use crate::host::HostState;
use crate::{Interpreter, Machine, VmError, VmSettings};

const PAGE: u64 = 65_536;

struct State {
    host: HostState,
    limits: StoreLimits,
    memory: Option<Memory>,
}

pub struct Wasmi;

struct WasmiMachine {
    store: Store<State>,
    memory: Memory,
    tick: TypedFunc<(), ()>,
    version: Option<TypedFunc<(), i32>>,
}

fn host_error(e: String) -> Error {
    Error::new(e)
}

fn classify(e: Error) -> VmError {
    match e.as_trap_code() {
        Some(TrapCode::OutOfFuel) => VmError::Budget,
        Some(TrapCode::GrowthOperationLimited) => VmError::Memory,
        _ => VmError::Trap(e.to_string()),
    }
}

fn linker(engine: &Engine) -> Result<Linker<State>, Error> {
    let mut linker = Linker::<State>::new(engine);
    let memory = |caller: &Caller<'_, State>| {
        caller
            .data()
            .memory
            .ok_or_else(|| host_error("an import was called before memory exists".into()))
    };
    linker.func_wrap(
        "sariema",
        "submit",
        move |mut caller: Caller<'_, State>, ptr: u32, len: u32| -> Result<(), Error> {
            let m = memory(&caller)?;
            let (data, state) = m.data_and_store_mut(&mut caller);
            state.host.submit(data, ptr, len).map_err(host_error)
        },
    )?;
    linker.func_wrap(
        "sariema",
        "events",
        move |mut caller: Caller<'_, State>, dst: u32, cap: u32| -> Result<i32, Error> {
            let m = memory(&caller)?;
            let (data, state) = m.data_and_store_mut(&mut caller);
            state.host.events(data, dst, cap).map_err(host_error)
        },
    )?;
    linker.func_wrap(
        "sariema",
        "register",
        move |mut caller: Caller<'_, State>, kind: u32, ptr: u32, len: u32| -> Result<i32, Error> {
            let m = memory(&caller)?;
            let (data, state) = m.data_and_store_mut(&mut caller);
            Ok(state.host.register(data.len(), kind, ptr, len))
        },
    )?;
    linker.func_wrap(
        "sariema",
        "log",
        move |mut caller: Caller<'_, State>, level: u32, ptr: u32, len: u32| -> Result<(), Error> {
            let m = memory(&caller)?;
            let (data, state) = m.data_and_store_mut(&mut caller);
            state.host.log(data, level, ptr, len).map_err(host_error)
        },
    )?;
    linker.func_wrap(
        "sariema",
        "ray",
        move |mut caller: Caller<'_, State>,
              query: u32,
              query_len: u32,
              hit: u32,
              hit_len: u32|
              -> Result<i32, Error> {
            let m = memory(&caller)?;
            let (data, state) = m.data_and_store_mut(&mut caller);
            state
                .host
                .ray(data, query, query_len, hit, hit_len)
                .map_err(host_error)
        },
    )?;
    linker.func_wrap(
        "sariema",
        "button",
        |caller: Caller<'_, State>, name: u32| -> i32 { caller.data().host.button(name) },
    )?;
    Ok(linker)
}

impl Interpreter for Wasmi {
    const NAME: &'static str = "wasmi";

    fn load(wasm: &[u8], settings: &VmSettings) -> Result<Box<dyn Machine>, VmError> {
        let mut config = Config::default();
        config
            .consume_fuel(true)
            .compilation_mode(CompilationMode::Eager)
            .set_max_recursion_depth(settings.max_call_depth as usize);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, wasm).map_err(|e| VmError::Module(e.to_string()))?;
        for export in module.exports() {
            if let ExternType::Memory(ty) = export.ty()
                && ty.minimum().saturating_mul(PAGE) > settings.memory_bytes
            {
                return Err(VmError::Memory);
            }
        }
        let limits = StoreLimitsBuilder::new()
            .memory_size(settings.memory_bytes.min(usize::MAX as u64) as usize)
            .table_elements(settings.table_elements as usize)
            .instances(1)
            .memories(1)
            .tables(16)
            .trap_on_grow_failure(true)
            .build();
        let state = State {
            host: HostState::new(*settings),
            limits,
            memory: None,
        };
        let mut store = Store::new(&engine, state);
        store.limiter(|s| &mut s.limits);
        store
            .set_fuel(settings.fuel_first_step)
            .expect("fuel metering is enabled");
        let linker = linker(&engine).map_err(|e| VmError::Module(e.to_string()))?;
        let instance =
            linker
                .instantiate_and_start(&mut store, &module)
                .map_err(|e| match classify(e) {
                    VmError::Trap(why) => VmError::Module(why),
                    other => other,
                })?;
        let memory = instance
            .get_memory(&store, "memory")
            .ok_or_else(|| VmError::Module("the module exports no memory".into()))?;
        store.data_mut().memory = Some(memory);
        let tick = instance
            .get_typed_func::<(), ()>(&store, "sariema_tick")
            .map_err(|_| VmError::Module("the module exports no sariema_tick() function".into()))?;
        let version = instance
            .get_typed_func::<(), i32>(&store, "sariema_batch_version")
            .ok();
        Ok(Box::new(WasmiMachine {
            store,
            memory,
            tick,
            version,
        }))
    }
}

impl Machine for WasmiMachine {
    fn tick(&mut self, fuel: u64, host: &mut HostState) -> Result<(), VmError> {
        self.store.set_fuel(fuel).expect("fuel metering is enabled");
        std::mem::swap(&mut self.store.data_mut().host, host);
        let result = self.tick.call(&mut self.store, ()).map_err(classify);
        std::mem::swap(&mut self.store.data_mut().host, host);
        result
    }

    fn batch_version(&mut self, fuel: u64) -> Result<Option<u32>, VmError> {
        let Some(f) = self.version else {
            return Ok(None);
        };
        self.store.set_fuel(fuel).expect("fuel metering is enabled");
        f.call(&mut self.store, ())
            .map(|v| Some(v as u32))
            .map_err(classify)
    }

    fn memory(&self) -> &[u8] {
        self.memory.data(&self.store)
    }
}
