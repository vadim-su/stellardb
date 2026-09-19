mod common;

use serde_json::json;

// ─── Authentication ───

#[tokio::test]
async fn unauthenticated_request_rejected() {
    let (addr, _root_pw, _tmp) = common::spawn_server_with_auth().await;
    let client = reqwest::Client::new(); // No auth headers
    let res = client
        .post(format!("http://{}/sql", addr))
        .header("X-Database", "test")
        .json(&json!({"query": "SELECT 1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn login_returns_jwt_token() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let token = common::login(addr, "root", &root_pw).await;
    assert!(!token.is_empty());
    // Token should have 3 parts (header.payload.signature)
    assert_eq!(token.split('.').count(), 3);
}

#[tokio::test]
async fn login_with_wrong_password_returns_401() {
    let (addr, _, _tmp) = common::spawn_server_with_auth().await;
    let client = reqwest::Client::new();
    let res = client
        .post(format!("http://{}/auth/login", addr))
        .json(&json!({"username": "root", "password": "wrong"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn authenticated_request_succeeds() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let token = common::login(addr, "root", &root_pw).await;
    let body = common::sql_with_auth(addr, &token, "SELECT 1").await;
    assert!(body["error"].is_null());
    assert_eq!(body["completed"], 1);
}

#[tokio::test]
async fn invalid_jwt_token_returns_401() {
    let (addr, _root_pw, _tmp) = common::spawn_server_with_auth().await;
    let client = common::auth_client("invalid.jwt.token");
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT 1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);
}

// ─── Anonymous Access ───

async fn anonymous_sql(addr: std::net::SocketAddr, query: &str) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("http://{}/sql", addr))
        .header("X-Database", "test")
        .json(&json!({"query": query}))
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn anonymous_user_policies_govern_credential_less_requests() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_anonymous_user("guest").await;
    let root_token = common::login(addr, "root", &root_pw).await;

    // Anonymous user not created yet: still rejected.
    assert_eq!(anonymous_sql(addr, "SELECT 1").await.status(), 401);

    common::sql_with_auth(addr, &root_token, "CREATE USER 'guest' PASSWORD 'unused'").await;
    common::sql_with_auth(addr, &root_token, "ALTER USER 'guest' SET role = 'public'").await;
    common::sql_with_auth(
        addr,
        &root_token,
        "CREATE POLICY public_read WHEN subject.role = 'public' AND action = 'SELECT' AND resource.database = 'test' ALLOW",
    )
    .await;
    common::auth_client(&root_token)
        .post(format!("http://{}/databases", addr))
        .json(&json!({"name": "private"}))
        .send()
        .await
        .unwrap();
    common::sql_with_auth(addr, &root_token, "INSERT INTO items {name: 'a'}").await;

    let res = anonymous_sql(addr, "SELECT * FROM items").await;
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert!(body["error"].is_null(), "anonymous read failed: {body:?}");
    assert_eq!(body["completed"], 1);

    let res = anonymous_sql(addr, "INSERT INTO items {name: 'b'}").await;
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    let err_msg = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        err_msg.contains("Access denied"),
        "anonymous write allowed: {body:?}"
    );

    // An invalid token is never downgraded to anonymous.
    let res = common::auth_client("invalid.jwt.token")
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT 1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);

    let me: serde_json::Value = reqwest::Client::new()
        .get(format!("http://{}/v1/auth/me", addr))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me["username"], "guest");
    assert_eq!(me["authMethod"], "anonymous");

    // Database discovery is filtered to what the anonymous subject may read.
    let databases: serde_json::Value = reqwest::Client::new()
        .get(format!("http://{}/databases", addr))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(databases, json!(["test"]));
}

// ─── User Management ───

#[tokio::test]
async fn create_user_and_login() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let token = common::login(addr, "root", &root_pw).await;

    // Create user
    let body =
        common::sql_with_auth(addr, &token, "CREATE USER 'alice' PASSWORD 'secret123'").await;
    assert!(body["error"].is_null(), "create user failed: {:?}", body);

    // Login as new user
    let alice_token = common::login(addr, "alice", "secret123").await;
    assert!(!alice_token.is_empty());
}

