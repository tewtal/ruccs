use crate::devices::device::Device;
use async_trait::async_trait;

pub struct Lua {
    name: &'static str,
}

impl Lua {
    pub fn new() -> Self
    {
        Self {
            name: "Lua Device",
        }
    }
}

#[async_trait]
impl Device for Lua {
    fn get_name(&self) -> &str {
        self.name
    }
}