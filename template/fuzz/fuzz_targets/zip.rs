//! `pgstore::unzip_into`: the Postgres extension bundle (checked against a pinned SHA-256
//! before unpacking, but the unpacker must still hold against any zip).
#![no_main]

use desktop_core::pgstore::unzip_limited;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let dir = tempfile::tempdir().unwrap();
    if let Err(e) = unzip_limited(data, dir.path(), 64, 1 << 20) {
        assert!(!e.to_string().contains("the parser failed on this input"), "{e}");
    }
    // Nothing is ever written outside the folder (zip slip).
    for entry in std::fs::read_dir(dir.path()).unwrap().flatten() {
        let name = entry.file_name();
        assert!(name == "lib" || name == "share", "{name:?}");
    }
});
