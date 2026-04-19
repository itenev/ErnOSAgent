//! Browser tool — observable, DOM-aware web browsing with headed/headless modes.
//! Provides open, click, type, navigate, wait, extract, screenshot, evaluate, close.

use anyhow::{Context, Result};
use crate::config::BrowserConfig;
use chromiumoxide::Page;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

const MAX_PAGES: usize = 5;

/// Shared browser state — lazily initialized, with named page slots.
pub struct BrowserState {
    browser: Option<chromiumoxide::Browser>,
    _handle: Option<tokio::task::JoinHandle<()>>,
    pages: HashMap<String, Page>,
    next_page_id: usize,
    config: BrowserConfig,
}

impl BrowserState {
    pub fn new() -> Self {
        Self { browser: None, _handle: None, pages: HashMap::new(), next_page_id: 0, config: BrowserConfig::default() }
    }
    pub fn with_config(config: BrowserConfig) -> Self {
        Self { browser: None, _handle: None, pages: HashMap::new(), next_page_id: 0, config }
    }
}

/// Auto-detect Chrome/Chromium binary path.
fn find_chrome_binary() -> Option<String> {
    let candidates = [
        // macOS
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        // Linux
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
        "/usr/bin/chromium-browser",
        "/usr/bin/chromium",
        // Homebrew (macOS / Linux)
        "/opt/homebrew/bin/chromium",
        "/usr/local/bin/chromium",
        // Windows (common paths)
        "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
        "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
    ];
    for c in &candidates {
        if std::path::Path::new(c).exists() {
            return Some(c.to_string());
        }
    }
    // Fallback: check for headless-chrome download in user data
    if let Some(home) = dirs::home_dir() {
        let hc = home.join("Library/Application Support/headless-chrome");
        if hc.exists() {
            if let Ok(entries) = std::fs::read_dir(&hc) {
                for entry in entries.flatten() {
                    let chromium = entry.path().join("chrome-mac/Chromium.app/Contents/MacOS/Chromium");
                    if chromium.exists() {
                        return Some(chromium.to_string_lossy().to_string());
                    }
                }
            }
        }
    }
    None
}

/// Ensure browser is initialized, lazily starting Chrome.
async fn ensure_browser(state: &Arc<RwLock<BrowserState>>) -> Result<()> {
    let mut s = state.write().await;
    if s.browser.is_some() { return Ok(()); }

    let chrome = find_chrome_binary()
        .context("Chrome/Chromium not found. Install Google Chrome.")?;

    // Use a unique user-data-dir per process to prevent SingletonLock conflicts.
    let user_data_dir = std::env::temp_dir()
        .join(format!("ern-os-chrome-{}", std::process::id()));
    std::fs::create_dir_all(&user_data_dir).ok();

    let mut builder = chromiumoxide::BrowserConfig::builder()
        .chrome_executable(chrome)
        .user_data_dir(user_data_dir)
        .window_size(s.config.window_width, s.config.window_height);

    // Headed mode: visible Chrome window. Headless: invisible.
    if s.config.headed {
        builder = builder.with_head();
    } else {
        builder = builder.arg("--headless=new");
    }

    builder = builder
        .arg("--disable-gpu")
        .arg("--no-sandbox")
        .arg("--disable-dev-shm-usage")
        .arg("--no-first-run")
        .arg("--disable-extensions")
        .arg("--disable-default-apps")
        .arg("--disable-background-networking")
        .arg("--disable-sync")
        .arg("--disable-translate");

    let (browser, mut handler) = chromiumoxide::Browser::launch(
        builder.build().map_err(|e| anyhow::anyhow!("{}", e))?,
    )
    .await
    .context("Failed to launch Chrome")?;

    let handle = tokio::spawn(async move {
        while handler.next().await.is_some() {}
    });

    let mode = if s.config.headed { "headed" } else { "headless" };
    tracing::info!(mode, "Chrome browser initialized");
    s.browser = Some(browser);
    s._handle = Some(handle);
    Ok(())
}

// ─── Legacy API (kept for backwards compat) ───

