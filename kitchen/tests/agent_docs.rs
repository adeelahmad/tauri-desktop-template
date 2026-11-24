//! Generate apps from this repo's template (the folder as it is on disk) and check the
//! agent docs: no Liquid left anywhere in them, and `AGENTS.md` names exactly the features
//! that are on. Needs cargo-generate (skipped with a note when it is missing).

use kitchen::cli::{run, Args};
use kitchen::project::Project;
use kitchen::template::Template;
use std::path::{Path, PathBuf};

fn have(program: &str, args: &[&str]) -> bool {
    std::process::Command::new(program)
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    if dir.is_file() {
        out.push(dir.to_path_buf());
        return;
    }
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        files(&e.path(), out);
    }
}

/// Create `name` with `extra` arguments and check its agent docs.
fn check(work: &Path, repo: &Path, name: &str, extra: &[&str]) {
    let source = repo.display().to_string();
    let mut args = vec!["new", name, "--source", &source, "--no-git"];
    args.extend_from_slice(extra);
    let r = run(&Args::parse(args), work).unwrap();
    assert!(
        r.lines.iter().any(|l| l.contains("AGENTS.md")),
        "kitchen new should list the agent docs: {r}"
    );
    let app = work.join(name);

    let mut docs = Vec::new();
    for f in kitchen::doctor::AGENT_FILES {
        let p = app.join(f);
        assert!(p.exists(), "{f} was not generated");
        files(&p, &mut docs);
    }
    let skills: Vec<_> = docs
        .iter()
        .filter(|p| p.file_name().is_some_and(|n| n == "SKILL.md"))
        .collect();
    assert_eq!(skills.len(), 4, "{skills:?}");
    for p in &docs {
        let text = std::fs::read_to_string(p).unwrap();
        for leftover in ["{{", "}}", "{%", "%}"] {
            assert!(
                !text.contains(leftover),
                "{} has leftover Liquid {leftover:?}",
                p.display()
            );
        }
    }
    let mcp: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(app.join(".mcp.json")).unwrap()).unwrap();
    assert_eq!(mcp["mcpServers"]["kitchen"]["args"][0], "mcp");

    // Every yes/no feature that is in effect is named in AGENTS.md as `name`; no other is.
    let project = Project::find(&app).unwrap();
    let features = Template::open(&source, None).unwrap().features().unwrap();
    let agents = std::fs::read_to_string(app.join("AGENTS.md")).unwrap();
    let on = |n: &str| project.manifest.is_on(n);
    for f in features.iter().filter(|f| f.is_switch()) {
        let effective = on(&f.name) && f.requires.as_deref().is_none_or(on);
        let named = agents.contains(&format!("`{}`", f.name));
        assert_eq!(
            named,
            effective,
            "{name}: AGENTS.md {} `{}`, which is {}",
            if named { "names" } else { "does not name" },
            f.name,
            if effective { "on" } else { "off" }
        );
    }
    assert_eq!(
        mcp["mcpServers"].get(name).is_some(),
        on("mcp_server"),
        "the app's own MCP server is registered only with mcp_server"
    );
}

#[test]
fn agent_docs_render_cleanly_and_name_only_enabled_features() {
    if !have("cargo", &["generate", "--version"]) {
        eprintln!("skipped: needs cargo-generate");
        return;
    }
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let work = std::env::temp_dir().join(format!("kitchen-agents-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    check(
        &work,
        repo,
        "lean",
        &[
            "--disable",
            "embeddings,postgres,pdf,wizard,updater,installers,ipc_worker,python_example",
            "--set",
            "frontend=vanilla",
        ],
    );
    check(
        &work,
        repo,
        "rich",
        &[
            "--enable",
            "email,mcp_server,tts,plugins,tauri_dialog,local_llm,mobile",
        ],
    );
    let _ = std::fs::remove_dir_all(&work);
}
