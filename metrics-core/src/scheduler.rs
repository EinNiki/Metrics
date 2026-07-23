use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use chrono::Utc;
use tokio::sync::Mutex;
use crate::db::{self, Database};
use crate::loader::LoadedModule;

pub struct Scheduler {
    db: Database,
    active_runs: Arc<Mutex<HashSet<String>>>,
}

impl Scheduler {
    pub fn new(db: Database) -> Self {
        Self {
            db,
            active_runs: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn start(self) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                if let Err(e) = self.tick().await {
                    eprintln!("Scheduler tick error: {}", e);
                }
            }
        });
    }

    async fn tick(&self) -> Result<(), String> {
        let modules = db::get_modules(&self.db).await?;
        let backend_opt = db::get_global_settings(&self.db).await?;

        let backend = match backend_opt {
            Some(b) => b,
            None => {
                // If database backend is not configured, we cannot run collections.
                // Just return Ok until configured.
                return Ok(());
            }
        };

        let now = Utc::now();
        let active_runs = self.active_runs.clone();

        for mut module in modules {
            if !module.enabled {
                // If it was running but is now disabled, we update status to "paused"
                if module.status == "running" {
                    module.status = "paused".to_string();
                    db::save_module(&self.db, &module).await?;
                }
                continue;
            }

            // Check if it's already running
            let mut runs = active_runs.lock().await;
            if runs.contains(&module.id) {
                continue;
            }

            // Check if it's time to run
            let should_run = match module.last_run {
                None => true,
                Some(last) => {
                    let elapsed = now.signed_duration_since(last).num_seconds();
                    elapsed >= module.interval_secs as i64
                }
            };

            if should_run {
                runs.insert(module.id.clone());
                
                // Spawn execution task
                let db_clone = self.db.clone();
                let active_runs_clone = active_runs.clone();
                let backend_clone = backend.clone();

                tokio::spawn(async move {
                    let id = module.id.clone();
                    let lib_path = module.lib_path.clone();
                    let config_str = module.config.clone();

                    println!("Scheduler: Starting module {}", id);
                    let _ = db::add_log(&db_clone, &id, "info", "Starting metrics collection").await;

                    let run_result = unsafe {
                        match LoadedModule::load(&lib_path) {
                            Ok(loaded) => {
                                // Call collect
                                loaded.module.collect(&config_str, &backend_clone).await
                            }
                            Err(e) => Err(format!("Failed to load dynamic library: {}", e)),
                        }
                    };

                    // Re-fetch current record to avoid overwriting newer modifications
                    let mut current_module = match db::get_module(&db_clone, &id).await {
                        Ok(Some(m)) => m,
                        _ => module.clone(),
                    };

                    current_module.last_run = Some(Utc::now());

                    match run_result {
                        Ok(_) => {
                            current_module.status = "running".to_string();
                            current_module.last_error = None;
                            let _ = db::add_log(&db_clone, &id, "info", "Successfully collected metrics").await;
                        }
                        Err(err) => {
                            current_module.status = "error".to_string();
                            current_module.last_error = Some(err.clone());
                            let _ = db::add_log(&db_clone, &id, "error", &format!("Collection failed: {}", err)).await;
                        }
                    }

                    if let Err(e) = db::save_module(&db_clone, &current_module).await {
                        eprintln!("Failed to save module state for {}: {}", id, e);
                    }

                    // Remove from active runs
                    active_runs_clone.lock().await.remove(&id);
                });
            }
        }

        Ok(())
    }
}
