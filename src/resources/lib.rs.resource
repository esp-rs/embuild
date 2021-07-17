// Remove if STD is supported for your platform and you plan to use it
#![no_std]

// Remove if STD is supported for your platform and you plan to use it
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

//
// The functions below are just sample entry points so that there are no linkage errors
// Leave only the one corresponding to your vendor SDK framework
//

////////////////////////////////////////////////////////
// Arduino                                            //
////////////////////////////////////////////////////////

#[no_mangle]
extern "C" fn setup() {
}

#[no_mangle]
#[export_name = "loop"]
extern "C" fn arduino_loop() {
}

////////////////////////////////////////////////////////
// ESP-IDF                                            //
////////////////////////////////////////////////////////

#[no_mangle]
extern "C" fn app_main() {
}

////////////////////////////////////////////////////////
// All others                                         //
////////////////////////////////////////////////////////

#[no_mangle]
extern "C" fn main() -> i32 {
    0
}
