use cpp_core::{Ptr, StaticUpcast};
use qt_core::{QBox, QObject, QPtr, SlotNoArgs, qs, slot};
use qt_gui::QIcon;
use qt_widgets::{QAction, QApplication, QMenu, QSystemTrayIcon, QWidget};
use std::rc::Rc;
use tokio::sync::RwLock;
use std::{sync::Arc};
use crate::manager::{Manager};

struct Form {
    widget: QBox<QWidget>,
    _tray_menu: QBox<QMenu>,
    device_menu: QBox<QMenu>,
    tray_icon: QBox<QSystemTrayIcon>,
    test_action: QPtr<QAction>,
    quit_action: QPtr<QAction>,
    runtime: tokio::runtime::Runtime,
    manager: Arc<RwLock<Manager>>
}

impl StaticUpcast<QObject> for Form {
    unsafe fn static_upcast(ptr: Ptr<Self>) -> Ptr<QObject> {
        ptr.widget.as_ptr().static_upcast()
    }
}

impl Form {
    fn new(runtime: tokio::runtime::Runtime, manager: Arc<RwLock<Manager>>) -> Rc<Form> {
        unsafe {
            let widget = QWidget::new_0a();

            let tray_menu = QMenu::new();

            let device_menu = QMenu::new();
            device_menu.set_title(&qs("Devices"));
            tray_menu.add_menu_q_menu(&device_menu);
            tray_menu.add_separator();

            let test_action = tray_menu.add_action_q_string(&qs("Test Action"));
            tray_menu.add_separator();
            let quit_action = tray_menu.add_action_q_string(&qs("Quit"));
            

            let icon = QIcon::from_q_string(&qs(":/trayicon.png"));
            let tray_icon = QSystemTrayIcon::new();
            tray_icon.set_icon(&icon);
            tray_icon.set_context_menu(&tray_menu);
            tray_icon.set_tool_tip(&qs("Test Menu"));
            tray_icon.show();

            let this = Rc::new(Self {
                widget,
                _tray_menu: tray_menu,
                device_menu,
                tray_icon,
                test_action,
                quit_action,
                runtime,
                manager
            });
            this.init();
            this
        }
    }

    unsafe fn init(self: &Rc<Self>) {
        self.test_action
            .triggered()
            .connect(&self.slot_on_test_action_triggered());
        self.quit_action
            .triggered()
            .connect(&self.slot_on_quit_action_triggered());
        self.device_menu
            .about_to_show()
            .connect(&self.slot_on_device_menu_about_to_show());
    }

    #[slot(SlotNoArgs)]
    unsafe fn on_device_menu_about_to_show(self: &Rc<Self>) {
        self.device_menu.clear();        
        let names = self.runtime.block_on(async {
            let mgr = self.manager.as_ref().read().await;
            mgr.get_names().await
        });

        for name in names {
            self.device_menu.add_action_q_string(&qs(name));
        }        
    }

    #[slot(SlotNoArgs)]
    unsafe fn on_test_action_triggered(self: &Rc<Self>) {
        self.tray_icon.show_message_2_q_string(&qs("Amazing thing"), &qs("A new thing was thung"));
    }

    #[slot(SlotNoArgs)]
    unsafe fn on_quit_action_triggered(self: &Rc<Self>) {
        self.widget.close();
    }
}

pub fn run_qt(runtime: tokio::runtime::Runtime, manager: Arc<RwLock<Manager>>) {
    QApplication::init(|_| unsafe {
        qt_core::q_init_resource!("resources");
        let _form = Form::new(runtime, manager);
        QApplication::exec()
    })
}