#[tokio::test]
async fn drop_user_prevents_login() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let token = common::login(addr, "root", &root_pw).await;

    common::sql_with_auth(addr, &token, "CREATE USER 'bob' PASSWORD 'pass'").await;
    common::sql_with_auth(addr, &token, "DROP USER 'bob'").await;

    // Login should fail
    let client = reqwest::Client::new();
    let res = client
        .post(format!("http://{}/auth/login", addr))
        .json(&json!({"username": "bob", "password": "pass"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn alter_user_password() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let token = common::login(addr, "root", &root_pw).await;

    common::sql_with_auth(addr, &token, "CREATE USER 'carol' PASSWORD 'old'").await;
    common::sql_with_auth(addr, &token, "ALTER USER 'carol' PASSWORD 'new'").await;

    // Old password fails
    let client = reqwest::Client::new();
    let res = client
        .post(format!("http://{}/auth/login", addr))
        .json(&json!({"username": "carol", "password": "old"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);

    // New password works
    let _token = common::login(addr, "carol", "new").await;
}

#[tokio::test]
async fn cannot_drop_root_user() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let token = common::login(addr, "root", &root_pw).await;

    let body = common::sql_with_auth(addr, &token, "DROP USER 'root'").await;
    assert_eq!(body["error"]["code"], "SDB-AC003");
}

#[tokio::test]
async fn create_duplicate_user_fails() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let token = common::login(addr, "root", &root_pw).await;

    let body = common::sql_with_auth(addr, &token, "CREATE USER 'dupuser' PASSWORD 'pass1'").await;
    assert!(body["error"].is_null(), "first create should succeed");

    let body = common::sql_with_auth(addr, &token, "CREATE USER 'dupuser' PASSWORD 'pass2'").await;
    assert!(
        body["error"].is_object(),
        "duplicate user should fail: {:?}",
        body
    );
    assert_eq!(body["error"]["code"], "SDB-AC005");
}

#[tokio::test]
async fn auth_management_not_found_errors_are_typed() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let token = common::login(addr, "root", &root_pw).await;

    for query in [
        "ALTER USER 'missing' PASSWORD 'new'",
        "DROP USER 'missing'",
        "CREATE API KEY 'missing-user-key' FOR USER 'missing'",
        "DROP API KEY 'missing'",
        "DROP POLICY missing",
    ] {
        let body = common::sql_with_auth(addr, &token, query).await;
        assert_eq!(
            body["error"]["code"], "SDB-AC004",
            "unexpected error for {query}: {body:?}"
        );
    }
}

// ─── API Key Authentication ───

#[tokio::test]
async fn api_key_authentication() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let token = common::login(addr, "root", &root_pw).await;

    // Create API key
    let body = common::sql_with_auth(addr, &token, "CREATE API KEY 'my-key' FOR USER 'root'").await;
    let api_key = body["results"][0]["data"][0]["key"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(api_key.starts_with("stl_"));

    // Use API key for authentication
    let body = common::sql_with_auth(addr, &api_key, "SELECT 1").await;
    assert!(body["error"].is_null(), "API key auth should work");
}

#[tokio::test]
async fn drop_api_key_revokes_access() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let token = common::login(addr, "root", &root_pw).await;

    let body =
        common::sql_with_auth(addr, &token, "CREATE API KEY 'temp-key' FOR USER 'root'").await;
    let api_key = body["results"][0]["data"][0]["key"]
        .as_str()
        .unwrap()
        .to_string();

    // Key works before drop
    let body = common::sql_with_auth(addr, &api_key, "SELECT 1").await;
    assert!(body["error"].is_null());

    // Drop the key
    common::sql_with_auth(addr, &token, "DROP API KEY 'temp-key'").await;

    // Key should no longer work (middleware rejects it -> 401)
    let client = common::auth_client(&api_key);
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT 1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 401);
}

// ─── Authorization (ABAC Policies) ───

#[tokio::test]
async fn root_bypasses_all_policies() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let token = common::login(addr, "root", &root_pw).await;

    // No policies needed -- root always allowed
    let body = common::sql_with_auth(addr, &token, "DEFINE COLLECTION products").await;
    assert!(body["error"].is_null(), "DEFINE failed: {:?}", body);
    let body = common::sql_with_auth(addr, &token, "INSERT INTO products {name: 'test'}").await;
    assert!(body["error"].is_null(), "INSERT failed: {:?}", body);
    let body = common::sql_with_auth(addr, &token, "SELECT * FROM products").await;
    assert!(body["error"].is_null(), "SELECT failed: {:?}", body);
}

#[tokio::test]
async fn default_deny_for_non_root_user() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let root_token = common::login(addr, "root", &root_pw).await;

    // Create a non-root user
    common::sql_with_auth(addr, &root_token, "CREATE USER 'viewer' PASSWORD 'pass'").await;
    let viewer_token = common::login(addr, "viewer", "pass").await;

    // Non-root, no policies -> denied
    let body = common::sql_with_auth(addr, &viewer_token, "SELECT * FROM users").await;
    assert!(
        body["error"].is_object(),
        "should be denied by default: {:?}",
        body
    );
    let err_msg = body["error"]["message"].as_str().unwrap_or("");
    assert!(err_msg.contains("Access denied"));
}

