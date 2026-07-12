#[tokio::main]
async fn main() {
    let listen = std::env::var("LISTEN").unwrap_or_else(|_| "127.0.0.1:8613".to_string());
    let listener = tokio::net::TcpListener::bind(&listen)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {listen}: {e}"));
    println!(
        "mock-changedetection listening on {}",
        listener.local_addr().expect("local_addr")
    );
    axum::serve(listener, mock_changedetection::router())
        .await
        .expect("serve");
}
