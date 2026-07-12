//! Drives the real spaceship-api clients against the mock router.

use std::num::NonZeroU32;
use std::time::Duration;

use spaceship_api::compat::{ContactDetails, NameserverConfig, SimpleClient};
use spaceship_api::types::{
    ContactId, DomainAvailabilityStatus, DomainContacts, DomainCreateRequest, DomainPrivacyLevel,
    DomainPrivacyOptions,
};

async fn serve(router: axum::Router) -> String {
    // Workspace feature unification builds reqwest with rustls-no-provider.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{addr}")
}

fn compat_client(base: &str) -> SimpleClient {
    SimpleClient::with_base_url(base, "test-key", "test-secret")
}

/// Progenitor-generated client with auth headers baked in.
fn raw_client(base: &str) -> spaceship_api::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("X-Api-Key", "test-key".parse().unwrap());
    headers.insert("X-Api-Secret", "test-secret".parse().unwrap());
    let http = reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap();
    spaceship_api::Client::new_with_client(base, http)
}

fn contact() -> ContactDetails {
    ContactDetails {
        first_name: "John".into(),
        last_name: "Doe".into(),
        email: "john@example.com".into(),
        phone: "+1.2345678901".into(),
        address1: "286 King St.".into(),
        city: "San Francisco".into(),
        country: "US".into(),
        organization: None,
        address2: None,
        state_province: Some("CA".into()),
        postal_code: Some("94103".into()),
    }
}

