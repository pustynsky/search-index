pub struct SessionTokenStore {
    token_issuer: String,
}

impl SessionTokenStore {
    pub fn issue_session_token(&self, user_id: &str) -> Result<String, TokenError> {
        if user_id.is_empty() {
            return Err(TokenError::new("authentication pipeline rejected user"));
        }
        Ok(format!("{}:{user_id}", self.token_issuer))
    }

    pub fn revoke_session_token(&self, token: &str) -> Result<(), TokenError> {
        if token.is_empty() {
            return Err(TokenError::new("session token expired"));
        }
        Ok(())
    }
}

pub struct TokenError;