#[tokio::test]
async fn policy_allows_select() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let root_token = common::login(addr, "root", &root_pw).await;

    // Create user with department attribute
    common::sql_with_auth(addr, &root_token, "CREATE USER 'analyst' PASSWORD 'pass'").await;
    common::sql_with_auth(
        addr,
        &root_token,
        "ALTER USER 'analyst' SET department = 'analytics'",
    )
    .await;

    // Create policy allowing SELECT for analytics department
    common::sql_with_auth(
        addr,
        &root_token,
        "CREATE POLICY analytics_read WHEN subject.department = 'analytics' AND action = 'SELECT' ALLOW",
    )
    .await;

    // Login as analyst (re-login to get JWT with updated attributes)
    let analyst_token = common::login(addr, "analyst", "pass").await;

    // SELECT allowed
    let body = common::sql_with_auth(addr, &analyst_token, "SELECT 1").await;
    assert!(
        body["error"].is_null(),
        "SELECT should be allowed: {:?}",
        body
    );

    // INSERT denied (no policy for INSERT)
    let body = common::sql_with_auth(addr, &analyst_token, "INSERT INTO test {name: 'x'}").await;
    assert!(body["error"].is_object(), "INSERT should be denied");
}

#[tokio::test]
async fn deny_policy_overrides_allow() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let root_token = common::login(addr, "root", &root_pw).await;

    // Create user
    common::sql_with_auth(addr, &root_token, "CREATE USER 'user1' PASSWORD 'pass'").await;
    common::sql_with_auth(addr, &root_token, "ALTER USER 'user1' SET role = 'intern'").await;

    // Allow all SELECTs
    common::sql_with_auth(
        addr,
        &root_token,
        "CREATE POLICY allow_select WHEN action = 'SELECT' ALLOW",
    )
    .await;

    // Deny for interns
    common::sql_with_auth(
        addr,
        &root_token,
        "CREATE POLICY deny_interns WHEN action = 'SELECT' AND subject.role = 'intern' DENY",
    )
    .await;

    let user_token = common::login(addr, "user1", "pass").await;

    // Denied because deny wins
    let body = common::sql_with_auth(addr, &user_token, "SELECT 1").await;
    assert!(
        body["error"].is_object(),
        "deny should win over allow: {:?}",
        body
    );
    let err_msg = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        err_msg.contains("deny_interns"),
        "should reference the deny policy"
    );
}

#[tokio::test]
async fn drop_policy_removes_authorization() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let root_token = common::login(addr, "root", &root_pw).await;

    // Create user + allow policy
    common::sql_with_auth(addr, &root_token, "CREATE USER 'temp' PASSWORD 'pass'").await;
    common::sql_with_auth(
        addr,
        &root_token,
        "CREATE POLICY temp_read WHEN action = 'SELECT' ALLOW",
    )
    .await;

    let temp_token = common::login(addr, "temp", "pass").await;

    // SELECT allowed
    let body = common::sql_with_auth(addr, &temp_token, "SELECT 1").await;
    assert!(body["error"].is_null());

    // Drop policy
    common::sql_with_auth(addr, &root_token, "DROP POLICY temp_read").await;

    // SELECT now denied
    let body = common::sql_with_auth(addr, &temp_token, "SELECT 1").await;
    assert!(
        body["error"].is_object(),
        "should be denied after policy drop"
    );
}

// ─── System Database Protection ───

#[tokio::test]
async fn cannot_drop_system_database() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let token = common::login(addr, "root", &root_pw).await;

    let client = common::auth_client(&token);
    let res = client
        .delete(format!("http://{}/databases/_system", addr))
        .send()
        .await
        .unwrap();
    // Should fail (400 or 500 -- depends on error mapping, but definitely not 204)
    assert_ne!(res.status(), 204, "should NOT be able to drop _system");
}

// ─── Collection Attributes in Policies ───

#[tokio::test]
async fn collection_attributes_in_policy_evaluation() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let root_token = common::login(addr, "root", &root_pw).await;

    // Create user
    common::sql_with_auth(addr, &root_token, "CREATE USER 'reader' PASSWORD 'pass'").await;

    // Allow SELECT for everyone
    common::sql_with_auth(
        addr,
        &root_token,
        "CREATE POLICY allow_read WHEN action = 'SELECT' ALLOW",
    )
    .await;

    // Deny SELECT on sensitive collections
    common::sql_with_auth(
        addr,
        &root_token,
        "CREATE POLICY deny_sensitive WHEN action = 'SELECT' AND resource.sensitivity = 'pii' DENY",
    )
    .await;

    // Mark "users" collection as sensitive
    common::sql_with_auth(
        addr,
        &root_token,
        "ALTER COLLECTION users SET sensitivity = 'pii'",
    )
    .await;

    let reader_token = common::login(addr, "reader", "pass").await;

    // SELECT on non-sensitive collection: allowed
    let body = common::sql_with_auth(addr, &reader_token, "SELECT 1").await;
    assert!(body["error"].is_null(), "scalar SELECT should be allowed");

    // SELECT on sensitive collection: denied
    let body = common::sql_with_auth(addr, &reader_token, "SELECT * FROM users").await;
    assert!(
        body["error"].is_object(),
        "SELECT on pii collection should be denied: {:?}",
        body
    );
    let err_msg = body["error"]["message"].as_str().unwrap_or("");
    assert!(err_msg.contains("deny_sensitive"));
}

