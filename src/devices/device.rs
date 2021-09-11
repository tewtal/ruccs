use tokio::sync::mpsc::{Sender, Receiver};
use crate::protocol;
use uuid::Uuid;

type Responder<T> = tokio::sync::oneshot::Sender<T>;

#[allow(dead_code)]
#[derive(Debug)]
pub enum DeviceRequest {
    Request { req: protocol::Request, resp: Responder<DeviceResponse> },
    Close
}

#[allow(dead_code)]
#[derive(Debug)]
pub enum DeviceResponse {
    Strings(Vec<String>),
    BinaryWriter((usize, Sender<Vec<u8>>)),
    BinaryReader((usize, Receiver<Vec<u8>>)),
    FileReader((usize, Receiver<Vec<u8>>)),
    Empty
}

#[allow(dead_code)]
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
    pub sender: Sender<DeviceRequest>,
}

#[allow(dead_code)]
impl Device {
    /* Helper function for sending a request to a device that creates a oneshot channel for the response and awaits a response */
    pub async fn request(&self, req: protocol::Request) -> Result<DeviceResponse, Box<dyn ::std::error::Error + Send + Sync>> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.sender.send(DeviceRequest::Request { req, resp: tx }).await?;
        Ok(rx.await?)
    }
}

