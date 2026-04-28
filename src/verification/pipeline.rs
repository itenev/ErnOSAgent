//! Verification pipeline — orchestrates compile → test → browser → fix loop.
//!
//! The pipeline runs after the agent makes code changes. If any step fails,
//! it formats the error into a fix prompt for the coding agent. Bounded
//! retry prevents infinite loops.

use anyhow::{Context, Result};
use std::path::PathBuf;

use super::compiler_check::{self, CompileResult};
use super::browser::{self, BrowserCheckResult};

/// Configuration for a verification run.
#[derive(Debug, Clone)]
pub struct VerificationConfig {
    pub project_root: PathBuf,
    pub run_tests: bool,
    pub browser_url: Option<String>,
    pub browser_actions: Vec<browser::BrowserAction>,
    pub max_fix_attempts: usize,
}

impl Default for VerificationConfig {
    fn default() -> Self {
        Self {
            project_root: std::env::current_dir().unwrap_or_default(),
            run_tests: true,
            browser_url: None,
            browser_actions: Vec::new(),
            max_fix_attempts: 3,
        }
    }
}

/// Result of a full verification pass.
#[derive(Debug, Clone)]
pub struct VerificationResult {
    pub overall_pass: bool,
    pub build_result: CompileResult,
    pub test_result: Option<CompileResult>,
    pub browser_result: Option<BrowserCheckResult>,
    pub stage_failed: Option<VerificationStage>,
}

/// Which stage of verification failed.
#[derive(Debug, Clone, PartialEq)]
pub enum VerificationStage {
    Build,
    Tests,
    Browser,
}

/// Run the full verification pipeline: build → test → browser.
pub async fn run_verification(config: &VerificationConfig) -> Result<VerificationResult> {
    tracing::info!(root = %config.project_root.display(), "Starting verification pipeline");

    let build_result = compiler_check::check_build(&config.project_root).await
        .context("Build verification failed")?;

    if !build_result.success {
        tracing::warn!(errors = build_result.errors.len(), "Build failed");
        return Ok(make_failure(build_result, None, None, VerificationStage::Build));
    }

    let test_result = run_test_stage(config, &build_result).await?;
    if let Some(ref tr) = test_result {
        if !tr.success {
            return Ok(make_failure(build_result, Some(tr.clone()), None, VerificationStage::Tests));
        }
    }

    let browser_result = run_browser_stage(config, &build_result, &test_result).await?;
    if let Some(ref br) = browser_result {
        if !br.console_errors.is_empty() {
            return Ok(make_failure(build_result, test_result, Some(br.clone()), VerificationStage::Browser));
        }
    }

    tracing::info!("Verification pipeline passed");
    Ok(VerificationResult {
        overall_pass: true,
        build_result,
        test_result,
        browser_result,
        stage_failed: None,
    })
}

/// Create a failure result for the given stage.
fn make_failure(
    build_result: CompileResult,
    test_result: Option<CompileResult>,
    browser_result: Option<BrowserCheckResult>,
    stage: VerificationStage,
) -> VerificationResult {
    VerificationResult {
        overall_pass: false,
        build_result,
        test_result,
        browser_result,
        stage_failed: Some(stage),
    }
}

/// Run the test verification stage if configured.
async fn run_test_stage(config: &VerificationConfig, _build: &CompileResult) -> Result<Option<CompileResult>> {
    if !config.run_tests { return Ok(None); }
    let tr = compiler_check::check_tests(&config.project_root).await
        .context("Test verification failed")?;
    if !tr.success {
        tracing::warn!(failed = tr.test_summary.as_ref().map(|t| t.failed).unwrap_or(0), "Tests failed");
    }
    Ok(Some(tr))
}

/// Run the browser verification stage if a URL is configured.
async fn run_browser_stage(
    config: &VerificationConfig,
    _build: &CompileResult,
    _tests: &Option<CompileResult>,
) -> Result<Option<BrowserCheckResult>> {
    let Some(ref url) = config.browser_url else { return Ok(None); };

    // If browser actions are specified, use the interactive pipeline
    if !config.browser_actions.is_empty() {
        let results = browser::run_browser_actions(url, config.browser_actions.clone()).await
            .context("Browser action verification failed")?;
        // Merge all results — return the first failure or the last success
        let failed = results.iter().find(|r| !r.console_errors.is_empty());
        if let Some(fail) = failed {
            tracing::warn!(errors = fail.console_errors.len(), "Browser action errors");
            return Ok(Some(fail.clone()));
        }
        return Ok(results.into_iter().last());
    }

    let br = browser::check_url(url, 5).await
        .context("Browser verification failed")?;
    if !br.console_errors.is_empty() {
        tracing::warn!(errors = br.console_errors.len(), "Browser errors detected");
    }
    Ok(Some(br))
}

