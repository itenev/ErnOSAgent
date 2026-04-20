// Ern-OS — High-performance, model-neutral Rust AI agent engine
// Created by @mettamazza (github.com/mettamazza)
// License: MIT
//! Entry point — initialises config, logging, provider, memory, launches
//! llama-server subprocess, and starts the WebUI hub.

use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::sync::RwLock;

#[tokio::main]
async fn main() -> Result<()> {
    let config = ern_os::config::AppConfig::load()
        .context("Failed to load configuration")?;

    ern_os::logging::init(&config.general.data_dir)
        .context("Failed to initialise logging")?;

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        data_dir = %config.general.data_dir.display(),
        provider = %config.general.active_provider,
        "Ern-OS starting"
    );

    let _server_handle = maybe_start_llama_server(&config).await?;
    let provider = create_and_verify_provider(&config).await?;
    let model_spec = detect_model_spec(&provider).await?;
    let state = build_app_state(&config, provider, model_spec)?;

    let _scheduler = ern_os::scheduler::start(state.clone());

    // Auto-start Kokoro TTS server if configured and not already running
    services::maybe_start_kokoro(&config).await;

    // Auto-start Flux image generation server
    services::maybe_start_flux(&config).await;

    // Auto-start code-server (VS Code IDE) if enabled
    services::maybe_start_code_server(&config).await;

    // Check for post-recompile resume state — store in AppState for WebSocket delivery
    let resume_msg = services::check_recompile_resume(&config);
    if resume_msg.is_some() {
        *state.resume_message.write().await = resume_msg;
    }

    // Register platform adapters and auto-connect configured ones
    {
        let mut reg = state.platforms.write().await;
        #[cfg(feature = "discord")]
        reg.register(Box::new(ern_os::platform::discord::DiscordAdapter::new(config.discord.clone())));
        #[cfg(feature = "telegram")]
        reg.register(Box::new(ern_os::platform::telegram::TelegramAdapter::new(config.telegram.clone())));
        reg.connect_all().await;
    }

    // Start platform router (forwards platform messages to hub)
    let router_registry = state.platforms.clone();
    let hub_port = config.web.port;
    tokio::spawn(async move {
        ern_os::platform::router::start_platform_router(router_registry, hub_port).await;
    });

    launch_webui(state, &config).await
}

/// Conditionally start llama-server subprocess.
async fn maybe_start_llama_server(
    config: &ern_os::config::AppConfig,
) -> Result<Option<tokio::process::Child>> {
    if config.general.active_provider != "llamacpp" {
        return Ok(None);
    }

    let llama_config = &config.llamacpp;
    let provider = ern_os::provider::llamacpp::LlamaCppProvider::new(llama_config);
    let args = provider.build_server_args();

    tracing::info!(
        binary = %llama_config.server_binary,
        model = %llama_config.model_path,
        port = llama_config.port,
        "Starting llama-server"
    );

    let child = tokio::process::Command::new(&llama_config.server_binary)
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context(format!(
            "Failed to start llama-server at '{}'. Is it installed?",
            llama_config.server_binary
        ))?;

    tracing::info!(pid = child.id().unwrap_or(0), "llama-server started");
    Ok(Some(child))
}

