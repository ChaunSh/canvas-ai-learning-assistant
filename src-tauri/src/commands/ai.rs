//! AI provider, chat and generation commands.

use tauri::{AppHandle, State, WebviewWindow};

use crate::commands::guard::require_main_window;
use crate::domain::{
    AiProvider, AskAccepted, AskInput, ChatMessage, ChatSession, ProviderInput, ProviderTestResult,
    UsageStats,
};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::storage::ai_repo;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderView {
    pub providers: Vec<AiProvider>,
    pub secret_backing: String,
    pub secret_backing_label: String,
    pub secret_backing_secure: bool,
}

#[tauri::command]
pub async fn list_providers(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> AppResult<ProviderView> {
    require_main_window(&window)?;
    let mut providers = state.db.with(|c| ai_repo::list_providers(c))?;
    // `hasApiKey` is answered by the secret store, never by the database.
    for provider in providers.iter_mut() {
        provider.has_api_key = state
            .secrets
            .contains(&crate::security::secrets::provider_key(&provider.id))
            .unwrap_or(false);
    }
    let backing = state.secrets.backing();
    Ok(ProviderView {
        providers,
        secret_backing: backing.as_str().to_string(),
        secret_backing_label: backing.label().to_string(),
        secret_backing_secure: backing.is_secure(),
    })
}

#[tauri::command]
pub async fn save_provider(
    window: WebviewWindow,
    state: State<'_, AppState>,
    input: ProviderInput,
) -> AppResult<AiProvider> {
    require_main_window(&window)?;

    // Validate the key is present before creating a provider that cannot work.
    let api_key = input.api_key.clone().filter(|k| !k.trim().is_empty());
    if input.id.is_none() && api_key.is_none() {
        return Err(AppError::invalid("请填写 API Key"));
    }

    let provider = state.db.with(|c| {
        ai_repo::save_provider(
            c,
            &ai_repo::ProviderUpsert {
                id: input.id.as_deref(),
                name: &input.name,
                base_url: &input.base_url,
                chat_model: &input.chat_model,
                vision_model: input.vision_model.as_deref(),
                embedding_model: input.embedding_model.as_deref(),
                max_output_tokens: input.max_output_tokens,
                temperature: input.temperature,
                timeout_seconds: input.timeout_seconds.unwrap_or(120),
                is_default: input.is_default.unwrap_or(false),
            },
        )
    })?;

    if let Some(key) = api_key {
        state
            .secrets
            .set(&crate::security::secrets::provider_key(&provider.id), key.trim())?;
    }

    let mut provider = provider;
    provider.has_api_key = state
        .secrets
        .contains(&crate::security::secrets::provider_key(&provider.id))
        .unwrap_or(false);
    Ok(provider)
}

#[tauri::command]
pub async fn delete_provider(
    window: WebviewWindow,
    state: State<'_, AppState>,
    provider_id: String,
) -> AppResult<()> {
    require_main_window(&window)?;
    state.db.with(|c| ai_repo::delete_provider(c, &provider_id))?;
    // Best effort: the row is already gone, so a stale secret is harmless.
    let _ = state
        .secrets
        .delete(&crate::security::secrets::provider_key(&provider_id));
    Ok(())
}

#[tauri::command]
pub async fn set_default_provider(
    window: WebviewWindow,
    state: State<'_, AppState>,
    provider_id: String,
) -> AppResult<()> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| ai_repo::set_default_provider(c, &provider_id))
}

#[tauri::command]
pub async fn test_provider(
    window: WebviewWindow,
    state: State<'_, AppState>,
    provider_id: String,
) -> AppResult<ProviderTestResult> {
    require_main_window(&window)?;

    let (provider, config) = crate::ai::resolve_client(&state.db, state.secrets.as_ref(), Some(&provider_id))
        .map_err(|error| {
            // Surface a precise message when only the key is missing.
            if error.code() == "invalid_input" {
                error
            } else {
                AppError::Ai(error.to_string())
            }
        })?;

    let result = crate::ai::test_provider(
        &state.http,
        &config,
        provider.vision_model.as_deref(),
        provider.embedding_model.as_deref(),
    )
    .await?;

    state.db.with(|c| {
        ai_repo::mark_tested(c, &provider.id, result.ok, result.error.as_deref())?;
        ai_repo::record_usage(
            c,
            Some(&provider.id),
            Some(&config.model),
            "test",
            None,
            None,
            Some(result.latency_ms),
            result.ok,
            result.error.as_deref(),
        )
    })?;

    Ok(result)
}

