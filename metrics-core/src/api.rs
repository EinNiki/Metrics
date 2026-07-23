use axum::{
    async_trait,
    extract::{FromRequestParts, Path, State},
    http::{request::Parts, StatusCode},
    Json,
};
use std::collections::HashMap;
use std::sync::OnceLock;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use crate::db::{self, Database, ModuleRecord};
use crate::loader::LoadedModule;
use metrics_api::DbBackend;

static JWT_SECRET: OnceLock<Vec<u8>> = OnceLock::new();

pub fn get_jwt_secret() -> &'static [u8] {
    JWT_SECRET.get_or_init(|| {
        let mut key = vec![0u8; 32];
        rand::thread_rng().fill_bytes(&mut key);
        key
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TokenClaims {
    pub sub: String,
    pub role: String,
    pub exp: i64,
}

#[allow(dead_code)]
pub struct Claims {
    pub username: String,
    pub role: String,
}

#[async_trait]
impl<S> FromRequestParts<S> for Claims
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, Json<serde_json::Value>);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get("Authorization")
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({ "error": "Missing Authorization header" })),
                )
            })?;

        if !auth_header.starts_with("Bearer ") {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "Invalid Authorization header format" })),
            ));
        }

        let token = &auth_header[7..];
        let key = jsonwebtoken::DecodingKey::from_secret(get_jwt_secret());
        let validation = jsonwebtoken::Validation::default();

        let token_data = jsonwebtoken::decode::<TokenClaims>(token, &key, &validation)
            .map_err(|_| {
                (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({ "error": "Invalid or expired token" })),
                )
            })?;

        Ok(Claims {
            username: token_data.claims.sub,
            role: token_data.claims.role,
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct LoginPayload {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub role: String,
}

pub async fn login_handler(
    State(db): State<Database>,
    Json(payload): Json<LoginPayload>,
) -> Result<Json<LoginResponse>, (StatusCode, Json<serde_json::Value>)> {
    let user = db::get_user(&db, &payload.username)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e }))))?
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid credentials" }))))?;

    use argon2::{
        password_hash::{PasswordHash, PasswordVerifier},
        Argon2,
    };

    let parsed_hash = match PasswordHash::new(&user.password_hash) {
        Ok(hash) => hash,
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": format!("Invalid password hash format: {}", e) })))),
    };

    let password_ok = Argon2::default()
        .verify_password(payload.password.as_bytes(), &parsed_hash)
        .is_ok();

    if !password_ok {
        return Err((StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Invalid credentials" }))));
    }

    let expiration = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::hours(24))
        .expect("valid timestamp")
        .timestamp();

    let claims = TokenClaims {
        sub: user.username.clone(),
        role: user.role.clone(),
        exp: expiration,
    };

    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(get_jwt_secret()),
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))))?;

    Ok(Json(LoginResponse {
        token,
        role: user.role,
    }))
}

// Global Settings Handlers
pub async fn get_settings_handler(
    State(db): State<Database>,
    _claims: Claims,
) -> Result<Json<Option<DbBackend>>, (StatusCode, Json<serde_json::Value>)> {
    let settings = db::get_global_settings(&db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e }))))?;
    Ok(Json(settings))
}

pub async fn save_settings_handler(
    State(db): State<Database>,
    claims: Claims,
    Json(payload): Json<DbBackend>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Admin role required" }))));
    }

    db::save_global_settings(&db, &payload)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e }))))?;

    Ok(Json(serde_json::json!({ "status": "success" })))
}

// Module Handlers
pub async fn list_modules_handler(
    State(db): State<Database>,
    _claims: Claims,
) -> Result<Json<Vec<ModuleRecord>>, (StatusCode, Json<serde_json::Value>)> {
    let modules = db::get_modules(&db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e }))))?;
    Ok(Json(modules))
}

#[derive(Debug, Deserialize)]
pub struct ModuleConfigPayload {
    pub config: String,
    pub interval_secs: u64,
}

