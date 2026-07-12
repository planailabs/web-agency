//! Drives the real `cloudflare_api::compat::SimpleClient` against the mock.

use cloudflare_api::compat::{CreateDnsRecord, Error, SimpleClient, UpdatePagesProject};

async fn spawn_mock() -> String {
    // Workspace feature unification builds reqwest with rustls-no-provider.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, mock_cloudflare::router())
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

async fn client() -> SimpleClient {
    SimpleClient::with_base_url(&spawn_mock().await, "test-token")
}

#[tokio::test]
async fn resolve_account_id() {
    let client = client().await;

    // Configured ID passes through untouched.
    assert_eq!(
        client.resolve_account_id("explicit-id").await.unwrap(),
        "explicit-id"
    );

    // Empty config resolves via GET /accounts to the seeded account.
    let resolved = client.resolve_account_id("").await.unwrap();
    assert_eq!(resolved.len(), 32);
    assert!(resolved.chars().all(|c| c.is_ascii_hexdigit()));
}

#[tokio::test]
async fn zone_lifecycle() {
    let client = client().await;
    let account_id = client.resolve_account_id("").await.unwrap();

    let zone = client.create_zone("example.com", &account_id).await.unwrap();
    assert_eq!(zone.name, "example.com");
    assert_eq!(zone.status, "pending");
    assert_eq!(zone.name_servers.as_ref().unwrap().len(), 2);
    assert_eq!(
        zone.account.as_ref().unwrap()["id"].as_str().unwrap(),
        account_id
    );

    // Name filter matches exactly.
    let found = client.list_zones(Some("example.com")).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, zone.id);
    assert!(client.list_zones(Some("other.com")).await.unwrap().is_empty());
    assert_eq!(client.list_zones(None).await.unwrap().len(), 1);

    let fetched = client.get_zone(&zone.id).await.unwrap();
    assert_eq!(fetched.name, "example.com");

    // Duplicate zone name is rejected.
    assert!(matches!(
        client.create_zone("example.com", &account_id).await,
        Err(Error::Api(errs)) if errs[0].code == 1061
    ));
}

#[tokio::test]
async fn dns_record_lifecycle() {
    let client = client().await;
    let account_id = client.resolve_account_id("").await.unwrap();
    let zone = client.create_zone("dns.example", &account_id).await.unwrap();

    assert!(client.list_dns_records(&zone.id).await.unwrap().is_empty());

    let record = client
        .create_dns_record(
            &zone.id,
            &CreateDnsRecord {
                record_type: "TXT".into(),
                name: "_acme-challenge.dns.example".into(),
                content: Some("token-value".into()),
                data: None,
                ttl: Some(120),
                proxied: Some(false),
                comment: Some("acme".into()),
                priority: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(record.record_type, "TXT");
    assert_eq!(record.name, "_acme-challenge.dns.example");
    assert_eq!(record.content.as_deref(), Some("token-value"));
    assert_eq!(record.proxied, Some(false));
    assert_eq!(record.comment.as_deref(), Some("acme"));

    let records = client.list_dns_records(&zone.id).await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, record.id);

    client.delete_dns_record(&zone.id, &record.id).await.unwrap();
    assert!(client.list_dns_records(&zone.id).await.unwrap().is_empty());

    // Deleting again fails with a Cloudflare API error.
    assert!(matches!(
        client.delete_dns_record(&zone.id, &record.id).await,
        Err(Error::Api(errs)) if errs[0].code == 81044
    ));
}

#[tokio::test]
async fn pages_project_lifecycle() {
    let client = client().await;
    let account_id = client.resolve_account_id("").await.unwrap();

    let project = client
        .create_pages_project(&account_id, "my-site", "main")
        .await
        .unwrap();
    assert_eq!(project.name, "my-site");
    assert_eq!(project.subdomain.as_deref(), Some("my-site.pages.dev"));
    assert_eq!(project.production_branch.as_deref(), Some("main"));

    let projects = client.list_pages_projects(&account_id).await.unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].name, "my-site");

    let updated = client
        .update_pages_project(
            &account_id,
            "my-site",
            &UpdatePagesProject {
                production_branch: Some("develop".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.production_branch.as_deref(), Some("develop"));

    let fetched = client.get_pages_project(&account_id, "my-site").await.unwrap();
    assert_eq!(fetched.production_branch.as_deref(), Some("develop"));
    assert_eq!(fetched.id, project.id);
}

#[tokio::test]
async fn check_domains() {
    let client = client().await;
    let account_id = client.resolve_account_id("").await.unwrap();

    // A domain that already exists as a zone is not registrable.
    client.create_zone("taken.example", &account_id).await.unwrap();

    let checks = client
        .check_domains(
            &account_id,
            &["fresh.example".to_string(), "taken.example".to_string()],
        )
        .await
        .unwrap();
    assert_eq!(checks.len(), 2);

    let fresh = checks.iter().find(|c| c.name == "fresh.example").unwrap();
    assert!(fresh.registrable);
    let pricing = fresh.pricing.as_ref().unwrap();
    assert_eq!(pricing.currency, "USD");
    assert!(pricing.registration_cost.is_some());

    let taken = checks.iter().find(|c| c.name == "taken.example").unwrap();
    assert!(!taken.registrable);
    assert!(taken.reason.is_some());
}

#[tokio::test]
async fn rejects_missing_token() {
    let base = spawn_mock().await;
    let client = SimpleClient::with_base_url(&base, "");
    assert!(matches!(
        client.list_accounts().await,
        Err(Error::Api(errs)) if errs[0].code == 9109
    ));
}