#[tauri::command]
pub async fn list_models(
    window: WebviewWindow,
    state: State<'_, AppState>,
    provider_id: String,
) -> AppResult<Vec<crate::domain::ModelInfo>> {
    require_main_window(&window)?;
    let (_provider, config) =
        crate::ai::resolve_client(&state.db, state.secrets.as_ref(), Some(&provider_id))?;
    config.client(&state.http).list_models().await
}

/// Start a generation. Returns immediately; tokens arrive on `ai://stream`.
#[tauri::command]
pub async fn ask_question(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, AppState>,
    input: AskInput,
) -> AppResult<AskAccepted> {
    require_main_window(&window)?;

    let language = state
        .db
        .with(|c| crate::storage::settings::get(c, crate::storage::settings::KEY_LOCALE))
        .ok()
        .flatten()
        .unwrap_or_else(|| "zh".into());

    let prepared = crate::ai::prepare_answer(
        &state.db,
        state.secrets.as_ref(),
        &state.http,
        state.index.as_ref(),
        crate::ai::PrepareInput {
            question: &input.question,
            session_id: input.session_id.as_deref(),
            scope: input.scope,
            provider_id: input.provider_id.as_deref(),
            model: input.model.as_deref(),
            top_k: input.top_k.unwrap_or(6),
            no_retrieval: input.no_retrieval,
            language: &language,
        },
    )
    .await?;

    let accepted = crate::ai::accepted_from(&prepared);

    // The stream must outlive this command.
    let token = state.ai.register(&prepared.request_id);

    let db = state.db.clone();
    let http = state.http.clone();
    let runtime = state.ai.clone();
    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) =
            crate::ai::execute_generation(&app_handle, db, http, runtime, prepared, token).await
        {
            tracing::warn!(error = %error, "generation failed");
        }
    });

    Ok(accepted)
}

#[tauri::command]
pub async fn cancel_generation(
    window: WebviewWindow,
    state: State<'_, AppState>,
    request_id: String,
) -> AppResult<bool> {
    require_main_window(&window)?;
    Ok(state.ai.cancel(&request_id))
}

#[tauri::command]
pub async fn list_chat_sessions(
    window: WebviewWindow,
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> AppResult<Vec<ChatSession>> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| ai_repo::list_sessions(c, limit.unwrap_or(100).clamp(1, 500)))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatDetail {
    pub session: ChatSession,
    pub messages: Vec<ChatMessage>,
}

#[tauri::command]
pub async fn get_chat_session(
    window: WebviewWindow,
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<ChatDetail> {
    require_main_window(&window)?;
    state.db.with(|c| {
        Ok(ChatDetail {
            session: ai_repo::get_session(c, &session_id)?,
            messages: ai_repo::list_messages(c, &session_id)?,
        })
    })
}

#[tauri::command]
pub async fn delete_chat_session(
    window: WebviewWindow,
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<()> {
    require_main_window(&window)?;
    state.db.with(|c| ai_repo::delete_session(c, &session_id))
}

#[tauri::command]
pub async fn rename_chat_session(
    window: WebviewWindow,
    state: State<'_, AppState>,
    session_id: String,
    title: String,
) -> AppResult<()> {
    require_main_window(&window)?;
    state.db.with(|c| ai_repo::rename_session(c, &session_id, &title))
}

#[tauri::command]
pub async fn update_chat_scope(
    window: WebviewWindow,
    state: State<'_, AppState>,
    session_id: String,
    scope: crate::domain::ChatScope,
) -> AppResult<()> {
    require_main_window(&window)?;
    state
        .db
        .with(|c| ai_repo::update_session_scope(c, &session_id, &scope))
}

#[tauri::command]
pub async fn ai_usage(
    window: WebviewWindow,
    state: State<'_, AppState>,
) -> AppResult<UsageStats> {
    require_main_window(&window)?;
    state.db.with(|c| ai_repo::usage_stats(c))
}