pub async fn save_module_config_handler(
    State(db): State<Database>,
    claims: Claims,
    Path(id): Path<String>,
    Json(payload): Json<ModuleConfigPayload>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Admin role required" }))));
    }

    let mut module = db::get_module(&db, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e }))))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Module not found" }))))?;

    module.config = payload.config;
    module.interval_secs = payload.interval_secs;

    db::save_module(&db, &module)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e }))))?;

    Ok(Json(serde_json::json!({ "status": "success" })))
}

#[derive(Debug, Deserialize)]
pub struct TogglePayload {
    pub enabled: bool,
}

pub async fn toggle_module_handler(
    State(db): State<Database>,
    claims: Claims,
    Path(id): Path<String>,
    Json(payload): Json<TogglePayload>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Admin role required" }))));
    }

    let mut module = db::get_module(&db, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e }))))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Module not found" }))))?;

    module.enabled = payload.enabled;
    module.status = if payload.enabled { "running".to_string() } else { "paused".to_string() };

    db::save_module(&db, &module)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e }))))?;

    Ok(Json(serde_json::json!({ "status": "success", "enabled": payload.enabled })))
}

pub async fn get_module_logs_handler(
    State(db): State<Database>,
    _claims: Claims,
    Path(id): Path<String>,
) -> Result<Json<Vec<db::LogRecord>>, (StatusCode, Json<serde_json::Value>)> {
    let logs = db::get_logs(&db, Some(&id), 50)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e }))))?;
    Ok(Json(logs))
}

// Store Handlers
#[derive(Debug, Serialize)]
pub struct StoreModule {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub repo_url: String,
    pub is_installed: bool,
}

pub async fn list_store_modules_handler(
    State(db): State<Database>,
    _claims: Claims,
) -> Result<Json<Vec<StoreModule>>, (StatusCode, Json<serde_json::Value>)> {
    let installed = db::get_modules(&db)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e }))))?;

    let installed_ids: HashMap<String, String> = installed.into_iter().map(|m| (m.id.clone(), m.version)).collect();

    // Curated store list
    let store_list = vec![
        StoreModule {
            id: "metrics-system".to_string(),
            name: "System Resources".to_string(),
            description: "Monitor CPU, RAM, and disk utilization".to_string(),
            author: "Metrics Core Team".to_string(),
            repo_url: "curated://metrics-system".to_string(),
            is_installed: installed_ids.contains_key("metrics-system"),
        },
        StoreModule {
            id: "metrics-zfs".to_string(),
            name: "ZFS Storage".to_string(),
            description: "Check status and usage of local ZFS pools".to_string(),
            author: "Metrics Core Team".to_string(),
            repo_url: "curated://metrics-zfs".to_string(),
            is_installed: installed_ids.contains_key("metrics-zfs"),
        },
        StoreModule {
            id: "metrics-immich".to_string(),
            name: "Immich Statistics".to_string(),
            description: "Collects total assets and user counts from Immich API".to_string(),
            author: "Metrics Core Team".to_string(),
            repo_url: "curated://metrics-immich".to_string(),
            is_installed: installed_ids.contains_key("metrics-immich"),
        },
    ];

    Ok(Json(store_list))
}

#[derive(Debug, Deserialize)]
pub struct InstallPayload {
    pub id: String,
    pub git_url: Option<String>,
}

