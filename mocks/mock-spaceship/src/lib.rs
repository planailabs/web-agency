//! Stateful in-memory mock of the Spaceship domain-registrar API.
//!
//! Implements just the endpoints the `spaceship-api` compat/progenitor
//! clients use, with response shapes matching `openapi-trimmed.json`.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use axum::extract::{Path, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use axum::{Json, Router};
use serde_json::{json, Value};

const BASIC_NAMESERVERS: [&str; 2] = ["launch1.spaceship.net", "launch2.spaceship.net"];
const ISO_DATE: &str = "2026-01-01T00:00:00Z";

#[derive(Debug, Clone)]
struct Domain {
    auto_renew: bool,
    registration_date: String,
    expiration_date: String,
    lifecycle_status: String,
    ns_provider: String,
    ns_hosts: Vec<String>,
    contacts: Value,
}

impl Domain {
    fn new(years: u32, auto_renew: bool, contacts: Value) -> Self {
        Self {
            auto_renew,
            registration_date: ISO_DATE.to_string(),
            expiration_date: format!("{}-01-01T00:00:00Z", 2026 + years),
            lifecycle_status: "registered".to_string(),
            ns_provider: "basic".to_string(),
            ns_hosts: BASIC_NAMESERVERS.iter().map(|s| s.to_string()).collect(),
            contacts,
        }
    }
}

#[derive(Debug)]
struct Operation {
    op_type: String,
    /// Number of GETs that still report `pending` before `success`.
    pending_polls: u32,
}

#[derive(Default)]
struct Inner {
    domains: BTreeMap<String, Domain>,
    contacts: HashMap<String, Value>,
    operations: HashMap<String, Operation>,
}

#[derive(Clone, Default)]
struct AppState(Arc<Mutex<Inner>>);

/// Router with empty state.
pub fn router() -> Router {
    with_state(AppState::default())
}

/// Router pre-seeded with registered domains (auto-renew off, basic NS).
pub fn router_with_seed(domains: Vec<&str>) -> Router {
    let state = AppState::default();
    {
        let mut inner = state.0.lock().unwrap();
        for name in domains {
            inner
                .domains
                .insert(name.to_string(), Domain::new(1, false, json!({})));
        }
    }
    with_state(state)
}

fn with_state(state: AppState) -> Router {
    Router::new()
        .route("/v1/domains", get(list_domains))
        .route("/v1/domains/{domain}", get(get_domain).post(create_domain))
        .route("/v1/domains/{domain}/available", get(check_availability))
        .route("/v1/domains/{domain}/nameservers", put(set_nameservers))
        .route("/v1/domains/{domain}/autorenew", put(set_autorenew))
        .route("/v1/contacts", put(save_contact))
        .route("/v1/contacts/{contact}", get(get_contact))
        .route("/v1/async-operations/{operationId}", get(get_operation))
        .layer(middleware::from_fn(require_auth))
        .with_state(state)
}

// ── Errors / auth ─────────────────────────────────────────────────────

fn problem(status: StatusCode, body: Value) -> Response {
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "application/problem+json")],
        body.to_string(),
    )
        .into_response()
}

fn error_detail(status: StatusCode, detail: &str) -> Response {
    problem(status, json!({ "detail": detail }))
}

fn not_found() -> Response {
    error_detail(StatusCode::NOT_FOUND, "The requested object was not found.")
}

fn validation_error(field: &str, details: &str) -> Response {
    problem(
        StatusCode::BAD_REQUEST,
        json!({
            "detail": "The request is invalid.",
            "data": [{ "field": field, "details": details }],
        }),
    )
}

async fn require_auth(req: Request, next: Next) -> Response {
    fn has(headers: &axum::http::HeaderMap, name: &str) -> bool {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| !v.is_empty())
    }
    if !has(req.headers(), "x-api-key") || !has(req.headers(), "x-api-secret") {
        return error_detail(
            StatusCode::UNAUTHORIZED,
            "The user's request was not authorized properly.",
        );
    }
    next.run(req).await
}

// ── Domains ───────────────────────────────────────────────────────────

fn domain_json(name: &str, d: &Domain) -> Value {
    json!({
        "name": name,
        "unicodeName": name,
        "isPremium": name.starts_with("premium"),
        "autoRenew": d.auto_renew,
        "registrationDate": d.registration_date,
        "expirationDate": d.expiration_date,
        "lifecycleStatus": d.lifecycle_status,
        "verificationStatus": "success",
        "eppStatuses": [],
        "suspensions": [],
        "privacyProtection": { "contactForm": true, "level": "high" },
        "nameservers": { "provider": d.ns_provider, "hosts": d.ns_hosts },
        "contacts": d.contacts,
    })
}

async fn list_domains(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let parse = |key: &str| params.get(key).and_then(|v| v.parse::<i64>().ok());
    let (Some(take), Some(skip)) = (parse("take"), parse("skip")) else {
        return validation_error("take", "take and skip query parameters are required");
    };
    if !(1..=100).contains(&take) || skip < 0 {
        return validation_error("take", "take must be 1..100 and skip must be >= 0");
    }
    let inner = state.0.lock().unwrap();
    let items: Vec<Value> = inner
        .domains
        .iter()
        .skip(skip as usize)
        .take(take as usize)
        .map(|(name, d)| domain_json(name, d))
        .collect();
    Json(json!({ "items": items, "total": inner.domains.len() })).into_response()
}

