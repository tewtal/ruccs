use async_trait::async_trait;

#[async_trait]
pub trait Device {
    fn get_name(&self) -> &str;
}