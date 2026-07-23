use axum::{
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use tower_http::cors::CorsLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod db;
mod loader;
mod scheduler;
mod api;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing / logging
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,metrics_core=debug".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    println!("Starting Metrics Core Engine...");

    // Configurations from environment
    let port = std::env::var("PORT")
        .unwrap_or_else(|_| "3000".to_string())
        .parse::<u16>()
        .unwrap_or(3000);

    let admin_password = std::env::var("ADMIN_PASSWORD").unwrap_or_else(|_| "admin123".to_string());
    let db_dir = std::env::var("DB_PATH").unwrap_or_else(|_| "./data".to_string());

    // Create database folder if missing
    std::fs::create_dir_all(&db_dir)?;
    let db_path = format!("{}/metrics.skv", db_dir.trim_end_matches('/'));

    // Initialize database
    println!("Connecting to SurrealKV database at: {}", db_path);
    let db = db::init_db(&db_path).await?;

    // Create default admin user
    db::create_admin_user_if_missing(&db, &admin_password).await?;

    // Start Scheduler
    println!("Booting up metrics task scheduler...");
    let scheduler = scheduler::Scheduler::new(db.clone());
    scheduler.start();

    // CORS configuration
    let cors = CorsLayer::permissive();

    // App Routing
    let app = Router::new()
        // Embedded Frontend SPA
        .route("/", get(index_handler))
        // API Authentication
        .route("/api/auth/login", post(api::login_handler))
        // API Settings
        .route("/api/settings", get(api::get_settings_handler).post(api::save_settings_handler))
        // API Modules
        .route("/api/modules", get(api::list_modules_handler))
        .route("/api/modules/:id/config", post(api::save_module_config_handler))
        .route("/api/modules/:id/toggle", post(api::toggle_module_handler))
        .route("/api/modules/:id/logs", get(api::get_module_logs_handler))
        // API Store
        .route("/api/store", get(api::list_store_modules_handler))
        .route("/api/store/install", post(api::install_module_handler))
        .route("/api/store/uninstall", post(api::uninstall_module_handler))
        .layer(cors)
        .with_state(db);

    // Run Server
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    println!("Web UI listening on: http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    println!("Metrics core server has terminated.");
    Ok(())
}

async fn index_handler() -> impl axum::response::IntoResponse {
    axum::response::Html(include_str!("static/index.html"))
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    println!("SIGTERM/SIGINT received. Shutting down gracefully...");
}