async fn get_domain(State(state): State<AppState>, Path(domain): Path<String>) -> Response {
    let inner = state.0.lock().unwrap();
    match inner.domains.get(&domain) {
        Some(d) => Json(domain_json(&domain, d)).into_response(),
        None => not_found(),
    }
}

async fn create_domain(
    State(state): State<AppState>,
    Path(domain): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let Some(auto_renew) = body.get("autoRenew").and_then(Value::as_bool) else {
        return validation_error("autoRenew", "autoRenew is required");
    };
    let years = match body.get("years").and_then(Value::as_u64) {
        Some(y @ 1..=10) => y as u32,
        _ => return validation_error("years", "years must be between 1 and 10"),
    };
    if body.get("privacyProtection").is_none() {
        return validation_error("privacyProtection", "privacyProtection is required");
    }
    let contacts = body.get("contacts").cloned().unwrap_or(Value::Null);
    let Some(registrant) = contacts.get("registrant").and_then(Value::as_str) else {
        return validation_error("contacts.registrant", "registrant contact ID is required");
    };

    let mut inner = state.0.lock().unwrap();
    if !inner.contacts.contains_key(registrant) {
        return validation_error("contacts.registrant", "unknown contact ID");
    }
    if inner.domains.contains_key(&domain) {
        return error_detail(
            StatusCode::BAD_REQUEST,
            "The domain is not available for registration.",
        );
    }
    inner
        .domains
        .insert(domain, Domain::new(years, auto_renew, contacts));

    let op_id = uuid::Uuid::new_v4().simple().to_string();
    inner.operations.insert(
        op_id.clone(),
        Operation {
            op_type: "domains_Create".to_string(),
            pending_polls: 1,
        },
    );
    (
        StatusCode::ACCEPTED,
        [("spaceship-async-operationid", op_id)],
    )
        .into_response()
}

async fn check_availability(State(state): State<AppState>, Path(domain): Path<String>) -> Response {
    let inner = state.0.lock().unwrap();
    let result = if inner.domains.contains_key(&domain) {
        "taken"
    } else {
        "available"
    };
    // Domains named premium* carry premium register pricing.
    let pricing = if domain.starts_with("premium") {
        json!([{ "operation": "register", "price": 200.0, "currency": "USD" }])
    } else {
        json!([])
    };
    Json(json!({ "domain": domain, "result": result, "premiumPricing": pricing })).into_response()
}

async fn set_nameservers(
    State(state): State<AppState>,
    Path(domain): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let provider = body.get("provider").and_then(Value::as_str);
    let hosts: Vec<String> = body
        .get("hosts")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let (provider, hosts) = match provider {
        Some("basic") => (
            "basic",
            BASIC_NAMESERVERS.iter().map(|s| s.to_string()).collect(),
        ),
        Some("custom") if hosts.len() >= 2 => ("custom", hosts),
        Some("custom") => {
            return validation_error("hosts", "custom provider requires at least 2 hosts");
        }
        _ => return validation_error("provider", "provider must be basic or custom"),
    };

    let mut inner = state.0.lock().unwrap();
    let Some(d) = inner.domains.get_mut(&domain) else {
        return not_found();
    };
    d.ns_provider = provider.to_string();
    d.ns_hosts = hosts.clone();
    Json(json!({ "provider": provider, "hosts": hosts })).into_response()
}

async fn set_autorenew(
    State(state): State<AppState>,
    Path(domain): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let Some(enabled) = body.get("isEnabled").and_then(Value::as_bool) else {
        return validation_error("isEnabled", "isEnabled is required");
    };
    let mut inner = state.0.lock().unwrap();
    let Some(d) = inner.domains.get_mut(&domain) else {
        return not_found();
    };
    d.auto_renew = enabled;
    Json(json!({ "isEnabled": enabled })).into_response()
}

// ── Contacts ──────────────────────────────────────────────────────────

async fn save_contact(State(state): State<AppState>, Json(body): Json<Value>) -> Response {
    for field in [
        "firstName",
        "lastName",
        "email",
        "address1",
        "city",
        "country",
        "phone",
    ] {
        if body
            .get(field)
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            return validation_error(field, "field is required");
        }
    }
    let mut inner = state.0.lock().unwrap();
    // Saving identical details returns the existing contact's ID.
    if let Some(id) = inner
        .contacts
        .iter()
        .find(|(_, v)| **v == body)
        .map(|(id, _)| id.clone())
    {
        return Json(json!({ "contactId": id })).into_response();
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    inner.contacts.insert(id.clone(), body);
    Json(json!({ "contactId": id })).into_response()
}

async fn get_contact(State(state): State<AppState>, Path(contact): Path<String>) -> Response {
    let inner = state.0.lock().unwrap();
    match inner.contacts.get(&contact) {
        Some(c) => Json(c.clone()).into_response(),
        None => not_found(),
    }
}

// ── Async operations ──────────────────────────────────────────────────

async fn get_operation(State(state): State<AppState>, Path(op_id): Path<String>) -> Response {
    let mut inner = state.0.lock().unwrap();
    let Some(op) = inner.operations.get_mut(&op_id) else {
        return not_found();
    };
    let status = if op.pending_polls > 0 {
        op.pending_polls -= 1;
        "pending"
    } else {
        "success"
    };
    Json(json!({
        "status": status,
        "type": op.op_type,
        "details": {},
        "createdAt": ISO_DATE,
    }))
    .into_response()
}
