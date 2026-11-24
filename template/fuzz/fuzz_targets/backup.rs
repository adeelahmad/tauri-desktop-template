//! `backup::read`: a backup file picked for restore (it may come from anywhere).
#![no_main]

use desktop_core::backup::{read, BackupKey};
use libfuzzer_sys::fuzz_target;
use std::sync::OnceLock;

fn setup() -> &'static (tempfile::TempDir, BackupKey) {
    static S: OnceLock<(tempfile::TempDir, BackupKey)> = OnceLock::new();
    S.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let key = BackupKey::load_or_create(dir.path()).unwrap();
        (dir, key)
    })
}

fuzz_target!(|data: &[u8]| {
    let (dir, key) = setup();
    let file = dir.path().join("backup-fuzz.jsonl");
    std::fs::write(&file, data).unwrap();
    let _ = read(&file, std::slice::from_ref(key));
});