// ─── Public Endpoints ───

#[tokio::test]
async fn login_endpoint_is_public() {
    let (addr, _, _tmp) = common::spawn_server_with_auth().await;
    // /auth/login should not require auth
    let client = reqwest::Client::new();
    let res = client
        .post(format!("http://{}/auth/login", addr))
        .json(&json!({"username": "nobody", "password": "wrong"}))
        .send()
        .await
        .unwrap();
    // Should get 401 (invalid credentials), NOT 401 (missing auth header)
    assert_eq!(res.status(), 401);
    let body: serde_json::Value = res.json().await.unwrap();
    let error = body["error"]["message"].as_str().unwrap_or("");
    assert!(
        error.contains("invalid credentials"),
        "login endpoint should be accessible without auth: got {:?}",
        body
    );
    assert_eq!(
        body["error"]["code"].as_str().unwrap(),
        "SDB-AC001",
        "expected auth error code"
    );
}

// ─── Multi-action Policy ───

#[tokio::test]
async fn policy_with_multiple_actions() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let root_token = common::login(addr, "root", &root_pw).await;

    // Create user
    common::sql_with_auth(addr, &root_token, "CREATE USER 'writer' PASSWORD 'pass'").await;

    // Allow SELECT and INSERT
    common::sql_with_auth(
        addr,
        &root_token,
        "CREATE POLICY allow_select WHEN action = 'SELECT' ALLOW",
    )
    .await;
    common::sql_with_auth(
        addr,
        &root_token,
        "CREATE POLICY allow_insert WHEN action = 'INSERT' ALLOW",
    )
    .await;

    let writer_token = common::login(addr, "writer", "pass").await;

    // Setup collection as root
    common::sql_with_auth(addr, &root_token, "DEFINE COLLECTION items").await;

    // SELECT allowed
    let body = common::sql_with_auth(addr, &writer_token, "SELECT * FROM items").await;
    assert!(body["error"].is_null(), "SELECT should be allowed");

    // INSERT allowed
    let body =
        common::sql_with_auth(addr, &writer_token, "INSERT INTO items {name: 'widget'}").await;
    assert!(body["error"].is_null(), "INSERT should be allowed");

    // DELETE denied (no policy for DELETE)
    let body = common::sql_with_auth(addr, &writer_token, "DELETE items:nonexistent").await;
    assert!(body["error"].is_object(), "DELETE should be denied");
}

// ─── User Attribute Changes Reflected in New Tokens ───

#[tokio::test]
async fn user_attribute_changes_in_new_tokens() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let root_token = common::login(addr, "root", &root_pw).await;

    // Create user and policy
    common::sql_with_auth(addr, &root_token, "CREATE USER 'dev' PASSWORD 'pass'").await;
    common::sql_with_auth(
        addr,
        &root_token,
        "CREATE POLICY team_read WHEN subject.team = 'core' AND action = 'SELECT' ALLOW",
    )
    .await;

    // Login before setting attributes -- should be denied
    let dev_token = common::login(addr, "dev", "pass").await;
    let body = common::sql_with_auth(addr, &dev_token, "SELECT 1").await;
    assert!(
        body["error"].is_object(),
        "should be denied without team attribute"
    );

    // Set team attribute
    common::sql_with_auth(addr, &root_token, "ALTER USER 'dev' SET team = 'core'").await;

    // Re-login to get updated token
    let dev_token2 = common::login(addr, "dev", "pass").await;
    let body = common::sql_with_auth(addr, &dev_token2, "SELECT 1").await;
    assert!(
        body["error"].is_null(),
        "should be allowed after team attribute set: {:?}",
        body
    );
}

// ─── Non-root Cannot Manage Auth ───

#[tokio::test]
async fn non_root_cannot_create_users() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let root_token = common::login(addr, "root", &root_pw).await;

    // Create a regular user with only SELECT permissions
    common::sql_with_auth(addr, &root_token, "CREATE USER 'regular' PASSWORD 'pass'").await;
    common::sql_with_auth(
        addr,
        &root_token,
        "CREATE POLICY allow_select WHEN action = 'SELECT' ALLOW",
    )
    .await;

    let regular_token = common::login(addr, "regular", "pass").await;

    // Regular user tries to create a user (requires MANAGE action) -- should be denied
    let body =
        common::sql_with_auth(addr, &regular_token, "CREATE USER 'hacker' PASSWORD 'hack'").await;
    assert!(
        body["error"].is_object(),
        "non-root should not be able to create users: {:?}",
        body
    );
    let err_msg = body["error"]["message"].as_str().unwrap_or("");
    assert!(err_msg.contains("Access denied"));
}

// ─── Batch Authorization ───