/// Create provider and verify health with retries.
async fn create_and_verify_provider(
    config: &ern_os::config::AppConfig,
) -> Result<Arc<dyn ern_os::provider::Provider>> {
    let provider: Arc<dyn ern_os::provider::Provider> = Arc::from(
        ern_os::provider::create_provider(config)
            .context("Failed to create provider")?
    );

    tracing::info!("Waiting for provider health check...");
    let mut retries = 0;
    while !provider.health().await {
        retries += 1;
        if retries > 60 {
            anyhow::bail!("Provider failed health check after 60 attempts. Is the server running?");
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        if retries % 10 == 0 {
            tracing::info!(retries, "Still waiting for provider...");
        }
    }
    tracing::info!("Provider healthy");
    Ok(provider)
}

/// Detect model specification from the active provider.
async fn detect_model_spec(
    provider: &Arc<dyn ern_os::provider::Provider>,
) -> Result<ern_os::model::ModelSpec> {
    let spec = provider.get_model_spec().await
        .context("Failed to get model spec from provider")?;
    tracing::info!(
        model = %spec.name, context_length = spec.context_length,
        vision = spec.supports_vision, "Model detected"
    );
    Ok(spec)
}

/// Build the shared application state from all initialised components.
fn build_app_state(
    config: &ern_os::config::AppConfig,
    provider: Arc<dyn ern_os::provider::Provider>,
    model_spec: ern_os::model::ModelSpec,
) -> Result<ern_os::web::state::AppState> {
    let data_dir = config.general.data_dir.clone();
    let (memory, golden_buffer, rejection_buffer, scheduler, agents, teams) =
        init_subsystems(&data_dir)?;

    Ok(ern_os::web::state::AppState {
        config: Arc::new(config.clone()),
        model_spec: Arc::new(model_spec),
        memory: Arc::new(RwLock::new(memory)),
        sessions: Arc::new(RwLock::new(
            ern_os::session::SessionManager::new(&data_dir.join("sessions"))
                .context("Failed to initialise session manager")?,
        )),
        provider,
        golden_buffer: Arc::new(RwLock::new(golden_buffer)),
        rejection_buffer: Arc::new(RwLock::new(rejection_buffer)),
        scheduler: Arc::new(RwLock::new(scheduler)),
        agents: Arc::new(RwLock::new(agents)),
        teams: Arc::new(RwLock::new(teams)),
        browser: Arc::new(RwLock::new(ern_os::tools::browser_tool::BrowserState::with_config(config.browser.clone()))),
        platforms: Arc::new(RwLock::new(ern_os::platform::registry::PlatformRegistry::new())),
        mutable_config: Arc::new(RwLock::new(config.clone())),
        resume_message: Arc::new(RwLock::new(None)),
        sae: Arc::new(RwLock::new(load_sae_weights())),
    })
}

/// Initialise all subsystems from the data directory.
fn init_subsystems(data_dir: &std::path::Path) -> Result<(
    ern_os::memory::MemoryManager,
    ern_os::learning::buffers::GoldenBuffer,
    ern_os::learning::buffers_rejection::RejectionBuffer,
    ern_os::scheduler::store::JobStore,
    ern_os::agents::AgentRegistry,
    ern_os::agents::teams::TeamRegistry,
)> {
    let memory = ern_os::memory::MemoryManager::new(data_dir)
        .context("Failed to initialise memory system")?;
    tracing::info!(status = %memory.status_summary(), "Memory system initialised");

    let golden = ern_os::learning::buffers::GoldenBuffer::open(
        &data_dir.join("golden_buffer.json"), 500,
    ).context("Failed to initialise golden buffer")?;

    let rejection = ern_os::learning::buffers_rejection::RejectionBuffer::open(
        &data_dir.join("rejection_buffer.json"),
    ).context("Failed to initialise rejection buffer")?;

    tracing::info!(golden = golden.count(), rejection = rejection.count(), "Training buffers initialised");

    let scheduler = ern_os::scheduler::store::JobStore::load(data_dir)
        .context("Failed to initialise scheduler")?;
    tracing::info!(jobs = scheduler.jobs.len(), "Scheduler initialised");

    let agents = ern_os::agents::AgentRegistry::new(data_dir)
        .context("Failed to initialise agent registry")?;
    let teams = ern_os::agents::teams::TeamRegistry::new(data_dir)
        .context("Failed to initialise team registry")?;

    Ok((memory, golden, rejection, scheduler, agents, teams))
}

/// Load SAE weights from models/sae/ directory at startup.
fn load_sae_weights() -> Option<ern_os::interpretability::sae::SparseAutoencoder> {
    let sae_dir = std::path::Path::new("models/sae");
    if !sae_dir.exists() {
        tracing::debug!("No models/sae/ directory — SAE interpretability disabled");
        return None;
    }

    // Find the first .safetensors file
    let entry = std::fs::read_dir(sae_dir).ok()?.find_map(|e| {
        let e = e.ok()?;
        if e.path().extension().map_or(false, |ext| ext == "safetensors") {
            Some(e.path())
        } else {
            None
        }
    });

    match entry {
        Some(path) => {
            tracing::info!(path = %path.display(), "Loading SAE weights...");
            match ern_os::interpretability::sae::SparseAutoencoder::load_safetensors(&path) {
                Ok(sae) => {
                    tracing::info!(
                        features = sae.num_features,
                        model_dim = sae.model_dim,
                        "SAE loaded — interpretability encode active"
                    );
                    Some(sae)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to load SAE weights");
                    None
                }
            }
        }
        None => {
            tracing::debug!("No .safetensors files in models/sae/ — SAE disabled");
            None
        }
    }
}

/// Start the WebUI hub and optionally open browser.
async fn launch_webui(
    state: ern_os::web::state::AppState,
    config: &ern_os::config::AppConfig,
) -> Result<()> {
    let addr = format!("0.0.0.0:{}", config.web.port);
    tracing::info!(addr = %addr, "Starting WebUI hub");

    if config.web.open_browser {
        let url = format!("http://localhost:{}", config.web.port);
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            let _ = services::open_browser(&url);
        });
    }

    ern_os::web::server::run(state, &addr).await
        .context("WebUI server failed")
}

mod services;

