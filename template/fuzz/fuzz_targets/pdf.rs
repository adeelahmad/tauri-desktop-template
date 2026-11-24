//! PDFs: `pdf::pdf_pages_from_bytes` (what the `<app> --parse` child runs) and the child's
//! request reader. A panic inside the parser is caught, so it is reported here by turning
//! the guard's error back into a crash: libFuzzer then keeps the input.
#![no_main]

use desktop_core::pdf::{pdf_pages_from_bytes, read_request};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = read_request(&mut &data[..]);
    if let Err(e) = pdf_pages_from_bytes(data.to_vec()) {
        let e = e.to_string();
        assert!(!e.contains("the parser failed on this input"), "{e}");
    }
});