#[tokio::test]
async fn batch_stops_at_first_denied_statement() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let root_token = common::login(addr, "root", &root_pw).await;

    // Create user with only SELECT allowed
    common::sql_with_auth(addr, &root_token, "CREATE USER 'batchuser' PASSWORD 'pass'").await;
    common::sql_with_auth(
        addr,
        &root_token,
        "CREATE POLICY allow_select_only WHEN action = 'SELECT' ALLOW",
    )
    .await;

    let user_token = common::login(addr, "batchuser", "pass").await;

    // Batch: SELECT succeeds, INSERT fails at authorization
    let body =
        common::sql_with_auth(addr, &user_token, "SELECT 1; INSERT INTO test {name: 'a'}").await;
    assert_eq!(body["completed"], 1, "should stop after first statement");
    assert!(body["error"].is_object());
    assert_eq!(body["error"]["statement_index"], 1);
    let err_msg = body["error"]["message"].as_str().unwrap_or("");
    assert!(err_msg.contains("Access denied"));
}

// ─── DESCRIBE and LET skip authorization ───

#[tokio::test]
async fn describe_skips_authorization() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let root_token = common::login(addr, "root", &root_pw).await;

    // Create user with no policies
    common::sql_with_auth(addr, &root_token, "CREATE USER 'noauth' PASSWORD 'pass'").await;

    let noauth_token = common::login(addr, "noauth", "pass").await;

    // DESCRIBE should not require authorization
    let body = common::sql_with_auth(addr, &noauth_token, "DESCRIBE COLLECTIONS").await;
    assert!(
        body["error"].is_null(),
        "DESCRIBE should skip authorization: {:?}",
        body
    );
}

// ─── API Key Inherits User Permissions ───

#[tokio::test]
async fn api_key_inherits_user_permissions() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let root_token = common::login(addr, "root", &root_pw).await;

    // Create user with department attribute
    common::sql_with_auth(addr, &root_token, "CREATE USER 'svc' PASSWORD 'pass'").await;
    common::sql_with_auth(
        addr,
        &root_token,
        "ALTER USER 'svc' SET department = 'data'",
    )
    .await;

    // Create policy that allows SELECT for data department
    common::sql_with_auth(
        addr,
        &root_token,
        "CREATE POLICY data_read WHEN subject.department = 'data' AND action = 'SELECT' ALLOW",
    )
    .await;

    // Create API key for the user
    let body =
        common::sql_with_auth(addr, &root_token, "CREATE API KEY 'svc-key' FOR USER 'svc'").await;
    let api_key = body["results"][0]["data"][0]["key"]
        .as_str()
        .unwrap()
        .to_string();

    // API key should inherit user's attributes and permissions
    let body = common::sql_with_auth(addr, &api_key, "SELECT 1").await;
    assert!(
        body["error"].is_null(),
        "API key should inherit user permissions: {:?}",
        body
    );

    // INSERT should be denied (no policy for it)
    let body = common::sql_with_auth(addr, &api_key, "INSERT INTO test {name: 'x'}").await;
    assert!(
        body["error"].is_object(),
        "INSERT should be denied via API key: {:?}",
        body
    );
}

// ─── Drop User Revokes API Keys ───

