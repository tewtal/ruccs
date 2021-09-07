
use tokio::runtime::Builder;
use tokio::sync::RwLock;
use std::sync::Arc;

mod server;
mod qml;
mod manager;
mod devices;

fn main() {

    let manager = Arc::new(RwLock::new(manager::Manager::new()));
    // let runtime_sender = manager.tx.clone();
    // let qt_sender = manager.tx.clone();

    /* Create tokio runtime */
    let runtime = Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();

    /* Spawn manager task */
    let task_manager = manager.clone();
    let _ = runtime.spawn(async move {         
        manager::run_manager(task_manager).await;
    });
    
    /* Spawn websocket server */   
    let runtime_manager = manager.clone();
    let _ = runtime.spawn(async move {         
        server::run_server(runtime_manager).await;
    });

    let qt_manager = manager;
    /* Run QT Main Thread */
    qml::run_qml_ui(runtime, qt_manager);
}
