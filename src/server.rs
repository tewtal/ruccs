use futures::{SinkExt, StreamExt};
use futures::future::join_all;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::net::{TcpListener, TcpStream};
use futures::stream::SplitSink;
use tokio_tungstenite::{accept_async, WebSocketStream};
use tokio_tungstenite::tungstenite::Message;
use tungstenite::{Result};
use serde_json;
use crate::manager::{Manager};
use crate::protocol::{WSRequest, WSResponse, Request, Command};
use crate::devices::device::DeviceResponse;
use uuid::Uuid;

enum ClientState {
    Ready,
    Busy,
    SendingData((usize, tokio::sync::mpsc::Sender<Vec<u8>>)),
}

struct Client {
    name: String,
    state: ClientState,
    device_id: Option<Uuid>,
}

async fn handle_request(request: Request, manager: &Arc<RwLock<Manager>>, write: &mut SplitSink<WebSocketStream<TcpStream>, Message>, client: &mut Client) -> Result<ClientState, Box<dyn std::error::Error + Send + Sync>> {
    client.state = ClientState::Busy;
    let response = match (&request.command, client.device_id) {
        /* Handle commands that are specific to the websocket connection and shouldn't be sent to any device */
        (Command::DeviceList, _) => Some(manager.read().await.get_names()),
        (Command::Attach(device_name), _) => {
            if let Some((id, _)) = manager.read().await.get_device(device_name) {
                client.device_id = Some(*id);
            } else {
                Err("Invalid device name")?                
            }
            Some(vec![])
        },
        (Command::Name(name), _) => {
            client.name = name.to_string();
            None
        },

        /* As long as we're attached to a device, pass any remaining commands through to the device manager so it can be 
           executed on a device. */
        (_, Some(id)) => {
            {
                let response = {
                    let mgr = manager.read().await;
                    mgr.request(request, id).await?
                };

                /* Handle the response here, depending on the response type from the device */
                match response {
                    DeviceResponse::Strings(s) => Some(s),
                    DeviceResponse::BinaryReader((size, mut receiver)) => {
                        /* Forward binary data from the device to the websocket */
                        let mut received_len = 0;
                        
                        while received_len < size {
                            if let Some(data) = receiver.recv().await {
                                received_len += data.len();                                
                                write.send(Message::Binary(data)).await?;
                            } else {
                                /* Received nothing from the channel, possibly channel broke? */
                                Err("Could not read binary data from the device")?
                            }
                        }
                        None
                    },
                    DeviceResponse::BinaryWriter(send_data_info) => {
                        /* This is a special case where we need to set up to receive binary data and pass it through to the device */
                        /* Return early here with the special SendingData state */
                        return Ok(ClientState::SendingData(send_data_info));
                    }
                    _ => None
                }                
            }
        },
        _ => None
    };

    if let Some(response) = response {
        let data = serde_json::to_string(&WSResponse { results: response })?;
        println!("Sending response: {:?}", data);
        write.send(Message::Text(data)).await?;
    }

    Ok(ClientState::Ready)
}

/* Handles incoming connections */
async fn handle_client(manager: Arc<RwLock<Manager>>, peer: SocketAddr, stream: TcpStream) -> Result<(), Box<dyn std::error::Error + Send + Sync>> { 
    let ws_stream = accept_async(stream).await?;
    println!("Accepted websocket connection from: {:?}", peer);        
    let (mut write, mut read) = ws_stream.split();
    
    let mut client = Client { 
        name: format!("WebSocket Connection: {:?}", peer),
        state: ClientState::Ready,
        device_id: None
    };

    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(message)) => {
                        match message {
                            Message::Text(txt) => {                                
                                /* Incoming text command, parse it and make sure it's a valid command */
                                if let Ok(ws_request) = serde_json::from_str::<WSRequest>(&txt) {
                                    if let Ok(request) = Request::from_wsrequest(&ws_request) {
                                        println!("Valid request received: {:?}", &request);
                                        client.state = handle_request(request, &manager, &mut write, &mut client).await?;
                                    } else {
                                        Err("The request is not a valid USB2SNES request.")?
                                    }
                                } else {
                                    Err("The request is not in JSON format or does not include the required fields.")?
                                }
                            },
                            Message::Binary(data) => {
                                if let ClientState::SendingData((mut remaining, ref sender)) = client.state {
                                    remaining -= data.len();
                                    sender.send(data).await.unwrap();
                                    if remaining > 0 {
                                        client.state = ClientState::SendingData((remaining, sender.clone()));
                                    } else {
                                        client.state = ClientState::Ready;
                                    }
                                }
                            }
                            _ => ()
                        }
                    },
                    Some(Err(e)) => {
                        println!("Error parsing websocket message: {:?}", e);
                        break
                    },
                    None => {
                        println!("The connection was closed by the remote peer.");
                        break
                    }
                }
            }
        }
    }

    Ok(())
}

/* Starts the tokio tcp listeners for the ports we want to listen to */
pub async fn run_server(manager: Arc<RwLock<Manager>>)
{
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
                        let _ = tokio::spawn(async move {
                            if let Err(e) = handle_client(thread_mgr, peer, stream).await {
                                println!("Websocket client exited with error: {:?}", e);
                            }
                        });
                    }
                }               
            }));
        } else {
            println!("Could not listen to port");
        }
    }
    
    join_all(handles).await;
}