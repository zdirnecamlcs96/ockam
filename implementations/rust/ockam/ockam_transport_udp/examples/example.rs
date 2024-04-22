use std::io;
use tokio::net::UdpSocket;

#[tokio::main]
async fn main() -> io::Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:8080").await?;
    let len = socket
        .send_to(b"hello world", "216.58.208.206:8081")
        .await?;

    println!("Sent {} bytes", len);

    Ok(())
}
