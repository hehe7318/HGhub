#[tokio::main]
async fn main() {
    if let Err(error) = huangguo_client_lib::web::run().await {
        eprintln!("hghub-web: {error}");
        std::process::exit(1);
    }
}
