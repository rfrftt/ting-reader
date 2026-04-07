//! JWT token generation and validation

use crate::core::error::{Result, TingError};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

/// JWT Claims structure
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub user_id: String,
    pub exp: usize,
}

/// Generate a JWT token for a user
pub fn generate_token(user_id: &str, secret: &str) -> Result<String> {
    let expiration = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::days(7))
        .ok_or_else(|| TingError::AuthenticationError("无法计算令牌过期时间".to_string()))?
        .timestamp() as usize;

    let claims = Claims {
        user_id: user_id.to_string(),
        exp: expiration,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| TingError::AuthenticationError(format!("生成令牌失败: {}", e)))
}

/// Validate a JWT token and extract claims
pub fn validate_token(token: &str, secret: &str) -> Result<Claims> {
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|e| {
        // Parse the error to provide more specific Chinese messages
        let error_msg = e.to_string();
        if error_msg.contains("ExpiredSignature") {
            TingError::AuthenticationError("令牌已过期".to_string())
        } else if error_msg.contains("InvalidSignature") {
            TingError::AuthenticationError("令牌签名无效".to_string())
        } else if error_msg.contains("InvalidToken") {
            TingError::AuthenticationError("令牌格式无效".to_string())
        } else {
            TingError::AuthenticationError(format!("令牌验证失败: {}", e))
        }
    })?;

    Ok(token_data.claims)
}
