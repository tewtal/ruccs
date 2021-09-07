use cstr::cstr;
use qmetaobject::{QObjectPinned, prelude::*};
use tokio::sync::RwLock;
use std::sync::Arc;
use crate::manager::{Manager};
use serde::Serialize;

#[derive(Serialize, Debug)]
struct DeviceItem {
    name: String,
    port: i32
}

#[derive(QObject, Default)]
struct Wrapper {
    base: qt_base_class!(trait QObject),    
    compute_devicelist: qt_method!(fn(&mut self) -> QString),
    runtime: Option<tokio::runtime::Runtime>,
    manager: Option<Arc<RwLock<Manager>>>
}

impl Wrapper {
    fn compute_devicelist(&mut self) -> QString {
        // Wrap everything into JSON because rust->qml interop isn't great and it's easier to pass it as a string
        // and deserialize inside the QML JS code
        let mut devices: Vec<DeviceItem> = Vec::new();

        // Accesses the device manager through 
        if let Some(runtime) = &self.runtime {            
            devices = runtime.block_on(async { 
                self.manager.as_ref().unwrap().read().await.get_names().await
            }).iter().map(|name| DeviceItem { name: name.to_string(), port: 0 }).collect()
        }

        serde_json::to_string(&devices).unwrap().into()
    }
}

pub fn run_qml_ui(runtime: tokio::runtime::Runtime, manager: Arc<RwLock<Manager>>) {
    qrc!(resources,
        "images/" as "images" {
            "tray-icon.png"
        },
        "qml/" as "qml" {
            "main.qml"
        }
    );
    resources();

    qml_register_type::<Wrapper>(cstr!("Wrapper"), 1, 0, cstr!("Wrapper"));
    let mut engine = QmlEngine::new();
    
    /* Create wrapper instance and hand it to QML */
    let wrapper = Wrapper { manager: Some(manager), runtime: Some(runtime), ..Default::default() };
    let wrapper_cell = std::cell::RefCell::new(wrapper);
    unsafe {
        engine.set_object_property("_wrapper".into(), QObjectPinned::new(&wrapper_cell));
    }
    
    engine.load_file("qrc:/qml/main.qml".into());
    engine.exec();
}