#[tokio::test]
async fn auth_is_required() {
    let base = serve(mock_spaceship::router()).await;
    let client = SimpleClient::with_base_url(&base, "", "");
    let err = client.check_availability("example.com").await.unwrap_err();
    match err {
        spaceship_api::compat::Error::Api { status, detail } => {
            assert_eq!(status, 401);
            assert!(detail.contains("detail"), "problem body: {detail}");
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}

#[tokio::test]
async fn check_availability() {
    let base = serve(mock_spaceship::router_with_seed(vec!["taken.example"])).await;
    let client = compat_client(&base);

    let free = client.check_availability("free.example").await.unwrap();
    assert_eq!(free.domain.as_deref(), Some("free.example"));
    assert!(free.premium_pricing.is_empty());

    let premium = client.check_availability("premium.example").await.unwrap();
    assert_eq!(premium.premium_pricing.len(), 1);
    assert_eq!(premium.premium_pricing[0].operation, "register");
    assert_eq!(premium.premium_pricing[0].currency, "USD");

    // The spec's availability field is `result`; the progenitor client
    // deserializes it as a typed enum.
    let raw = raw_client(&base);
    let free = raw
        .check_single_domain_availability("free.example")
        .await
        .unwrap();
    assert_eq!(free.result, DomainAvailabilityStatus::Available);
    let taken = raw
        .check_single_domain_availability("taken.example")
        .await
        .unwrap();
    assert_eq!(taken.result, DomainAvailabilityStatus::Taken);
}

#[tokio::test]
async fn contact_then_register_then_poll() {
    let base = serve(mock_spaceship::router()).await;
    let client = compat_client(&base);

    // Save contact; saving identical details returns the same ID.
    let saved = client.save_contact(&contact()).await.unwrap();
    assert_eq!(saved.contact_id.len(), 32);
    let again = client.save_contact(&contact()).await.unwrap();
    assert_eq!(again.contact_id, saved.contact_id);

    let roundtrip = client.get_contact(&saved.contact_id).await.unwrap();
    assert_eq!(roundtrip.email, "john@example.com");

    // Register through the progenitor client; op ID comes back in a header.
    let raw = raw_client(&base);
    let body = DomainCreateRequest {
        auto_renew: true,
        contacts: DomainContacts {
            registrant: saved.contact_id.parse::<ContactId>().unwrap(),
            admin: None,
            billing: None,
            tech: None,
            attributes: vec![],
        },
        privacy_protection: DomainPrivacyOptions {
            level: DomainPrivacyLevel::High,
            user_consent: true,
        },
        years: NonZeroU32::new(2).unwrap(),
    };
    let resp = raw.domain_create("shiny.example", &body).await.unwrap();
    assert_eq!(resp.status().as_u16(), 202);
    let op_id = resp
        .headers()
        .get("spaceship-async-operationid")
        .expect("async op header")
        .to_str()
        .unwrap()
        .to_string();

    // First poll is pending, then the operation completes.
    let op = client.get_async_operation(&op_id).await.unwrap();
    assert_eq!(op.status, "pending");
    let op = client
        .poll_until_complete(&op_id, Duration::from_secs(30))
        .await
        .unwrap();
    assert_eq!(op.status, "success");
    assert_eq!(op.op_type.as_deref(), Some("domains_Create"));

    let info = client.get_domain_info("shiny.example").await.unwrap();
    assert_eq!(info.name, "shiny.example");
    assert_eq!(info.auto_renew, Some(true));
    assert_eq!(info.lifecycle_status.as_deref(), Some("registered"));
    assert_eq!(
        info.expiration_date.as_deref(),
        Some("2028-01-01T00:00:00Z")
    );
    let ns = info.nameservers.unwrap();
    assert_eq!(ns.provider, "basic");

    // Registering with an unknown contact ID is rejected.
    let mut bad = body.clone();
    bad.contacts.registrant = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        .parse::<ContactId>()
        .unwrap();
    let err = raw.domain_create("other.example", &bad).await.unwrap_err();
    match err {
        spaceship_api::Error::ErrorResponse(rv) => assert_eq!(rv.status().as_u16(), 400),
        other => panic!("expected ErrorResponse, got {other}"),
    }
}

#[tokio::test]
async fn list_domains_paging() {
    let names: Vec<String> = (0..150).map(|i| format!("d{i:03}.example")).collect();
    let base = serve(mock_spaceship::router_with_seed(
        names.iter().map(String::as_str).collect(),
    ))
    .await;
    let client = compat_client(&base);

    let page1 = client.list_domains(0, 100).await.unwrap();
    assert_eq!(page1.items.len(), 100);
    assert_eq!(page1.items[0].name, "d000.example");

    let page2 = client.list_domains(100, 100).await.unwrap();
    assert_eq!(page2.items.len(), 50);
    assert_eq!(page2.items[49].name, "d149.example");

    let all = client.list_all_domains().await.unwrap();
    assert_eq!(all.len(), 150);
    let got: Vec<&str> = all.iter().map(|d| d.name.as_str()).collect();
    let want: Vec<&str> = names.iter().map(String::as_str).collect();
    assert_eq!(got, want);
}

#[tokio::test]
async fn nameservers_and_autorenew() {
    let base = serve(mock_spaceship::router_with_seed(vec!["one.example"])).await;
    let client = compat_client(&base);

    client
        .set_nameservers(
            "one.example",
            &NameserverConfig {
                provider: "custom".into(),
                hosts: Some(vec!["ns1.other.example".into(), "ns2.other.example".into()]),
            },
        )
        .await
        .unwrap();
    client.set_autorenew("one.example", true).await.unwrap();

    let info = client.get_domain_info("one.example").await.unwrap();
    assert_eq!(info.auto_renew, Some(true));
    let ns = info.nameservers.unwrap();
    assert_eq!(ns.provider, "custom");
    assert_eq!(
        ns.hosts.unwrap(),
        vec!["ns1.other.example", "ns2.other.example"]
    );

    // Unknown domain -> 404; custom with a single host -> 400.
    let err = client
        .set_autorenew("nope.example", true)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        spaceship_api::compat::Error::Api { status: 404, .. }
    ));
    let err = client
        .set_nameservers(
            "one.example",
            &NameserverConfig {
                provider: "custom".into(),
                hosts: Some(vec!["ns1.other.example".into()]),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        spaceship_api::compat::Error::Api { status: 400, .. }
    ));
}
