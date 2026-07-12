//! Stateful in-memory mock of the Cloudflare v4 API.
//!
//! Implements only the endpoints used by `cloudflare_api::compat::SimpleClient`,
//! wrapping every response in the standard Cloudflare envelope
//! (`{success, errors, messages, result, result_info}`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, Query, Request, State};
use axum::http::{header::AUTHORIZATION, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

#[derive(Default)]
struct Store {
    accounts: Vec<Value>,
    zones: Vec<Value>,
    // zone_id -> records
    dns_records: HashMap<String, Vec<Value>>,
    // account_id -> projects
    pages_projects: HashMap<String, Vec<Value>>,
}

type AppState = Arc<Mutex<Store>>;

fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

const CREATED_ON: &str = "2026-01-01T00:00:00.000000Z";

// ── Envelope helpers ──────────────────────────────────────────────────

fn ok(result: Value) -> Response {
    Json(json!({
        "success": true,
        "errors": [],
        "messages": [],
        "result": result,
    }))
    .into_response()
}

fn ok_list(result: Vec<Value>) -> Response {
    let count = result.len();
    Json(json!({
        "success": true,
        "errors": [],
        "messages": [],
        "result": result,
        "result_info": {
            "page": 1,
            "per_page": 50,
            "count": count,
            "total_count": count,
            "total_pages": 1,
        },
    }))
    .into_response()
}

fn api_error(status: StatusCode, code: i64, message: &str) -> Response {
    (
        status,
        Json(json!({
            "success": false,
            "errors": [{"code": code, "message": message}],
            "messages": [],
            "result": null,
        })),
    )
        .into_response()
}

fn not_found(what: &str) -> Response {
    api_error(
        StatusCode::NOT_FOUND,
        7003,
        &format!("Could not route to /{what}, perhaps your object identifier is invalid?"),
    )
}

// ── Auth ──────────────────────────────────────────────────────────────

async fn require_auth(req: Request, next: Next) -> Response {
    let authed = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|token| !token.trim().is_empty());
    if !authed {
        return api_error(StatusCode::FORBIDDEN, 9109, "Invalid access token");
    }
    next.run(req).await
}

// ── Accounts ──────────────────────────────────────────────────────────

async fn list_accounts(State(state): State<AppState>) -> Response {
    let store = state.lock().unwrap();
    ok_list(store.accounts.clone())
}

fn account_exists(store: &Store, account_id: &str) -> bool {
    store
        .accounts
        .iter()
        .any(|a| a["id"].as_str() == Some(account_id))
}

// ── Zones ─────────────────────────────────────────────────────────────

async fn list_zones(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let store = state.lock().unwrap();
    let zones = store
        .zones
        .iter()
        .filter(|z| match params.get("name") {
            Some(name) => z["name"].as_str() == Some(name.as_str()),
            None => true,
        })
        .cloned()
        .collect();
    ok_list(zones)
}

async fn create_zone(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    let Some(name) = body["name"].as_str().map(str::to_string) else {
        return api_error(StatusCode::BAD_REQUEST, 1004, "Invalid zone name");
    };
    let account_id = body["account"]["id"].as_str().unwrap_or_default();

    let mut store = state.lock().unwrap();
    let Some(account) = store
        .accounts
        .iter()
        .find(|a| a["id"].as_str() == Some(account_id))
        .cloned()
    else {
        return api_error(StatusCode::NOT_FOUND, 1099, "Invalid account identifier");
    };
    if store
        .zones
        .iter()
        .any(|z| z["name"].as_str() == Some(name.as_str()))
    {
        return api_error(
            StatusCode::BAD_REQUEST,
            1061,
            &format!("Zone {name} already exists"),
        );
    }
    let id = new_id();
    let zone = json!({
        "id": id,
        "name": name,
        "status": "pending",
        "paused": false,
        "type": "full",
        "name_servers": ["ada.ns.mock-cloudflare.test", "bob.ns.mock-cloudflare.test"],
        "account": account,
        "created_on": CREATED_ON,
        "modified_on": CREATED_ON,
    });
    store.zones.push(zone.clone());
    store.dns_records.insert(id, Vec::new());
    ok(zone)
}

