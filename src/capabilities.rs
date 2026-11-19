use std::sync::Arc;

use serde_json::{Value, json};

use crate::db::User;
use crate::error::AppError;
use crate::state::AppState;

#[derive(Debug, Clone, Copy)]
pub struct Capabilities {
    pub terminal: bool,
    pub zip: bool,
    pub unzip: bool,
}

impl Capabilities {
    pub fn json(&self) -> Value {
        json!({
            "terminal": self.terminal,
            "zip": self.zip,
            "unzip": self.unzip,
        })
    }
}

/// Effective capabilities: global settings + admin policy + user preference.
pub async fn load(state: &Arc<AppState>, user: &User) -> Result<Capabilities, AppError> {
    let preferences = state
        .db
        .get_user_preferences(user.id)
        .await
        .map_err(AppError::internal)?;
    let terminal = state.settings().settings.terminal_enabled
        && user.feature_terminal
        && preferences.terminal_enabled;
    Ok(Capabilities {
        terminal,
        zip: user.feature_zip,
        unzip: user.feature_unzip,
    })
}

pub async fn load_by_id(state: &Arc<AppState>, user_id: i64) -> Result<Capabilities, AppError> {
    let user = state
        .db
        .find_user_by_id(user_id)
        .await
        .map_err(AppError::internal)?
        .ok_or(AppError::Unauthorized)?;
    load(state, &user).await
}

pub async fn require_terminal(
    state: &Arc<AppState>,
    user: &User,
) -> Result<Capabilities, AppError> {
    let capabilities = load(state, user).await?;
    if !capabilities.terminal {
        return Err(AppError::Forbidden);
    }
    Ok(capabilities)
}

pub async fn require_zip(state: &Arc<AppState>, user: &User) -> Result<Capabilities, AppError> {
    let capabilities = load(state, user).await?;
    if !capabilities.zip {
        return Err(AppError::Forbidden);
    }
    Ok(capabilities)
}

pub async fn require_unzip(state: &Arc<AppState>, user: &User) -> Result<Capabilities, AppError> {
    let capabilities = load(state, user).await?;
    if !capabilities.unzip {
        return Err(AppError::Forbidden);
    }
    Ok(capabilities)
}
