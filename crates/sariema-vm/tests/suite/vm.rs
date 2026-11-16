// SPDX-License-Identifier: Apache-2.0

// The machine's behaviour, included once per interpreter with `Interp` set.

use sariema_vm::{Vm, VmError, VmSettings};

fn load(wasm: &[u8], settings: VmSettings) -> Result<Vm, VmError> {
    Vm::load_with::<Interp>(wasm, settings)
}

const IMPORTS: &str = r#"
    (import "sariema" "submit" (func $submit (param i32 i32)))
    (import "sariema" "events" (func $events (param i32 i32) (result i32)))
    (import "sariema" "register" (func $register (param i32 i32 i32) (result i32)))
    (import "sariema" "log" (func $log (param i32 i32 i32)))
"#;

fn module(body: &str) -> Vec<u8> {
    wat::parse_str(format!(
        "(module {IMPORTS} (memory (export \"memory\") 1 1024) {body})"
    ))
    .unwrap()
}

fn settings() -> VmSettings {
    VmSettings {
        fuel_per_step: 100_000,
        fuel_first_step: 100_000,
        memory_bytes: 4 << 20,
        ..VmSettings::default()
    }
}

fn tick(vm: &mut Vm) -> Result<Vec<u8>, VmError> {
    let mut out = Vec::new();
    vm.tick(&[], &[], &mut out).map(|()| out)
}

const ECHO: &str = r#"
    (data (i32.const 16) "hello")
    (global $n (mut i32) (i32.const 0))
    (func (export "sariema_tick")
        (global.set $n (i32.add (global.get $n) (i32.const 1)))
        (i32.store8 (i32.const 20) (i32.add (i32.const 48) (global.get $n)))
        (call $submit (i32.const 16) (i32.const 5)))
    (func (export "sariema_batch_version") (result i32) (i32.const 1))
"#;

#[test]
fn a_tick_returns_what_the_game_submitted() {
    let mut vm = load(&module(ECHO), settings()).unwrap();
    assert_eq!(vm.batch_version().unwrap(), Some(1));
    assert_eq!(tick(&mut vm).unwrap(), b"hell1");
    assert_eq!(tick(&mut vm).unwrap(), b"hell2");
    assert_eq!(vm.interpreter(), <Interp as sariema_vm::Interpreter>::NAME);
}

#[test]
fn an_infinite_loop_is_stopped_by_the_budget_within_the_step() {
    let wasm = module(r#"(func (export "sariema_tick") (loop $l (br $l)))"#);
    let mut vm = load(&wasm, settings()).unwrap();
    let started = std::time::Instant::now();
    assert_eq!(tick(&mut vm), Err(VmError::Budget));
    assert!(
        started.elapsed().as_millis() < 500,
        "took {:?}",
        started.elapsed()
    );
    assert_eq!(VmError::Budget.code(), "E_GAME_BUDGET");
    assert_eq!(
        tick(&mut vm),
        Err(VmError::Budget),
        "a stopped game stays stopped"
    );
    assert_eq!(vm.stopped(), Some(&VmError::Budget));
}

#[test]
fn growing_memory_past_the_ceiling_is_refused_and_reported() {
    let wasm = module(
        r#"(func (export "sariema_tick")
            (drop (memory.grow (i32.const 1)))
            (drop (memory.grow (i32.const 100))))"#,
    );
    let mut vm = load(&wasm, settings()).unwrap();
    let error = tick(&mut vm).unwrap_err();
    assert_eq!(error, VmError::Memory);
    assert_eq!(error.code(), "E_GAME_MEMORY");
    assert_eq!(
        vm.memory().len(),
        2 * 65_536,
        "the allowed growth happened, the refused one did not"
    );
}

#[test]
fn a_module_whose_initial_memory_is_over_the_ceiling_does_not_load() {
    let wasm = wat::parse_str(format!(
        "(module {IMPORTS} (memory (export \"memory\") 100) (func (export \"sariema_tick\")))"
    ))
    .unwrap();
    assert_eq!(load(&wasm, settings()).err(), Some(VmError::Memory));
}