/// Browse a URL and extract page content as markdown-formatted text.
pub async fn browse_url(
    state: &Arc<RwLock<BrowserState>>,
    url: &str,
) -> Result<String> {
    ensure_browser(state).await?;
    let s = state.read().await;
    let browser = s.browser.as_ref().context("Browser not initialized")?;

    let page = browser.new_page(url).await
        .context("Failed to open page")?;
    page.wait_for_navigation().await.ok();

    let title = page.get_title().await
        .unwrap_or_default()
        .unwrap_or_default();

    let content = page.evaluate(
        "document.body.innerText.substring(0, 8000)"
    ).await
    .context("Failed to extract page content")?
    .into_value::<String>()
    .unwrap_or_default();

    page.close().await.ok();

    Ok(format!("# {}\n\nURL: {}\n\n{}", title, url, content))
}

/// Take a screenshot of a URL, returning base64-encoded PNG.
pub async fn screenshot_url(
    state: &Arc<RwLock<BrowserState>>,
    url: &str,
) -> Result<String> {
    ensure_browser(state).await?;
    let s = state.read().await;
    let browser = s.browser.as_ref().context("Browser not initialized")?;

    let page = browser.new_page(url).await
        .context("Failed to open page")?;
    page.wait_for_navigation().await.ok();

    let screenshot = page.screenshot(
        chromiumoxide::page::ScreenshotParams::builder()
            .full_page(true)
            .build(),
    ).await.context("Failed to capture screenshot")?;

    page.close().await.ok();

    use base64::Engine;
    Ok(base64::engine::general_purpose::STANDARD.encode(&screenshot))
}

// ─── Interactive Browser API ───

/// Dispatch a browser action by name.
pub async fn execute_action(
    state: &Arc<RwLock<BrowserState>>,
    args: &serde_json::Value,
) -> Result<String> {
    let action = args["action"].as_str().unwrap_or("open");
    match action {
        "open" => action_open(state, args).await,
        "click" => action_click(state, args).await,
        "type" => action_type(state, args).await,
        "navigate" => action_navigate(state, args).await,
        "wait" => action_wait(state, args).await,
        "extract" => action_extract(state, args).await,
        "screenshot" => action_screenshot(state, args).await,
        "evaluate" => action_evaluate(state, args).await,
        "close" => action_close(state, args).await,
        "list" => action_list(state).await,
        other => anyhow::bail!("Unknown browser action: {}", other),
    }
}

/// Open a new page and return its page_id.
async fn action_open(state: &Arc<RwLock<BrowserState>>, args: &serde_json::Value) -> Result<String> {
    ensure_browser(state).await?;
    let url = args["url"].as_str().unwrap_or("about:blank");
    let mut s = state.write().await;

    if s.pages.len() >= MAX_PAGES {
        anyhow::bail!("Maximum {} concurrent pages reached. Close a page first.", MAX_PAGES);
    }

    let browser = s.browser.as_ref().context("Browser not initialized")?;
    let page = browser.new_page(url).await
        .context("Failed to open page")?;
    page.wait_for_navigation().await.ok();

    let title = page.get_title().await.unwrap_or_default().unwrap_or_default();
    let page_id = format!("page_{}", s.next_page_id);
    let context = get_page_context(&page).await;
    s.next_page_id += 1;
    s.pages.insert(page_id.clone(), page);

    tracing::info!(page_id = %page_id, url = %url, "Browser page opened");
    Ok(format!("Opened page '{}': {} — {}{}", page_id, url, title, context))
}

/// Get a page by ID from state (read lock).
fn get_page<'a>(state: &'a tokio::sync::RwLockReadGuard<BrowserState>, args: &serde_json::Value) -> Result<&'a Page> {
    let page_id = args["page_id"].as_str().unwrap_or("page_0");
    state.pages.get(page_id)
        .with_context(|| format!("Page '{}' not found. Open pages: {:?}", page_id, state.pages.keys().collect::<Vec<_>>()))
}

