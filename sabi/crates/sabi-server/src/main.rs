use sabi_core::{Request, Response, ErrorPayload, ErrorCode, TxId};
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use serde_json::json;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:4000").await?;
    println!("sabi-server listening on 127.0.0.1:4000");

    loop {
        let (socket, addr) = listener.accept().await?;
        println!("New connection from {}", addr);
        tokio::spawn(handle_client(socket));
    }
}

async fn handle_client(mut socket: TcpStream) {
    let mut buffer = vec![0u8; 4096];

    loop {
        match socket.read(&mut buffer).await {
            Ok(0) => {
                println!("Client disconnected");
                break;
            }
            Ok(n) => {
                let msg = &buffer[..n];
                match serde_json::from_slice::<Request>(msg) {
                    Ok(request) => {
                        println!("Received request: {:?}", request);
                        // Echo back OK response for now
                        let response = Response::Ok {
                            request_id: match &request {
                                Request::Query { request_id, .. } => *request_id,
                                Request::Subscribe { request_id, .. } => *request_id,
                                Request::Notify { request_id, .. } => *request_id,
                            },
                            payload: json!({"status": "ok"}),
                        };
                        let resp_bytes = serde_json::to_vec(&response).unwrap();
                        if let Err(e) = socket.write_all(&resp_bytes).await {
                            eprintln!("Failed to send response: {}", e);
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to parse request: {}", e);
                        // Send error back
                        let response = Response::Error {
                            request_id: TxId::new(),
                            payload: ErrorPayload {
                                code: ErrorCode::SyntaxError,
                                message: format!("Invalid request: {}", e),
                            },
                        };
                        let resp_bytes = serde_json::to_vec(&response).unwrap();
                        let _ = socket.write_all(&resp_bytes).await;
                    }
                }
            }
            Err(e) => {
                eprintln!("Socket error: {}", e);
                break;
            }
        }
    }
}
