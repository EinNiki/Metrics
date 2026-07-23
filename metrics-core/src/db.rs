use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::engine::local::SurrealKv;
use surrealdb::Surreal;
use metrics_api::DbBackend;
use surrealdb::types::SurrealValue;

pub type Database = Surreal<surrealdb::engine::local::Db>;

#[derive(Debug, Serialize, Deserialize, Clone, SurrealValue)]
pub struct User {
    pub username: String,
    pub password_hash: String,
    pub role: String, // "admin" or "viewer"
}

#[derive(Debug, Serialize, Deserialize, Clone, SurrealValue)]
pub struct ModuleRecord {
    pub id: String, // e.g., "metrics-system"
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub config: String, // Active JSON/TOML configuration
    pub default_config: String,
    pub interval_secs: u64,
    pub cron: Option<String>,
    pub enabled: bool,
    pub status: String, // "running", "paused", "error", "installed"
    pub last_run: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub lib_path: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, SurrealValue)]
pub struct LogRecord {
    pub module_id: String,
    pub timestamp: DateTime<Utc>,
    pub level: String, // "info", "error", "warn"
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, SurrealValue)]
pub struct DbSettingsRecord {
    pub config_json: String,
}

async fn select_all<T>(db: &Database, table: &str) -> Result<Vec<T>, String>
where
    T: serde::de::DeserializeOwned + surrealdb::types::SurrealValue,
{
    let res: Result<Vec<T>, surrealdb::Error> = db.select(table).await;
    match res {
        Ok(items) => Ok(items),
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("does not exist") || err_str.contains("not found") {
                Ok(Vec::new())
            } else {
                Err(err_str)
            }
        }
    }
}

pub async fn init_db(db_path: &str) -> Result<Database, surrealdb::Error> {
    let db = Surreal::new::<SurrealKv>(db_path).await?;
    db.use_ns("metrics").use_db("metrics").await?;
    Ok(db)
}

pub async fn create_admin_user_if_missing(db: &Database, default_password: &str) -> Result<(), String> {
    let users: Vec<User> = select_all(db, "user").await?;
    if users.is_empty() {
        use argon2::{
            password_hash::{SaltString, PasswordHasher},
            Argon2,
        };

        let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
        let hash = Argon2::default()
            .hash_password(default_password.as_bytes(), &salt)
            .map_err(|e| format!("Password hashing failed: {}", e))?
            .to_string();

        let admin = User {
            username: "admin".to_string(),
            password_hash: hash,
            role: "admin".to_string(),
        };

        let _: Option<User> = db.create(("user", "admin"))
            .content(admin)
            .await
            .map_err(|e| e.to_string())?;
        
        println!("Created default admin user with username 'admin'");
    }
    Ok(())
}

pub async fn get_user(db: &Database, username: &str) -> Result<Option<User>, String> {
    let user: Option<User> = db.select(("user", username))
        .await
        .map_err(|e| e.to_string())?;
    Ok(user)
}

pub async fn get_global_settings(db: &Database) -> Result<Option<DbBackend>, String> {
    let settings: Result<Option<DbSettingsRecord>, surrealdb::Error> = db.select(("settings", "global")).await;
    
    match settings {
        Ok(Some(record)) => {
            let backend: DbBackend = serde_json::from_str(&record.config_json)
                .map_err(|e| format!("Failed to deserialize settings: {}", e))?;
            Ok(Some(backend))
        }
        Ok(None) => Ok(None),
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("does not exist") || err_str.contains("not found") {
                Ok(None)
            } else {
                Err(err_str)
            }
        }
    }
}

pub async fn save_global_settings(db: &Database, settings: &DbBackend) -> Result<(), String> {
    let json = serde_json::to_string(settings)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;
    
    let record = DbSettingsRecord { config_json: json };
    let _: Option<DbSettingsRecord> = db.upsert(("settings", "global"))
        .content(record)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn get_modules(db: &Database) -> Result<Vec<ModuleRecord>, String> {
    select_all(db, "module").await
}

pub async fn get_module(db: &Database, id: &str) -> Result<Option<ModuleRecord>, String> {
    let module: Result<Option<ModuleRecord>, surrealdb::Error> = db.select(("module", id)).await;
    match module {
        Ok(opt) => Ok(opt),
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("does not exist") || err_str.contains("not found") {
                Ok(None)
            } else {
                Err(err_str)
            }
        }
    }
}

pub async fn save_module(db: &Database, module: &ModuleRecord) -> Result<(), String> {
    let _: Option<ModuleRecord> = db.upsert(("module", module.id.as_str()))
        .content(module.clone())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn delete_module(db: &Database, id: &str) -> Result<(), String> {
    let _: Option<ModuleRecord> = db.delete(("module", id))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn add_log(db: &Database, module_id: &str, level: &str, message: &str) -> Result<(), String> {
    let log = LogRecord {
        module_id: module_id.to_string(),
        timestamp: Utc::now(),
        level: level.to_string(),
        message: message.to_string(),
    };
    
    let _: Option<LogRecord> = db.create("log")
        .content(log)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn get_logs(db: &Database, module_id: Option<&str>, limit: usize) -> Result<Vec<LogRecord>, String> {
    let query_str = if module_id.is_some() {
        format!("SELECT * FROM log WHERE module_id = $mid ORDER BY timestamp DESC LIMIT {}", limit)
    } else {
        format!("SELECT * FROM log ORDER BY timestamp DESC LIMIT {}", limit)
    };

    let mut response = db.query(query_str);
    if let Some(mid) = module_id {
        response = response.bind(("mid", mid));
    }

    let mut res = match response.await {
        Ok(r) => r,
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("does not exist") || err_str.contains("not found") {
                return Ok(Vec::new());
            } else {
                return Err(err_str);
            }
        }
    };
    let logs: Vec<LogRecord> = res.take(0).map_err(|e| e.to_string())?;
    Ok(logs)
}
