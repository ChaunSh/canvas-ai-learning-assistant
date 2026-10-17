use std::sync::Arc;

use tauri::AppHandle;

use crate::ai::AiRuntime;
use crate::error::AppResult;
use crate::integrations::canvas::CanvasRuntime;
use crate::paths::AppPaths;
use crate::retrieval::vector::{SqliteBlobVectorIndex, VectorIndex};
use crate::security::{secrets, SecretStore};
use crate::storage::Db;

/// Long-lived application state, managed by Tauri.
///
/// Every field is cheap to clone (reference counted), so spawned tasks can take
/// exactly what they need without holding a reference into managed state.
pub struct AppState {
    pub paths: Arc<AppPaths>,
    pub db: Db,
    pub http: reqwest::Client,
    pub secrets: Arc<dyn SecretStore>,
    pub index: Arc<dyn VectorIndex>,
    pub ai: Arc<AiRuntime>,
    pub canvas: Arc<CanvasRuntime>,
}

impl AppState {
    pub fn new(app: &AppHandle) -> AppResult<Self> {
        let paths = AppPaths::resolve(app)?;
        let db = Db::open(&paths.db_path)?;
        let http = reqwest::Client::builder()
            .user_agent(format!(
                "CanvasAIAssistant/{} (Tauri; local-first study assistant)",
                env!("CARGO_PKG_VERSION")
            ))
            .connect_timeout(std::time::Duration::from_secs(20))
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .map_err(|e| crate::error::AppError::Message(format!("无法初始化 HTTP 客户端：{e}")))?;
        let secret_store = secrets::create(&paths.data_dir);
        Ok(Self {
            paths: Arc::new(paths),
            db,
            http,
            secrets: secret_store,
            index: Arc::new(SqliteBlobVectorIndex::new()),
            ai: Arc::new(AiRuntime::new()),
            canvas: Arc::new(CanvasRuntime::new()),
        })
    }
}
