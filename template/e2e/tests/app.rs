//! Feature tests: the real app, driven through its window. Run with `kitchen test --feature`
//! (see the crate docs for the manual steps). Each test skips itself, saying why, on a
//! machine that cannot run it.

mod webdriver;

/// Add a note, search for it, and see the hit: the whole path from the window through the
/// Tauri commands and the backend to the store and back.
#[test]
#[ignore = "feature test: needs the built app and tauri-driver; run `kitchen test --feature`"]
fn add_text_then_find_it() -> webdriver::Result<()> {
    let binary = webdriver::app_binary("{{project-name}}");
    if let Some(why) = webdriver::skip_reason(&binary) {
        eprintln!("skipped: {why}");
        return Ok(());
    }
    let app = webdriver::App::launch(&binary)?;

    app.find("textarea")?.type_text("The heart pumps blood through the arteries to the body.")?;
    app.find_xpath("//button[normalize-space()='Add text']")?.click()?;
    app.find_xpath("//*[contains(text(), 'Added')]")?;

    app.find("input[placeholder='What are you looking for?']")?.type_text("heart pumps blood")?;
    app.find_xpath("//button[normalize-space()='Search']")?.click()?;
    let hit = app.find("#results li")?.text()?;
    assert!(hit.contains("heart"), "first hit: {hit}");
    Ok(())
}
