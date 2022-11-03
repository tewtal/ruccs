#![allow(dead_code)]

#[cfg(target_os = "linux")]
use tao::platform::linux::SystemTrayBuilderExtLinux;

#[cfg(target_os = "macos")]
use tao::platform::macos::{SystemTrayBuilderExtMacOS, SystemTrayExtMacOS};

use tao::{
  event::{Event},
  event_loop::{ControlFlow, EventLoop},
  menu::{ContextMenu, MenuItemAttributes, MenuType},
  system_tray::SystemTrayBuilder,
  TrayId,
};
use tao::{event::TrayEvent, system_tray::SystemTray};
use tokio::sync::RwLock;
use std::sync::{Arc};

use crate::manager::Manager;

const QUIT_ID: u16 = 1000;

fn create_system_tray(id: &str, event_loop: &EventLoop<()>, tray_menu: ContextMenu) -> SystemTray {
  let main_tray_id = TrayId::new(id);  
  let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/trayicon.png");
  let icon = load_icon(std::path::Path::new(path));

  #[cfg(target_os = "linux")]
  let system_tray = SystemTrayBuilder::new(icon.clone(), Some(tray_menu))
    .with_id(main_tray_id)
    .with_temp_icon_dir(std::path::Path::new("/tmp/ruccs"))
    .build(&event_loop)
    .unwrap();

  #[cfg(target_os = "windows")]
  let system_tray = SystemTrayBuilder::new(icon.clone(), Some(tray_menu))
    .with_id(main_tray_id)
    .with_tooltip("ruccs - console control server")
    .build(&event_loop)
    .unwrap();

  #[cfg(target_os = "macos")]
  let system_tray = SystemTrayBuilder::new(icon.clone(), Some(tray_menu))
    .with_id(main_tray_id)
    .with_tooltip("ruccs - console control server")
    .with_title("ruccs")
    .build(&event_loop)
    .unwrap();

    system_tray
}

fn build_menu(runtime: &tokio::runtime::Runtime, manager: &Arc<RwLock<Manager>>) -> ContextMenu {
  let mut menu = ContextMenu::new();
  let _ = menu.add_item(MenuItemAttributes::new("ruccs 0.1").with_id(tao::menu::MenuId::EMPTY));
  let _ = menu.add_native_item(tao::menu::MenuItem::Separator);

  let mut devices = ContextMenu::new();

  let names = runtime.block_on(async {
    let mgr = manager.as_ref().read().await;
    mgr.get_names()
  });

  for name in names {
    devices.add_item(MenuItemAttributes::new(&name).with_id(tao::menu::MenuId::EMPTY));
  }  

  let _ = menu.add_submenu("Devices", true, devices);
  let _ = menu.add_native_item(tao::menu::MenuItem::Separator);
  let _ = menu.add_item(MenuItemAttributes::new("Quit").with_id(tao::menu::MenuId { 0: QUIT_ID }));

  menu
}

pub fn run(runtime: tokio::runtime::Runtime, manager: Arc<RwLock<Manager>>) {
  
    let event_loop = EventLoop::new();

    let tray_menu = ContextMenu::new();
    let tray_id = "main-tray";
    let mut system_tray = Some(create_system_tray(tray_id, &event_loop, tray_menu));
  
    event_loop.run(move |event, _event_loop, control_flow| {

      *control_flow = ControlFlow::Wait;
  
      match event {
        Event::MenuEvent {
          menu_id,
          origin: MenuType::ContextMenu,
          ..
        } => {
          if menu_id.0 == QUIT_ID {
            // drop the system tray before exiting to remove the icon from system tray on Windows
            system_tray.take();
            *control_flow = ControlFlow::Exit;
          }
        }
        Event::TrayEvent {
          id: _,
          bounds: _,
          event,
          position: _,
          ..
        } => {
          if event == TrayEvent::RightClick {
            /* Every time the menu is opened with a right click we rebuild it */
            system_tray.as_mut().unwrap().set_menu(&build_menu(&runtime, &manager));
          }
        }
        _ => (),
    }
  });
}
  
//#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
//#[cfg(any(feature = "tray", all(target_os = "linux", feature = "ayatana")))]
fn load_icon(path: &std::path::Path) -> tao::system_tray::Icon {
    let (icon_rgba, icon_width, icon_height) = {
        let image = image::open(path)
        .expect("Failed to open icon path")
        .into_rgba8();
        let (width, height) = image.dimensions();
        let rgba = image.into_raw();
        (rgba, width, height)
    };
    tao::system_tray::Icon::from_rgba(icon_rgba, icon_width, icon_height)
        .expect("Failed to open icon")
}
