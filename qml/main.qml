import QtQuick 2.0
import QtQuick.Window 2.0
import QtQuick.Controls 2.0
import Qt.labs.platform 1.1
import QtQml 2.2
import Wrapper 1.0

ApplicationWindow {
    visible: false
    title: "RuCCS"

    SystemTrayIcon {
        visible: true
        icon.source: "qrc:/images/tray-icon.png"
        menu: Menu {
            id: contextMenu

            Menu {
                title: "Devices"
                id: deviceMenu                        

                Instantiator {
                    id: deviceListInstantiator

                    function parseDeviceList(deviceList) {
                        return JSON.parse(deviceList)
                    }

                    model: []
                    delegate: MenuItem {
                        text: `${modelData.name} (${modelData.port})`
                    }

                    onObjectAdded: deviceMenu.insertItem(index, object)
                    onObjectRemoved: deviceMenu.removeItem(object)
                }

                onAboutToShow: deviceListInstantiator.model = deviceListInstantiator.parseDeviceList(_wrapper.compute_devicelist())
            }

            MenuSeparator {}
            
            MenuItem {
                text: qsTr("Quit")
                onTriggered: Qt.quit()
            }
        }                
    }
}