pub async fn install_module_handler(
    State(db): State<Database>,
    claims: Claims,
    Json(payload): Json<InstallPayload>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Admin role required" }))));
    }

    let is_curated = payload.git_url.is_none() || payload.git_url.as_ref().unwrap().starts_with("curated://");
    let name = payload.id.clone();
    
    // Paths
    let current_dir = std::env::current_dir().unwrap_or_default();
    
    // Determine source path and compile command
    let (_src_path, compile_cmd, lib_filename) = if is_curated {
        let src = current_dir.join("modules").join(&name);
        let lib_name = if cfg!(target_os = "windows") {
            format!("{}.dll", name.replace('-', "_"))
        } else if cfg!(target_os = "macos") {
            format!("lib{}.dylib", name.replace('-', "_"))
        } else {
            format!("lib{}.so", name.replace('-', "_"))
        };
        (src, format!("cargo build --release -p {}", name), lib_name)
    } else {
        // Custom Git installation
        let git_url = payload.git_url.unwrap();
        let clone_dest = current_dir.join("modules").join("custom").join(&name);
        
        // Clone repo
        if clone_dest.exists() {
            let _ = std::fs::remove_dir_all(&clone_dest);
        }
        
        let output = tokio::process::Command::new("git")
            .args(["clone", &git_url, clone_dest.to_str().unwrap()])
            .output()
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": format!("Git clone failed: {}", e) }))))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": format!("Git clone failed: {}", err) }))));
        }

        let lib_name = if cfg!(target_os = "windows") {
            format!("{}.dll", name.replace('-', "_"))
        } else if cfg!(target_os = "macos") {
            format!("lib{}.dylib", name.replace('-', "_"))
        } else {
            format!("lib{}.so", name.replace('-', "_"))
        };

        (
            clone_dest.clone(),
            format!("cargo build --release --manifest-path {}/Cargo.toml", clone_dest.to_str().unwrap()),
            lib_name,
        )
    };

    // Run compile command
    println!("Installing module: Running command: {}", compile_cmd);
    let mut parts = compile_cmd.split_whitespace();
    let program = parts.next().unwrap();
    let args: Vec<&str> = parts.collect();

    let output = tokio::process::Command::new(program)
        .args(args)
        .output()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": format!("Compilation failed to start: {}", e) }))))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": format!("Compilation failed: {}", err) }))));
    }

    // Locate compiled library
    let target_lib_path = current_dir.join("target").join("release").join(&lib_filename);
    
    // Copy compiled library to persistent modules directory
    let dest_dir = current_dir.join("modules_bin");
    std::fs::create_dir_all(&dest_dir).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))))?;
    let final_lib_path = dest_dir.join(&lib_filename);
    
    std::fs::copy(&target_lib_path, &final_lib_path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": format!("Failed to copy library from {:?} to {:?}: {}", target_lib_path, final_lib_path, e) }))))?;

    // Load library once to read metadata
    let final_lib_path_str = final_lib_path.to_str().unwrap().to_string();
    let metadata = unsafe {
        match LoadedModule::load(&final_lib_path_str) {
            Ok(loaded) => loaded.module.metadata(),
            Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": format!("Failed to load installed library: {}", e) })))),
        }
    };

    // Save module in DB
    let new_module = ModuleRecord {
        id: name.clone(),
        name: metadata.name,
        version: metadata.version,
        author: metadata.author,
        description: metadata.description,
        config: metadata.default_config.clone(),
        default_config: metadata.default_config,
        interval_secs: 60,
        cron: None,
        enabled: false,
        status: "installed".to_string(),
        last_run: None,
        last_error: None,
        lib_path: final_lib_path_str,
    };

    db::save_module(&db, &new_module)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e }))))?;

    let _ = db::add_log(&db, &name, "info", "Module successfully compiled and installed.").await;

    Ok(Json(serde_json::json!({ "status": "success", "module": new_module })))
}

#[derive(Debug, Deserialize)]
pub struct UninstallPayload {
    pub id: String,
}

pub async fn uninstall_module_handler(
    State(db): State<Database>,
    claims: Claims,
    Json(payload): Json<UninstallPayload>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if claims.role != "admin" {
        return Err((StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "Admin role required" }))));
    }

    let id = payload.id;
    let module = db::get_module(&db, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e }))))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "Module not found" }))))?;

    // Delete dynamic library if it exists
    if std::path::Path::new(&module.lib_path).exists() {
        let _ = std::fs::remove_file(&module.lib_path);
    }

    // Delete from DB
    db::delete_module(&db, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e }))))?;

    Ok(Json(serde_json::json!({ "status": "success" })))
}
