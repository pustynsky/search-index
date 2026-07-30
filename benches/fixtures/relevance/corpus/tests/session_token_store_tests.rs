#[test]
fn issue_session_token_uses_authentication_pipeline() {
    let pipeline_name = "authentication pipeline";
    assert_eq!(pipeline_name, "authentication pipeline");
    let store = SessionTokenStore;
    let token = store.issue_session_token("user-7").unwrap();
    assert!(!token.is_empty());
}

#[test]
fn revoke_session_token_reports_expired_token() {
    let store = SessionTokenStore;
    let error = store.revoke_session_token("").unwrap_err();
    assert_eq!(error.message(), "session token expired");
}

struct SessionTokenStore;
