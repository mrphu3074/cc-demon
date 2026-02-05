//! Session manager with message queue and background tasks.
//!
//! Handles message queueing, sequential processing, auto-compaction,
//! and health monitoring with auto-restart.

use anyhow::{Context, Result};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time::interval;

use super::tmux::TmuxSession;
use super::{ClaudeSession, MessageRequest, SessionConfig};

/// Manages a persistent Claude Code session.
///
/// Provides message queueing for sequential processing, automatic compaction,
/// and health monitoring with auto-restart on failure.
pub struct SessionManager {
    session: Arc<Mutex<TmuxSession>>,
    request_tx: mpsc::Sender<MessageRequest>,
    config: SessionConfig,
    #[allow(dead_code)]
    shutdown_tx: Option<mpsc::Sender<()>>,
}

impl SessionManager {
    /// Create a new session manager with the given configuration.
    ///
    /// This will:
    /// 1. Create a tmux session with Claude running inside
    /// 2. Start a worker loop to process messages sequentially
    /// 3. Start a compaction task that runs at the configured interval
    /// 4. Start a health monitor that auto-restarts on failure
    pub async fn new(config: SessionConfig) -> Result<Self> {
        tracing::info!(
            "Initializing SessionManager with session '{}'",
            config.session_name
        );

        // Create the tmux session
        let session = TmuxSession::new(config.clone())
            .await
            .context("Failed to create tmux session")?;
        let session = Arc::new(Mutex::new(session));

        // Create message queue (bounded to prevent memory exhaustion)
        let (request_tx, request_rx) = mpsc::channel::<MessageRequest>(100);

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>(1);

        let manager = Self {
            session: session.clone(),
            request_tx,
            config: config.clone(),
            shutdown_tx: Some(shutdown_tx),
        };

        // Spawn background tasks
        manager.spawn_worker_loop(request_rx);
        manager.spawn_compaction_task(shutdown_rx);
        manager.spawn_health_monitor(config.clone());

        tracing::info!("SessionManager initialized successfully");
        Ok(manager)
    }

    /// Send a message to Claude and wait for the response.
    ///
    /// Messages are queued and processed sequentially.
    pub async fn send_message(&self, prompt: &str) -> Result<String> {
        let (response_tx, response_rx) = oneshot::channel();

        let request = MessageRequest {
            prompt: prompt.to_string(),
            response_tx,
        };

        self.request_tx
            .send(request)
            .await
            .map_err(|_| anyhow::anyhow!("Session manager is shut down"))?;

        response_rx
            .await
            .map_err(|_| anyhow::anyhow!("Session worker dropped response"))?
    }

    /// Spawn the worker loop that processes messages sequentially.
    fn spawn_worker_loop(&self, mut request_rx: mpsc::Receiver<MessageRequest>) {
        let session = self.session.clone();

        tokio::spawn(async move {
            tracing::info!("Worker loop started");

            while let Some(request) = request_rx.recv().await {
                tracing::debug!("Processing message: {}...", &request.prompt[..request.prompt.len().min(50)]);

                let session_guard = session.lock().await;
                let result = session_guard.send_message(&request.prompt).await;
                drop(session_guard);

                // Send result back (ignore error if receiver dropped)
                let _ = request.response_tx.send(result);
            }

            tracing::info!("Worker loop ended");
        });
    }

    /// Spawn the compaction task that runs periodically.
    fn spawn_compaction_task(&self, mut shutdown_rx: mpsc::Receiver<()>) {
        let session = self.session.clone();
        let compact_interval = Duration::from_secs(self.config.compact_interval_secs);

        tokio::spawn(async move {
            tracing::info!(
                "Compaction task started (interval: {}s)",
                compact_interval.as_secs()
            );

            let mut ticker = interval(compact_interval);
            ticker.tick().await; // Skip first immediate tick

            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        tracing::info!("Running scheduled compaction");

                        let session_guard = session.lock().await;
                        if let Err(e) = session_guard.compact().await {
                            tracing::error!("Compaction failed: {}", e);
                        } else {
                            tracing::info!("Compaction completed successfully");
                        }
                        drop(session_guard);
                    }
                    _ = shutdown_rx.recv() => {
                        tracing::info!("Compaction task shutting down");
                        break;
                    }
                }
            }
        });
    }

    /// Spawn the health monitor that checks session liveness and auto-restarts.
    fn spawn_health_monitor(&self, config: SessionConfig) {
        let session = self.session.clone();
        let max_restart_attempts = config.max_restart_attempts;

        tokio::spawn(async move {
            tracing::info!("Health monitor started (check every 10s)");

            let mut ticker = interval(Duration::from_secs(10));
            let mut consecutive_failures = 0u32;

            loop {
                ticker.tick().await;

                let session_guard = session.lock().await;
                let is_alive = session_guard.is_alive().await;
                drop(session_guard);

                if !is_alive {
                    consecutive_failures += 1;
                    tracing::warn!(
                        "Session not alive (failure {}/{})",
                        consecutive_failures,
                        max_restart_attempts
                    );

                    if consecutive_failures <= max_restart_attempts {
                        tracing::info!("Attempting to restart session...");

                        let session_guard = session.lock().await;
                        if let Err(e) = session_guard.restart().await {
                            tracing::error!("Failed to restart session: {}", e);
                        } else {
                            tracing::info!("Session restarted successfully");
                            consecutive_failures = 0;
                        }
                        drop(session_guard);
                    } else {
                        tracing::error!(
                            "Max restart attempts ({}) exceeded, health monitor giving up",
                            max_restart_attempts
                        );
                        // Continue monitoring in case manual intervention fixes it
                    }
                } else {
                    // Reset failure counter on successful check
                    if consecutive_failures > 0 {
                        tracing::info!("Session recovered, resetting failure counter");
                        consecutive_failures = 0;
                    }
                }
            }
        });
    }

    /// Gracefully shutdown the session manager.
    #[allow(dead_code)]
    pub async fn shutdown(&mut self) -> Result<()> {
        tracing::info!("Shutting down SessionManager");

        // Signal shutdown to background tasks
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(()).await;
        }

        // Kill the tmux session
        let session = self.session.lock().await;
        let _ = session.restart().await; // This will kill the session

        Ok(())
    }
}

impl Drop for SessionManager {
    fn drop(&mut self) {
        // Note: Can't do async cleanup in Drop
        // The tmux session will remain until explicitly killed or manager restarts
        tracing::debug!("SessionManager dropped");
    }
}
