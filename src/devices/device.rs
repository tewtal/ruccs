use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::sync::mpsc::{Sender, Receiver, channel};
use uuid::Uuid;

type Responder<T> = tokio::sync::oneshot::Sender<T>;

#[derive(Debug)]
pub enum Command {
    GetInformation { resp: Responder<String> },
    Close
}

#[derive(Debug)]
pub enum DeviceManagerCommand {
    Close
}

#[derive(Debug)]
pub enum DeviceInfo {
    ConnectionClosed(String)
}

/* A Device is what's used from the outside to communicate with the actual devices.
   It contains a channel to send commands to a device, and some static information about it.
*/
#[derive(Debug, Clone)]
pub struct Device {
    pub name: String,
    pub id: Uuid,
    pub sender: Sender<Command>,
}

impl Device {
    pub async fn get_information(&self) -> Result<String, Box<dyn ::std::error::Error>> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.sender.send(Command::GetInformation { resp: tx }).await?;
        Ok(rx.await?)
    }
}