/// Extract a DOM summary from the page: title, URL, and interactive elements.
/// This gives the model awareness of what's actually on the page so it can
/// choose valid selectors instead of guessing blindly.
async fn get_page_context(page: &Page) -> String {
    let js = r#"(() => {
        try {
            const title = document.title || '';
            const url = location.href || '';
            const parts = [];
            const headings = [...document.querySelectorAll('h1,h2,h3')].slice(0, 10);
            for (const h of headings) {
                parts.push('  <' + h.tagName.toLowerCase() + '>' + (h.innerText||'').trim().substring(0, 80) + '</' + h.tagName.toLowerCase() + '>');
            }
            const links = [...document.querySelectorAll('a[href]')].slice(0, 20);
            for (const a of links) {
                parts.push('  <a href="' + a.href + '">' + (a.innerText||'').trim().substring(0, 50) + '</a>');
            }
            const buttons = [...document.querySelectorAll('button, input[type=submit], input[type=button]')].slice(0, 10);
            for (const b of buttons) {
                const tag = b.tagName.toLowerCase();
                parts.push('  <' + tag + '>' + ((b.innerText||b.value||'')).trim() + '</' + tag + '>');
            }
            const inputs = [...document.querySelectorAll('input:not([type=hidden]):not([type=submit]):not([type=button]), textarea, select')].slice(0, 10);
            for (const i of inputs) {
                const tag = i.tagName.toLowerCase();
                parts.push('  <' + tag + ' type="' + (i.type||'') + '" name="' + (i.name||'') + '" id="' + (i.id||'') + '">');
            }
            return JSON.stringify({
                title: title,
                url: url,
                links: links.length,
                buttons: buttons.length,
                inputs: inputs.length,
                elements: parts.join('\n')
            });
        } catch(e) { return '{}'; }
    })()"#;

    match page.evaluate(js).await {
        Ok(val) => {
            let raw = val.into_value::<String>().unwrap_or_default();
            let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
            let elements = parsed["elements"].as_str().unwrap_or("(empty page)");
            if elements.is_empty() || elements == "(empty page)" {
                format!("\n\n--- Page Context ---\nTitle: {}\nURL: {}\nLinks: 0 | Buttons: 0 | Inputs: 0\nDOM: (empty page — no interactive elements)",
                    parsed["title"].as_str().unwrap_or(""),
                    parsed["url"].as_str().unwrap_or(""))
            } else {
                format!("\n\n--- Page Context ---\nTitle: {}\nURL: {}\nLinks: {} | Buttons: {} | Inputs: {}\nDOM:\n{}",
                    parsed["title"].as_str().unwrap_or(""),
                    parsed["url"].as_str().unwrap_or(""),
                    parsed["links"], parsed["buttons"], parsed["inputs"],
                    elements)
            }
        }
        Err(_) => String::new()
    }
}

/// Click an element by CSS selector. Returns page context on failure.
async fn action_click(state: &Arc<RwLock<BrowserState>>, args: &serde_json::Value) -> Result<String> {
    let selector = args["selector"].as_str().context("'selector' required for click")?;
    let s = state.read().await;
    let page = get_page(&s, args)?;

    match page.find_element(selector).await {
        Ok(element) => {
            element.click().await
                .with_context(|| format!("Failed to click: {}", selector))?;
            let context = get_page_context(page).await;
            Ok(format!("Clicked: {}{}", selector, context))
        }
        Err(_) => {
            let context = get_page_context(page).await;
            anyhow::bail!(
                "Element '{}' not found on this page.{}\n\nUse the DOM above to choose a valid selector.",
                selector, context
            )
        }
    }
}

/// Type text into an element.
async fn action_type(state: &Arc<RwLock<BrowserState>>, args: &serde_json::Value) -> Result<String> {
    let selector = args["selector"].as_str().context("'selector' required for type")?;
    let text = args["text"].as_str().context("'text' required for type")?;
    let s = state.read().await;
    let page = get_page(&s, args)?;

    match page.find_element(selector).await {
        Ok(element) => {
            element.click().await.ok();
            element.type_str(text).await
                .with_context(|| format!("Failed to type into: {}", selector))?;
            Ok(format!("Typed '{}' into {}", text, selector))
        }
        Err(_) => {
            let context = get_page_context(page).await;
            anyhow::bail!(
                "Element '{}' not found for typing.{}\n\nUse the DOM above to choose a valid selector.",
                selector, context
            )
        }
    }
}

/// Navigate an existing page to a new URL.
async fn action_navigate(state: &Arc<RwLock<BrowserState>>, args: &serde_json::Value) -> Result<String> {
    let url = args["url"].as_str().context("'url' required for navigate")?;
    let s = state.read().await;
    let page = get_page(&s, args)?;

    page.goto(url).await
        .with_context(|| format!("Failed to navigate to: {}", url))?;
    page.wait_for_navigation().await.ok();

    let title = page.get_title().await.unwrap_or_default().unwrap_or_default();
    let context = get_page_context(page).await;
    Ok(format!("Navigated to: {} — {}{}", url, title, context))
}

