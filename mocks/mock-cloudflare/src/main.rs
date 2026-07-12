#[tokio::main]
async fn main() {
    let addr = std::env::var("LISTEN").unwrap_or_else(|_| "127.0.0.1:8612".to_string());
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {addr}: {e}"));
    println!(
        "mock-cloudflare listening on {}",
        listener.local_addr().unwrap()
    );
    axum::serve(listener, mock_cloudflare::router())
        .await
        .unwrap();
}