/// Format verification failures into a prompt for the coding agent to fix.
pub fn format_fix_prompt(result: &VerificationResult) -> String {
    let mut prompt = String::from(
        "[VERIFICATION FAILED — AUTO-FIX REQUIRED]\n\n\
         Your code changes did not pass verification. Fix the errors below, \
         then the system will re-verify automatically.\n\n"
    );

    match result.stage_failed {
        Some(VerificationStage::Build) => {
            prompt.push_str("## Build Failed\n\n");
            prompt.push_str(&format_build_errors(&result.build_result));
        }
        Some(VerificationStage::Tests) => {
            prompt.push_str("## Tests Failed\n\n");
            if let Some(ref tr) = result.test_result {
                prompt.push_str(&format_test_errors(tr));
            }
        }
        Some(VerificationStage::Browser) => {
            prompt.push_str("## Browser Validation Failed\n\n");
            if let Some(ref br) = result.browser_result {
                prompt.push_str(&browser::format_browser_report(&[br.clone()]));
            }
        }
        None => {
            prompt.push_str("(No specific failure stage recorded)\n");
        }
    }
    prompt
}

/// Format build errors for the fix prompt.
fn format_build_errors(result: &CompileResult) -> String {
    let mut out = String::new();
    out.push_str("```\n");
    for err in &result.errors {
        out.push_str(err);
        out.push('\n');
    }
    out.push_str("```\n\n");
    out.push_str("Full output:\n```\n");
    out.push_str(&result.raw_output);
    out.push_str("\n```\n");
    out
}

/// Format test failures for the fix prompt.
fn format_test_errors(result: &CompileResult) -> String {
    let mut out = String::new();
    if let Some(ref ts) = result.test_summary {
        out.push_str(&format!(
            "Passed: {}, Failed: {}, Ignored: {}\n\n",
            ts.passed, ts.failed, ts.ignored
        ));
        if !ts.failures.is_empty() {
            out.push_str("Failed tests:\n");
            for f in &ts.failures {
                out.push_str(&format!("  - {}\n", f));
            }
            out.push('\n');
        }
    }
    out.push_str("Full output:\n```\n");
    out.push_str(&result.raw_output);
    out.push_str("\n```\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = VerificationConfig::default();
        assert!(config.run_tests);
        assert!(config.browser_url.is_none());
        assert_eq!(config.max_fix_attempts, 3);
    }

    #[test]
    fn test_format_fix_prompt_build() {
        let result = VerificationResult {
            overall_pass: false,
            build_result: CompileResult {
                success: false,
                errors: vec!["error[E0308]: mismatched types".into()],
                warnings: vec![],
                test_summary: None,
                raw_output: "error output".into(),
            },
            test_result: None,
            browser_result: None,
            stage_failed: Some(VerificationStage::Build),
        };
        let prompt = format_fix_prompt(&result);
        assert!(prompt.contains("VERIFICATION FAILED"));
        assert!(prompt.contains("Build Failed"));
        assert!(prompt.contains("E0308"));
    }

    #[test]
    fn test_format_fix_prompt_tests() {
        let result = VerificationResult {
            overall_pass: false,
            build_result: CompileResult {
                success: true, errors: vec![], warnings: vec![],
                test_summary: None, raw_output: String::new(),
            },
            test_result: Some(CompileResult {
                success: false,
                errors: vec![],
                warnings: vec![],
                test_summary: Some(compiler_check::TestSummary {
                    passed: 10, failed: 2, ignored: 0,
                    failures: vec!["test_foo".into(), "test_bar".into()],
                }),
                raw_output: "test output".into(),
            }),
            browser_result: None,
            stage_failed: Some(VerificationStage::Tests),
        };
        let prompt = format_fix_prompt(&result);
        assert!(prompt.contains("Tests Failed"));
        assert!(prompt.contains("test_foo"));
        assert!(prompt.contains("Failed: 2"));
    }

    #[test]
    fn test_verification_stages() {
        assert_ne!(VerificationStage::Build, VerificationStage::Tests);
        assert_ne!(VerificationStage::Tests, VerificationStage::Browser);
    }
}
