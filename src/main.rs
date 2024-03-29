// If the OS is Windows, it's a release build and the gui is enabled, make the app run without a console window
//#![cfg_attr(target_os = "windows", cfg_attr(not(debug_assertions), cfg_attr(feature = "gui", windows_subsystem = "windows")))]

use tokio::runtime::Builder;
use tokio::sync::RwLock;
use std::sync::{Arc};
mod tao;

mod devices;
mod util;

mod server;
mod manager;
mod protocol;


fn main() {
    //console_subscriber::init();

    /* Create device manager instance */
    let manager = Arc::new(RwLock::new(manager::Manager::new()));

    let runtime = Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    /* Spawn device manager task */
    let task_manager = manager.clone();
    let _ = runtime.spawn(async move {         
        manager::run_manager(task_manager).await;
    });

    /* Spawn websocket server */   
    let runtime_manager = manager.clone();
    let _ = runtime.spawn(async move {         
        server::run_server(runtime_manager).await;
    });
    

    #[cfg(feature = "gui")]
    {
        /* Run tao Main Thread */
        
        /* By giving the tao thread access to the tokio threadpool runtime and a reference to the manager,
        it's possible to access the internal device manager state from the tao side by using
        runtime.block_on() to execute async-code on the tao main thread.
        
        This will ofcourse block the tao thread for the duration of the async calls, but it shouldn't
        be a big issue since most calls will be things that should be guaranteed to return quickly.
        */
        let tao_manager = manager.clone();
        tao::run(runtime, tao_manager);
    }

    #[cfg(not(feature = "gui"))]
    {
        /* Without a gui, just wait for ctrl + c */
        runtime.block_on(async move {
            tokio::signal::ctrl_c().await.unwrap();
        });            
    }



}
