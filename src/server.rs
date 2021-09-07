use futures::{StreamExt};
use futures::future::join_all;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::accept_async;
use tungstenite::Result;
use crate::manager::{Manager};

/* Handles incoming connections */
async fn handle_client(_manager: Arc<RwLock<Manager>> , peer: SocketAddr, stream: TcpStream) -> Result<(), Box<dyn std::error::Error + Send + Sync>> { 
    let ws_stream = accept_async(stream).await?;
    println!("Accepted websocket connection from: {:?}", peer);    
    
    let (_write, mut read) = ws_stream.split();

    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(_msg) => {                        
                        
                    },
                    None => break,
                }
            }
        }
    }

    Ok(())
}

/* Starts the tokio tcp listeners for the ports we want to listen to */
pub async fn run_server(manager: Arc<RwLock<Manager>>)
{
    println!("Run_server");
    let ports = vec![23074, 8080];
    let mut handles = Vec::new();
    for port in ports {
        let port_mgr = manager.clone();
        if let Ok(listener) = TcpListener::bind(format!("127.0.0.1:{}", port)).await {
            println!("Listener started on port: {}", port);
            handles.push(tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    let thread_mgr = port_mgr.clone();                
                    if let Ok(peer) = stream.peer_addr() {
                        let _ = tokio::spawn(handle_client(thread_mgr, peer, stream));
                    }
                }               
            }));
        } else {
            println!("Could not listen to port");
        }
    }
    
    join_all(handles).await;
}