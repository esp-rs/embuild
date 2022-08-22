// Remove if STD is supported for your platform and you plan to use it
#![no_std]

// Remove if STD is supported for your platform and you plan to use it
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

//
// Entry points
//

#[no_mangle]
extern "C" fn arduino_setup() {
}

#[no_mangle]
extern "C" fn arduino_loop() {
}
