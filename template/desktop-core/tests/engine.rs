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

const CORPUS: &[(&str, &str)] = &[
    ("erb", "Erb palsy is an upper trunk brachial plexus injury at C5 and C6 roots."),
    ("klumpke", "Klumpke palsy is a lower trunk injury affecting C8 and T1."),
    ("femoral", "The femoral nerve supplies the quadriceps and the skin of the anterior thigh."),
    ("heart", "The heart pumps blood through the arteries to the body."),
];

fn hybrid_checks(e: &Engine) {
    use desktop_core::{SearchConfig, SearchMode};
    for (source, text) in CORPUS {
        e.ingest_text(source, text).unwrap();
    }
    let hybrid = SearchConfig { mode: SearchMode::Hybrid, top_k: 3, ..SearchConfig::default() };
    let hits = e.search_with("upper trunk C5 palsy", &hybrid).unwrap();
    assert_eq!(hits[0].source, "erb", "{hits:?}");
    assert!(hits[0].vector_score.is_some() && hits[0].keyword_score.is_some(), "{hits:?}");
    // Keyword only, and a threshold that nothing meets.
    let kw = SearchConfig { mode: SearchMode::Keyword, ..hybrid.clone() };
    assert_eq!(e.search_with("quadriceps", &kw).unwrap()[0].source, "femoral");
    let strict = SearchConfig { min_score: Some(10.0), ..hybrid.clone() };
    assert!(e.search_with("upper trunk", &strict).unwrap().is_empty());
    // Vector weight 0 leaves the keyword ranking.
    let kw_only = SearchConfig { vector_weight: 0.0, ..hybrid };
    assert_eq!(e.search_with("femoral nerve", &kw_only).unwrap()[0].source, "femoral");
}

#[test]
fn hybrid_search_on_the_file_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = FileStore::open(&dir.path().join("store")).unwrap();
    let e = Engine::with_parts(Box::new(store), Some(Box::new(Bag("bag"))), dir.path(), &mut |_| {}).unwrap();
    hybrid_checks(&e);
}

#[test]
fn parallel_folder_ingest_respects_types_and_size() {
    let dir = tempfile::tempdir().unwrap();
    let docs = dir.path().join("docs");
    std::fs::create_dir_all(docs.join("sub")).unwrap();
    for i in 0..12 {
        std::fs::write(docs.join(format!("n{i}.md")), format!("Note {i} about the heart and blood.")).unwrap();
    }
    std::fs::write(docs.join("sub/deep.txt"), "Rust compiles to native code.").unwrap();
    std::fs::write(docs.join("skip.exe"), "binary").unwrap();
    std::fs::write(docs.join(".hidden.md"), "hidden").unwrap();
    let store = FileStore::open(&dir.path().join("store")).unwrap();
    let e = Engine::with_parts(Box::new(store), Some(Box::new(Bag("bag"))), dir.path(), &mut |_| {}).unwrap();
    let mut cfg = e.config();
    cfg.ingest.workers = 4;
    e.configure(cfg).unwrap();
    let files = e.files_in(&docs, true);
    assert_eq!(files.len(), 13);
    let seen = std::sync::atomic::AtomicUsize::new(0);
    let report = e.ingest_paths(&files, &|_, total, _| {
        assert_eq!(total, 13);
        seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });
    assert_eq!((report.files, report.chunks, report.errors.len()), (13, 13, 0));
    assert_eq!(seen.into_inner(), 13);
    // Ingesting again replaces, it does not duplicate.
    e.ingest_paths(&files, &|_, _, _| {});
    assert_eq!(e.status().unwrap().chunks, 13);
}

#[test]
fn a_cancelled_ingest_resumes_where_it_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let docs = dir.path().join("docs");
    std::fs::create_dir_all(&docs).unwrap();
    for i in 0..20 {
        std::fs::write(docs.join(format!("n{i:02}.md")), format!("Note {i} about the heart.")).unwrap();
    }
    let open = || {
        let store = FileStore::open(&dir.path().join("store")).unwrap();
        Engine::with_parts(Box::new(store), Some(Box::new(Bag("bag"))), dir.path(), &mut |_| {}).unwrap()
    };
    let e = open();
    let files = e.files_in(&docs, true);
    // Quit after 5 files (one worker, so the count is exact).
    let first = e.ingest_paths(&files, &|done, _, _| {
        if done == 5 {
            e.cancel_ingest();
        }
    });
    assert!(first.cancelled);
    assert_eq!(first.files, 5);
    drop(e);
    // The next start sees the rest and finishes it.
    let e = open();
    let left = e.interrupted();
    assert_eq!((left.len(), left[0].total, left[0].remaining.len()), (1, 20, 15));
    let rest = e.resume_interrupted(&|_, _, _| {});
    assert_eq!((rest.files, rest.cancelled), (15, false));
    assert!(e.interrupted().is_empty());
    assert_eq!(e.status().unwrap().chunks, 20);
}

/// Needs a Postgres with pgvector (and, for BM25, vchord_bm25 + pg_tokenizer):
/// `APP_TEST_PG_URL=postgres://... cargo test --features postgres,bm25`.
#[cfg(feature = "postgres")]
#[test]
fn hybrid_search_on_postgres() {
    let Ok(url) = std::env::var("APP_TEST_PG_URL") else {
        eprintln!("skipped: set APP_TEST_PG_URL");
        return;
    };
    std::env::set_var("APP_PG_URL", url);
    let dir = tempfile::tempdir().unwrap();
    let store = desktop_core::pgstore::PostgresStore::open(dir.path()).unwrap();
    let e = Engine::with_parts(Box::new(store), Some(Box::new(Bag("bag-pg"))), dir.path(), &mut |_| {}).unwrap();
    e.clear().unwrap();
    e.configure(desktop_core::Config::default()).unwrap();
    hybrid_checks(&e);
    e.clear().unwrap();
}