#[tokio::test]
async fn drop_user_revokes_api_keys() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let root_token = common::login(addr, "root", &root_pw).await;

    // Create user and API key
    common::sql_with_auth(addr, &root_token, "CREATE USER 'ephemeral' PASSWORD 'pass'").await;
    let body = common::sql_with_auth(
        addr,
        &root_token,
        "CREATE API KEY 'eph-key' FOR USER 'ephemeral'",
    )
    .await;
    let api_key = body["results"][0]["data"][0]["key"]
        .as_str()
        .unwrap()
        .to_string();

    // Drop the user (should cascade-delete API keys)
    common::sql_with_auth(addr, &root_token, "DROP USER 'ephemeral'").await;

    // API key should no longer work
    let client = common::auth_client(&api_key);
    let res = client
        .post(format!("http://{}/sql", addr))
        .json(&json!({"query": "SELECT 1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        401,
        "API key should be revoked after user drop"
    );
}

#[tokio::test]
async fn http_transport_endpoints_enforce_default_deny_and_read_only_policy() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let root_token = common::login(addr, "root", &root_pw).await;

    common::sql_with_auth(addr, &root_token, "DEFINE COLLECTION items").await;
    common::sql_with_auth(addr, &root_token, "INSERT INTO items {name: 'seed'}").await;
    common::sql_with_auth(addr, &root_token, "CREATE USER 'reader' PASSWORD 'pass'").await;
    let reader_token = common::login(addr, "reader", "pass").await;
    let reader = common::auth_client(&reader_token);

    for path in [
        "/v1/collections",
        "/collections/items/documents",
        "/nodes/items:seed/out/related",
    ] {
        let response = reader
            .get(format!("http://{addr}{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 403, "default deny must protect {path}");
    }

    common::sql_with_auth(
        addr,
        &root_token,
        "CREATE POLICY transport_read WHEN action = 'SELECT' ALLOW",
    )
    .await;

    let response = reader
        .get(format!("http://{addr}/collections/items/documents"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);

    let response = reader
        .post(format!("http://{addr}/collections/items/documents"))
        .json(&json!({"name": "forbidden"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        403,
        "SELECT policy must not allow writes"
    );
}

#[tokio::test]
async fn http_database_management_and_metrics_require_manage_policy() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let root_token = common::login(addr, "root", &root_pw).await;
    common::sql_with_auth(addr, &root_token, "CREATE USER 'operator' PASSWORD 'pass'").await;
    let operator_token = common::login(addr, "operator", "pass").await;
    let operator = common::auth_client(&operator_token);

    let create = || {
        operator
            .post(format!("http://{addr}/databases"))
            .json(&json!({"name": "managed"}))
            .send()
    };
    assert_eq!(create().await.unwrap().status(), 403);
    assert_eq!(
        operator
            .get(format!("http://{addr}/metrics"))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );

    common::sql_with_auth(
        addr,
        &root_token,
        "CREATE POLICY transport_manage WHEN action = 'MANAGE' ALLOW",
    )
    .await;

    assert_eq!(create().await.unwrap().status(), 201);
    assert_eq!(
        operator
            .get(format!("http://{addr}/stats"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        operator
            .delete(format!("http://{addr}/databases/managed"))
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
}

#[tokio::test]
async fn http_sql_authorization_uses_resource_database() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let root_token = common::login(addr, "root", &root_pw).await;
    let root = common::auth_client(&root_token);
    assert_eq!(
        root.post(format!("http://{addr}/databases"))
            .json(&json!({"name": "other"}))
            .send()
            .await
            .unwrap()
            .status(),
        201
    );

    common::sql_with_auth(addr, &root_token, "CREATE USER 'db_reader' PASSWORD 'pass'").await;
    common::sql_with_auth(
        addr,
        &root_token,
        "CREATE POLICY test_database_read WHEN action = 'SELECT' AND resource.database = 'test' ALLOW",
    )
    .await;
    let token = common::login(addr, "db_reader", "pass").await;
    let client = reqwest::Client::new();

    for endpoint in ["/sql", "/v1/query"] {
        let allowed: serde_json::Value = client
            .post(format!("http://{addr}{endpoint}"))
            .bearer_auth(&token)
            .header("X-Database", "test")
            .json(&json!({"query": "SELECT 1"}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(
            allowed["error"].is_null(),
            "{endpoint} should allow the policy database: {allowed:?}"
        );

        let denied: serde_json::Value = client
            .post(format!("http://{addr}{endpoint}"))
            .bearer_auth(&token)
            .header("X-Database", "other")
            .json(&json!({"query": "SELECT 1"}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(
            denied["error"].is_object(),
            "{endpoint} should deny a different database: {denied:?}"
        );
    }
}

#[tokio::test]
async fn jwt_is_revoked_after_password_attributes_and_user_changes() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let root_token = common::login(addr, "root", &root_pw).await;
    common::sql_with_auth(addr, &root_token, "CREATE USER 'revoked' PASSWORD 'pass'").await;

    let password_token = common::login(addr, "revoked", "pass").await;
    common::sql_with_auth(
        addr,
        &root_token,
        "ALTER USER 'revoked' PASSWORD 'new-pass'",
    )
    .await;
    assert_eq!(
        common::auth_client(&password_token)
            .post(format!("http://{addr}/sql"))
            .json(&json!({"query": "SELECT 1"}))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );

    let attribute_token = common::login(addr, "revoked", "new-pass").await;
    common::sql_with_auth(
        addr,
        &root_token,
        "ALTER USER 'revoked' SET role = 'reader'",
    )
    .await;
    assert_eq!(
        common::auth_client(&attribute_token)
            .post(format!("http://{addr}/sql"))
            .json(&json!({"query": "SELECT 1"}))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );

    let deletion_token = common::login(addr, "revoked", "new-pass").await;
    common::sql_with_auth(addr, &root_token, "DROP USER 'revoked'").await;
    assert_eq!(
        common::auth_client(&deletion_token)
            .post(format!("http://{addr}/sql"))
            .json(&json!({"query": "SELECT 1"}))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );

    common::sql_with_auth(
        addr,
        &root_token,
        "CREATE USER 'revoked' PASSWORD 'replacement-pass'",
    )
    .await;
    assert_eq!(
        common::auth_client(&deletion_token)
            .post(format!("http://{addr}/sql"))
            .json(&json!({"query": "SELECT 1"}))
            .send()
            .await
            .unwrap()
            .status(),
        401,
        "a JWT from the deleted user must not revive after username reuse"
    );
    let replacement_token = common::login(addr, "revoked", "replacement-pass").await;
    assert_ne!(replacement_token, deletion_token);
}

#[tokio::test]
async fn login_without_connect_info_uses_global_limiter_only() {
    let admission = stellardb::server::admission::AdmissionConfig {
        login_rate_per_second: 0.0,
        login_burst: 1,
        login_client_rate_per_second: 0.0,
        login_client_burst: 0,
        ..Default::default()
    };
    let (addr, root_password, _tmp) =
        common::spawn_server_with_auth_without_connect_info_and_admission(admission).await;
    let client = reqwest::Client::new();
    let first = client
        .post(format!("http://{addr}/auth/login"))
        .json(&json!({"username": "root", "password": root_password}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        first.status(),
        200,
        "ordinary axum::serve must support login"
    );

    let limited = client
        .post(format!("http://{addr}/auth/login"))
        .json(&json!({"username": "root", "password": "wrong"}))
        .send()
        .await
        .unwrap();
    assert_eq!(limited.status(), 429, "global limiter must remain active");
}

#[tokio::test]
async fn login_has_small_body_rate_and_concurrency_limits() {
    let (addr, _, _tmp) = common::spawn_server_with_auth().await;
    let oversized = reqwest::Client::new()
        .post(format!("http://{addr}/auth/login"))
        .json(&json!({"username": "root", "password": "x".repeat(20 * 1024)}))
        .send()
        .await
        .unwrap();
    assert_eq!(oversized.status(), 413);

    let rate_config = stellardb::server::admission::AdmissionConfig {
        login_rate_per_second: 0.0,
        login_burst: 1,
        ..Default::default()
    };
    let (rate_addr, _, _tmp) = common::spawn_server_with_auth_and_admission(rate_config).await;
    let client = reqwest::Client::new();
    let first = client
        .post(format!("http://{rate_addr}/auth/login"))
        .json(&json!({"username": "root", "password": "wrong"}))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 401);
    let limited = client
        .post(format!("http://{rate_addr}/auth/login"))
        .json(&json!({"username": "root", "password": "wrong"}))
        .send()
        .await
        .unwrap();
    assert_eq!(limited.status(), 429);

    let overload_config = stellardb::server::admission::AdmissionConfig {
        max_login_concurrency: 0,
        login_burst: 1,
        ..Default::default()
    };
    let (overload_addr, _, _tmp) =
        common::spawn_server_with_auth_and_admission(overload_config).await;
    let overloaded = client
        .post(format!("http://{overload_addr}/auth/login"))
        .json(&json!({"username": "root", "password": "wrong"}))
        .send()
        .await
        .unwrap();
    assert_eq!(overloaded.status(), 503);
}

#[tokio::test]
async fn configured_login_body_limit_is_enforced_independently() {
    let (addr, _, _tmp) =
        common::spawn_server_with_auth_http_limits(Default::default(), 0, 2 * 1024, 8 * 1024).await;
    let response = reqwest::Client::new()
        .post(format!("http://{addr}/auth/login"))
        .json(&json!({"username": "root", "password": "x".repeat(3 * 1024)}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 413);
}

#[tokio::test]
async fn spoofed_forwarded_for_is_ignored_without_trusted_proxy_mode() {
    let config = stellardb::server::admission::AdmissionConfig {
        login_rate_per_second: 1_000.0,
        login_burst: 100,
        login_client_rate_per_second: 0.0,
        login_client_burst: 1,
        ..Default::default()
    };
    let (addr, _, _tmp) = common::spawn_server_with_auth_config(config, 0).await;
    let client = reqwest::Client::new();
    let first = client
        .post(format!("http://{addr}/auth/login"))
        .header("X-Forwarded-For", "198.51.100.1")
        .json(&json!({"username": "root", "password": "wrong"}))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), 401);
    let second = client
        .post(format!("http://{addr}/auth/login"))
        .header("X-Forwarded-For", "198.51.100.2")
        .json(&json!({"username": "root", "password": "wrong"}))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), 429);
}

#[tokio::test]
async fn trusted_proxy_hop_uses_right_aligned_forwarded_client_ip() {
    let config = stellardb::server::admission::AdmissionConfig {
        login_rate_per_second: 1_000.0,
        login_burst: 100,
        login_client_rate_per_second: 0.0,
        login_client_burst: 1,
        ..Default::default()
    };
    let (addr, _, _tmp) = common::spawn_server_with_auth_config(config, 1).await;
    let client = reqwest::Client::new();
    for forwarded in ["198.51.100.1", "203.0.113.2"] {
        let response = client
            .post(format!("http://{addr}/auth/login"))
            .header("X-Forwarded-For", forwarded)
            .json(&json!({"username": "root", "password": "wrong"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 401);
    }
    let repeated = client
        .post(format!("http://{addr}/auth/login"))
        .header("X-Forwarded-For", "198.51.100.1")
        .json(&json!({"username": "root", "password": "wrong"}))
        .send()
        .await
        .unwrap();
    assert_eq!(repeated.status(), 429);
}

#[tokio::test]
async fn http_db_admission_overload_returns_503() {
    let config = stellardb::server::admission::AdmissionConfig {
        max_db_concurrency: 0,
        ..Default::default()
    };
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth_and_admission(config).await;
    let token = common::login(addr, "root", &root_pw).await;
    let response = common::auth_client(&token)
        .post(format!("http://{addr}/sql"))
        .json(&json!({"query": "SELECT 1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 503);
}

#[tokio::test]
async fn identity_admin_api_manages_users_and_one_time_keys_without_leaks() {
    let (addr, root_pw, _tmp) = common::spawn_server_with_auth().await;
    let root_token = common::login(addr, "root", &root_pw).await;
    let root = common::auth_client(&root_token);

    let me = root
        .get(format!("http://{addr}/v1/auth/me"))
        .send()
        .await
        .unwrap();
    assert_eq!(me.status(), 200);
    assert_eq!(me.headers()["cache-control"], "no-store");
    let me: serde_json::Value = me.json().await.unwrap();
    assert_eq!(me["username"], "root");
    assert_eq!(me["authMethod"], "credentials");

    let created = root
        .post(format!("http://{addr}/v1/admin/users"))
        .json(&json!({
            "username": "station-admin-test",
            "password": "initial-secret",
            "attributes": {"team": "platform", "role": "reader"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201);
    let created: serde_json::Value = created.json().await.unwrap();
    assert_eq!(created["user"]["username"], "station-admin-test");
    assert_eq!(created["user"]["attributes"]["team"], "platform");

    let key_response = root
        .post(format!("http://{addr}/v1/admin/api-keys"))
        .json(&json!({"name": "station-admin-key", "username": "station-admin-test"}))
        .send()
        .await
        .unwrap();
    assert_eq!(key_response.status(), 201);
    assert_eq!(key_response.headers()["cache-control"], "no-store");
    let created_key: serde_json::Value = key_response.json().await.unwrap();
    let secret = created_key["secret"].as_str().unwrap().to_string();
    assert!(secret.starts_with("stl_"));
    assert_eq!(created_key["key"]["name"], "station-admin-key");
    let key_identity = common::auth_client(&secret)
        .get(format!("http://{addr}/v1/auth/me"))
        .send()
        .await
        .unwrap();
    assert_eq!(key_identity.status(), 200);
    let key_identity: serde_json::Value = key_identity.json().await.unwrap();
    assert_eq!(key_identity["username"], "station-admin-test");
    assert_eq!(key_identity["authMethod"], "apiKey");
    assert_eq!(key_identity["credentialName"], "station-admin-key");

    let listed = root
        .get(format!("http://{addr}/v1/admin/users?limit=100"))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    let serialized_users = listed.to_string();
    for forbidden in [
        "password_hash",
        "user_identity",
        "token_version",
        "initial-secret",
    ] {
        assert!(!serialized_users.contains(forbidden));
    }
    let managed = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|user| user["username"] == "station-admin-test")
        .unwrap();
    assert_eq!(managed["apiKeyCount"], 1);

    let listed_keys = root
        .get(format!("http://{addr}/v1/admin/api-keys?limit=100"))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    let serialized_keys = listed_keys.to_string();
    for forbidden in [
        &secret,
        "secret_hash",
        "key_hash",
        "lookup_id",
        "user_identity",
    ] {
        assert!(!serialized_keys.contains(forbidden));
    }
    assert!(serialized_keys.contains("station-admin-key"));

    let updated = root
        .patch(format!("http://{addr}/v1/admin/users/station-admin-test"))
        .json(&json!({"set": {"role": "writer"}, "remove": ["team"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(updated.status(), 200);
    let updated: serde_json::Value = updated.json().await.unwrap();
    assert_eq!(updated["user"]["attributes"]["role"], "writer");
    assert!(updated["user"]["attributes"].get("team").is_none());

    let password = root
        .put(format!(
            "http://{addr}/v1/admin/users/station-admin-test/password"
        ))
        .json(&json!({"password": "replacement-secret"}))
        .send()
        .await
        .unwrap();
    assert_eq!(password.status(), 200);
    let user_token = common::login(addr, "station-admin-test", "replacement-secret").await;
    let denied = common::auth_client(&user_token)
        .get(format!("http://{addr}/v1/admin/users"))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 403);

    let deleted = root
        .delete(format!("http://{addr}/v1/admin/users/station-admin-test"))
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), 204);
    let keys_after_delete = root
        .get(format!("http://{addr}/v1/admin/api-keys?limit=100"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!keys_after_delete.contains("station-admin-key"));
    let revoked = common::auth_client(&secret)
        .get(format!("http://{addr}/v1/auth/me"))
        .send()
        .await
        .unwrap();
    assert_eq!(revoked.status(), 401);

    let protected_root = root
        .delete(format!("http://{addr}/v1/admin/users/root"))
        .send()
        .await
        .unwrap();
    assert_eq!(protected_root.status(), 400);
}
