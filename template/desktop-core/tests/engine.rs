//! End to end on the file store with a tiny deterministic embedder (no downloads).

use desktop_core::{normalise, Embedder, Engine, FileStore, Result};

/// Hashes words into 64 buckets: texts that share words point the same way.
struct Bag(&'static str);

impl Embedder for Bag {
    fn name(&self) -> String {
        self.0.into()
    }
    fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| bag(t)).collect())
    }
    fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        Ok(bag(text))
    }
}

fn bag(text: &str) -> Vec<f32> {
    let mut v = vec![0.0f32; 64];
    for w in desktop_core::store::terms(text) {
        let h = w.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
        v[(h % 64) as usize] += 1.0;
    }
    normalise(&mut v);
    v
}

const DOCS: &str = "The heart pumps blood through the arteries.\n\nRust programs compile to native code.\n\nTauri apps use the system web view.";

#[test]
fn keyword_and_vector_search_and_model_change() {
    let dir = tempfile::tempdir().unwrap();
    let open = |emb: Option<Box<dyn Embedder>>| {
        Engine::with_parts(Box::new(FileStore::open(&dir.path().join("store")).unwrap()), emb, dir.path(), &mut |_| {}).unwrap()
    };

    // Keywords only.
    let e = open(None);
    let n = e.ingest_text("notes", &DOCS.replace("\n\n", &format!("\n\n{}\n\n", "x ".repeat(500)))).unwrap();
    assert!(n >= 2);
    let hits = e.search("which web view does tauri use", 3).unwrap();
    assert!(hits[0].text.contains("Tauri"));
    assert_eq!(e.status().unwrap().embedder, None);

    // Re-ingesting a source replaces it.
    e.ingest_text("notes", DOCS).unwrap();
    assert_eq!(e.status().unwrap().chunks, 1);
    drop(e);

    // A model appears: stored chunks are embedded on open.
    let e = open(Some(Box::new(Bag("bag-v1"))));
    assert_eq!(e.status().unwrap().embedder.as_deref(), Some("bag-v1"));
    assert_eq!(e.status().unwrap().chunks, 1);
    e.ingest_text("more", "Blood flows from the heart into arteries.").unwrap();
    let hits = e.search("heart arteries blood", 1).unwrap();
    assert_eq!(hits[0].source, "more");
    drop(e);

    // Same model: nothing re-embedded, data persists.
    let e = open(Some(Box::new(Bag("bag-v1"))));
    assert_eq!(e.status().unwrap().chunks, 2);
    assert_eq!(e.remove_source("more").unwrap(), 1);
    e.clear().unwrap();
    assert_eq!(e.status().unwrap().chunks, 0);
}
