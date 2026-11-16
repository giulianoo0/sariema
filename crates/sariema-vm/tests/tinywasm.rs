// SPDX-License-Identifier: Apache-2.0

#![cfg(feature = "tinywasm")]

type Interp = sariema_vm::Tinywasm;

include!("suite/vm.rs");
