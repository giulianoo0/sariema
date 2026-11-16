// SPDX-License-Identifier: Apache-2.0

#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    crate::report_panic(info);
    core::arch::wasm32::unreachable()
}