#[test]
fn a_trap_is_reported_and_the_host_keeps_running() {
    let wasm = module(r#"(func (export "sariema_tick") unreachable)"#);
    let mut vm = load(&wasm, settings()).unwrap();
    let error = tick(&mut vm).unwrap_err();
    assert_eq!(error.code(), "E_GAME_TRAP");
    assert!(error.to_string().contains("unreachable"), "{error}");

    let wasm = module(
        r#"(func (export "sariema_tick") (call $submit (i32.const 65530) (i32.const 100)))"#,
    );
    let mut vm = load(&wasm, settings()).unwrap();
    let error = tick(&mut vm).unwrap_err();
    assert_eq!(error.code(), "E_GAME_TRAP");
    assert!(error.to_string().contains("outside"), "{error}");

    let wasm = module(r#"(func $f (export "sariema_tick") (call $f))"#);
    let mut vm = load(
        &wasm,
        VmSettings {
            fuel_per_step: u64::MAX / 2,
            ..settings()
        },
    )
    .unwrap();
    assert_eq!(
        tick(&mut vm).unwrap_err().code(),
        "E_GAME_TRAP",
        "unbounded recursion"
    );
}

#[test]
fn modules_that_cannot_run_are_refused_at_load() {
    let missing_tick = module("");
    assert!(matches!(
        load(&missing_tick, settings()),
        Err(VmError::Module(_))
    ));
    let wasi = wat::parse_str(
        r#"(module (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1) (func (export "sariema_tick")))"#,
    )
    .unwrap();
    let error = load(&wasi, settings()).err().unwrap();
    assert_eq!(error.code(), "E_GAME_MODULE");
    assert!(error.to_string().contains("fd_write"), "{error}");
    assert_eq!(
        load(b"not wasm", settings()).err().unwrap().code(),
        "E_GAME_MODULE"
    );
}

const READER: &str = r#"
    (func (export "sariema_tick") (local $n i32)
        (loop $more
            (local.set $n (call $events (i32.const 1024) (i32.const 6)))
            (if (i32.gt_s (local.get $n) (i32.const 0))
                (then
                    (call $submit (i32.const 1024) (local.get $n))
                    (call $submit (i32.const 0) (i32.const 1))
                    (br $more))))
        (if (i32.lt_s (local.get $n) (i32.const 0))
            (then (i32.store (i32.const 4) (local.get $n)) (call $submit (i32.const 4) (i32.const 4)))))
"#;

#[test]
fn events_are_read_in_whole_records_that_fit() {
    let mut vm = load(&module(READER), settings()).unwrap();
    let events = b"aaabbbcccccc";
    let mut out = Vec::new();
    vm.tick(events, &[3, 6, 12], &mut out).unwrap();
    assert_eq!(
        out, b"aaabbb\0cccccc\0",
        "two records fit in 6 bytes, then one"
    );

    let mut vm = load(&module(READER), settings()).unwrap();
    vm.tick(b"0123456789", &[10], &mut out).unwrap();
    assert_eq!(
        out,
        (-10i32).to_le_bytes(),
        "a record over the buffer reports its size"
    );
}

#[test]
fn regions_and_logs_reach_the_host() {
    let wasm = module(
        r#"(data (i32.const 100) "boot\01\00\00\00")
           (func (export "sariema_tick")
             (i32.store (i32.const 0) (call $register (i32.const 1) (i32.const 104) (i32.const 4)))
             (i32.store (i32.const 8) (call $register (i32.const 99) (i32.const 0) (i32.const 4)))
             (call $log (i32.const 2) (i32.const 100) (i32.const 4))
             (call $submit (i32.const 0) (i32.const 12)))"#,
    );
    let mut vm = load(&wasm, settings()).unwrap();
    let out = tick(&mut vm).unwrap();
    assert_eq!(&out[0..4], &0i32.to_le_bytes());
    assert_eq!(&out[8..12], &(-1i32).to_le_bytes(), "unknown region kind");
    let regions: Vec<_> = vm.regions().collect();
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].1, [1, 0, 0, 0]);
    assert_eq!(vm.log()[0].text, "boot");
    assert_eq!(vm.log()[0].level, 2);
}

#[test]
fn the_same_module_and_events_give_the_same_memory() {
    let wasm = module(
        r#"(global $x (mut i32) (i32.const 1))
           (func (export "sariema_tick") (local $i i32)
             (loop $l
               (global.set $x (i32.add (i32.mul (global.get $x) (i32.const 1103515245)) (i32.const 12345)))
               (i32.store (i32.and (i32.shl (local.get $i) (i32.const 2)) (i32.const 65532)) (global.get $x))
               (f32.store (i32.const 60000) (f32.div (f32.const 0) (f32.const 0)))
               (local.set $i (i32.add (local.get $i) (i32.const 1)))
               (br_if $l (i32.lt_u (local.get $i) (i32.const 5000)))))"#,
    );
    let run = || {
        let fuel = 10_000_000;
        let mut vm = load(
            &wasm,
            VmSettings {
                fuel_per_step: fuel,
                fuel_first_step: fuel,
                ..settings()
            },
        )
        .unwrap();
        for _ in 0..10 {
            tick(&mut vm).unwrap();
        }
        vm.memory().to_vec()
    };
    assert_eq!(run(), run());
}