/// Wait for an element to appear.
async fn action_wait(state: &Arc<RwLock<BrowserState>>, args: &serde_json::Value) -> Result<String> {
    let selector = args["selector"].as_str().context("'selector' required for wait")?;
    let timeout_ms = args["timeout_ms"].as_u64().unwrap_or(5000);
    let s = state.read().await;
    let page = get_page(&s, args)?;

    let js = format!(
        r#"new Promise((resolve, reject) => {{
            const start = Date.now();
            const check = () => {{
                if (document.querySelector('{}')) resolve(true);
                else if (Date.now() - start > {}) reject('timeout');
                else setTimeout(check, 100);
            }};
            check();
        }})"#,
        selector.replace('\'', "\\'"), timeout_ms
    );

    page.evaluate(js).await
        .with_context(|| format!("Wait for '{}' timed out after {}ms", selector, timeout_ms))?;

    Ok(format!("Element found: {}", selector))
}

/// Extract text content from an element.
async fn action_extract(state: &Arc<RwLock<BrowserState>>, args: &serde_json::Value) -> Result<String> {
    let selector = args["selector"].as_str().context("'selector' required for extract")?;
    let attribute = args["attribute"].as_str();
    let s = state.read().await;
    let page = get_page(&s, args)?;

    let js = if let Some(attr) = attribute {
        format!("document.querySelector('{}')?.getAttribute('{}')", selector, attr)
    } else {
        format!("document.querySelector('{}')?.innerText?.substring(0, 4000)", selector)
    };

    let result = page.evaluate(js).await
        .context("Extract evaluation failed")?
        .into_value::<serde_json::Value>()
        .unwrap_or(serde_json::Value::Null);

    Ok(serde_json::to_string_pretty(&result)?)
}

/// Take a screenshot and save to data/images/.
async fn action_screenshot(state: &Arc<RwLock<BrowserState>>, args: &serde_json::Value) -> Result<String> {
    let s = state.read().await;
    let page = get_page(&s, args)?;

    let screenshot = page.screenshot(
        chromiumoxide::page::ScreenshotParams::builder()
            .full_page(true)
            .build(),
    ).await.context("Screenshot failed")?;

    let id = uuid::Uuid::new_v4().to_string();
    let filename = format!("{}.png", id);
    let dir = std::path::PathBuf::from("data/images");
    std::fs::create_dir_all(&dir).ok();
    std::fs::write(dir.join(&filename), &screenshot)
        .context("Failed to save screenshot")?;

    Ok(format!("![screenshot](/api/images/{})", filename))
}

/// Evaluate JavaScript on a page.
async fn action_evaluate(state: &Arc<RwLock<BrowserState>>, args: &serde_json::Value) -> Result<String> {
    let script = args["script"].as_str().context("'script' required for evaluate")?;
    let s = state.read().await;
    let page = get_page(&s, args)?;

    let result = page.evaluate(script).await
        .context("JavaScript evaluation failed")?
        .into_value::<serde_json::Value>()
        .unwrap_or(serde_json::Value::Null);

    Ok(serde_json::to_string_pretty(&result)?)
}

/// Close a page and release resources.
async fn action_close(state: &Arc<RwLock<BrowserState>>, args: &serde_json::Value) -> Result<String> {
    let page_id = args["page_id"].as_str().unwrap_or("page_0");
    let mut s = state.write().await;

    if let Some(page) = s.pages.remove(page_id) {
        page.close().await.ok();
        tracing::info!(page_id = %page_id, "Browser page closed");
        Ok(format!("Closed page '{}'", page_id))
    } else {
        anyhow::bail!("Page '{}' not found", page_id)
    }
}

/// List all open pages.
async fn action_list(state: &Arc<RwLock<BrowserState>>) -> Result<String> {
    let s = state.read().await;
    if s.pages.is_empty() {
        return Ok("No pages open.".to_string());
    }
    let list: Vec<String> = s.pages.keys().cloned().collect();
    Ok(format!("Open pages: {}", list.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_chrome_binary() {
        let binary = find_chrome_binary();
        if let Some(path) = &binary {
            assert!(std::path::Path::new(path).exists());
        }
    }

    #[test]
    fn test_browser_state_new() {
        let state = BrowserState::new();
        assert!(state.browser.is_none());
        assert!(state.pages.is_empty());
    }

    #[test]
    fn test_max_pages_constant() {
        assert!(MAX_PAGES >= 3);
    }
}
