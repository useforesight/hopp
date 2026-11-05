import QtQuick
import qs.Commons

// The Hopp mark: a rounded square with a lowercase h, drawn in the bar's colour.
Item {
    id: root
    property color foreground: Color.foreground
    property bool alert: false
    // Someone tried to call: a steady dot until you call back or dismiss it.
    property bool missed: false
    implicitWidth: Style.font.display
    implicitHeight: implicitWidth
    Rectangle {
        anchors.centerIn: parent
        width: parent.width * 0.68
        height: width
        radius: width * 0.28
        color: "transparent"
        border.color: root.foreground
        border.width: Math.max(1, parent.width * 0.075)
        Text {
            anchors.centerIn: parent
            anchors.verticalCenterOffset: -parent.height * 0.02
            text: "h"
            color: root.foreground
            font.family: Style.font.family
            font.pixelSize: parent.height * 0.66
            font.weight: Font.Bold
        }
    }
    Rectangle {
        visible: root.missed && !root.alert
        width: parent.width * 0.3
        height: width
        radius: width / 2
        anchors.right: parent.right
        anchors.top: parent.top
        color: Color.urgent
    }
    Rectangle {
        visible: root.alert
        width: parent.width * 0.3
        height: width
        radius: width / 2
        anchors.right: parent.right
        anchors.top: parent.top
        color: Color.accent
        SequentialAnimation on opacity {
            running: root.alert
            loops: Animation.Infinite
            NumberAnimation { to: 0.35; duration: 600 }
            NumberAnimation { to: 1; duration: 600 }
        }
    }
}
