#[tokio::main]
async fn main() {
    let listen = std::env::var("LISTEN").unwrap_or_else(|_| "127.0.0.1:8611".to_string());
    let listener = tokio::net::TcpListener::bind(&listen)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {listen}: {e}"));
    println!(
        "mock-spaceship listening on {}",
        listener.local_addr().unwrap()
    );
    axum::serve(listener, mock_spaceship::router())
        .await
        .unwrap();
}