async fn get_zone(State(state): State<AppState>, Path(zone_id): Path<String>) -> Response {
    let store = state.lock().unwrap();
    match store
        .zones
        .iter()
        .find(|z| z["id"].as_str() == Some(zone_id.as_str()))
    {
        Some(zone) => ok(zone.clone()),
        None => not_found("zones"),
    }
}

// ── DNS records ───────────────────────────────────────────────────────

async fn list_dns_records(State(state): State<AppState>, Path(zone_id): Path<String>) -> Response {
    let store = state.lock().unwrap();
    match store.dns_records.get(&zone_id) {
        Some(records) => ok_list(records.clone()),
        None => not_found("dns_records"),
    }
}

async fn create_dns_record(
    State(state): State<AppState>,
    Path(zone_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let mut store = state.lock().unwrap();
    if !store.dns_records.contains_key(&zone_id) {
        return not_found("dns_records");
    }
    let (Some(record_type), Some(name)) = (
        body["type"].as_str().map(str::to_string),
        body["name"].as_str().map(str::to_string),
    ) else {
        return api_error(StatusCode::BAD_REQUEST, 9207, "Invalid or missing type/name");
    };
    let record = json!({
        "id": new_id(),
        "type": record_type,
        "name": name,
        "content": body.get("content").cloned().unwrap_or(Value::Null),
        "data": body.get("data").cloned().unwrap_or(Value::Null),
        "ttl": body.get("ttl").cloned().unwrap_or(json!(1)),
        "proxied": body.get("proxied").cloned().unwrap_or(json!(false)),
        "comment": body.get("comment").cloned().unwrap_or(Value::Null),
        "priority": body.get("priority").cloned().unwrap_or(Value::Null),
        "created_on": CREATED_ON,
        "modified_on": CREATED_ON,
    });
    store
        .dns_records
        .get_mut(&zone_id)
        .unwrap()
        .push(record.clone());
    ok(record)
}

async fn delete_dns_record(
    State(state): State<AppState>,
    Path((zone_id, record_id)): Path<(String, String)>,
) -> Response {
    let mut store = state.lock().unwrap();
    let Some(records) = store.dns_records.get_mut(&zone_id) else {
        return not_found("dns_records");
    };
    let before = records.len();
    records.retain(|r| r["id"].as_str() != Some(record_id.as_str()));
    if records.len() == before {
        return api_error(StatusCode::NOT_FOUND, 81044, "Record does not exist.");
    }
    ok(json!({"id": record_id}))
}

// ── Registrar ─────────────────────────────────────────────────────────

async fn domain_check(
    State(state): State<AppState>,
    Path(account_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let store = state.lock().unwrap();
    if !account_exists(&store, &account_id) {
        return api_error(StatusCode::NOT_FOUND, 1099, "Invalid account identifier");
    }
    let domains: Vec<Value> = body["domains"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|d| d.as_str())
        .map(|name| {
            // Domains already present as zones are considered taken.
            let taken = store
                .zones
                .iter()
                .any(|z| z["name"].as_str() == Some(name));
            if taken {
                json!({
                    "name": name,
                    "registrable": false,
                    "tier": null,
                    "pricing": null,
                    "reason": "domain is not available",
                })
            } else {
                json!({
                    "name": name,
                    "registrable": true,
                    "tier": "standard",
                    "pricing": {
                        "currency": "USD",
                        "registration_cost": "10.44",
                        "renewal_cost": "10.44",
                    },
                    "reason": null,
                })
            }
        })
        .collect();
    ok(json!({"domains": domains}))
}

// ── Pages projects ────────────────────────────────────────────────────

async fn list_pages_projects(
    State(state): State<AppState>,
    Path(account_id): Path<String>,
) -> Response {
    let store = state.lock().unwrap();
    if !account_exists(&store, &account_id) {
        return api_error(StatusCode::NOT_FOUND, 1099, "Invalid account identifier");
    }
    ok_list(
        store
            .pages_projects
            .get(&account_id)
            .cloned()
            .unwrap_or_default(),
    )
}

async fn create_pages_project(
    State(state): State<AppState>,
    Path(account_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let Some(name) = body["name"].as_str().map(str::to_string) else {
        return api_error(StatusCode::BAD_REQUEST, 8000000, "Invalid project name");
    };
    let mut store = state.lock().unwrap();
    if !account_exists(&store, &account_id) {
        return api_error(StatusCode::NOT_FOUND, 1099, "Invalid account identifier");
    }
    let projects = store.pages_projects.entry(account_id).or_default();
    if projects
        .iter()
        .any(|p| p["name"].as_str() == Some(name.as_str()))
    {
        return api_error(
            StatusCode::CONFLICT,
            8000009,
            "A project with this name already exists.",
        );
    }
    let project = json!({
        "id": new_id(),
        "name": name,
        "subdomain": format!("{name}.pages.dev"),
        "domains": [format!("{name}.pages.dev")],
        "production_branch": body.get("production_branch").cloned().unwrap_or(json!("main")),
        "source": body.get("source").cloned().unwrap_or(Value::Null),
        "build_config": body.get("build_config").cloned().unwrap_or(Value::Null),
        "created_on": CREATED_ON,
    });
    projects.push(project.clone());
    ok(project)
}

async fn get_pages_project(
    State(state): State<AppState>,
    Path((account_id, project_name)): Path<(String, String)>,
) -> Response {
    let store = state.lock().unwrap();
    match store.pages_projects.get(&account_id).and_then(|ps| {
        ps.iter()
            .find(|p| p["name"].as_str() == Some(project_name.as_str()))
    }) {
        Some(project) => ok(project.clone()),
        None => api_error(StatusCode::NOT_FOUND, 8000007, "Project not found."),
    }
}

async fn update_pages_project(
    State(state): State<AppState>,
    Path((account_id, project_name)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let mut store = state.lock().unwrap();
    let Some(project) = store.pages_projects.get_mut(&account_id).and_then(|ps| {
        ps.iter_mut()
            .find(|p| p["name"].as_str() == Some(project_name.as_str()))
    }) else {
        return api_error(StatusCode::NOT_FOUND, 8000007, "Project not found.");
    };
    for field in ["production_branch", "source", "build_config"] {
        if let Some(value) = body.get(field) {
            project[field] = value.clone();
        }
    }
    ok(project.clone())
}

// ── Router ────────────────────────────────────────────────────────────

pub fn router() -> Router {
    let mut store = Store::default();
    store.accounts.push(json!({
        "id": new_id(),
        "name": "Mock Account",
    }));
    let state: AppState = Arc::new(Mutex::new(store));

    Router::new()
        .route("/accounts", get(list_accounts))
        .route("/zones", get(list_zones).post(create_zone))
        .route("/zones/{zone_id}", get(get_zone))
        .route(
            "/zones/{zone_id}/dns_records",
            get(list_dns_records).post(create_dns_record),
        )
        .route(
            "/zones/{zone_id}/dns_records/{record_id}",
            delete(delete_dns_record),
        )
        .route(
            "/accounts/{account_id}/registrar/domain-check",
            post(domain_check),
        )
        .route(
            "/accounts/{account_id}/pages/projects",
            get(list_pages_projects).post(create_pages_project),
        )
        .route(
            "/accounts/{account_id}/pages/projects/{project_name}",
            get(get_pages_project).patch(update_pages_project),
        )
        .layer(middleware::from_fn(require_auth))
        .with_state(state)
}
