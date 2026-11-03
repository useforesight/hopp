import QtQuick
import QtQuick.Layouts
import qs.Ui as Ui
import qs.Commons

// A room: who's in it, and Join on hover.
Ui.BorderSurface {
    id: root
    required property var room
    signal joinRequested(string id)
    implicitHeight: Style.space(40)
    height: implicitHeight
    radius: Style.cornerRadius
    color: hover.hovered ? Style.hoverFillFor(Color.popups.text, Color.accent) : "transparent"
    borderSpec: hover.hovered ? Border.controlSpec("hover-cursor", Color.popups.text, Color.accent) : Border.none()
    HoverHandler {
        id: hover
    }
    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Style.spacing.lg
        anchors.rightMargin: Style.spacing.sm
        spacing: Style.spacing.rowGap
        Text {
            text: "#"
            color: Qt.darker(Color.popups.text, 1.4)
            font.family: Style.font.family
            font.pixelSize: Style.font.title
            Layout.preferredWidth: Style.space(28)
            horizontalAlignment: Text.AlignHCenter
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: Style.spacing.xxs
            Text {
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: root.room.name
                elide: Text.ElideRight
                color: Color.popups.text
                font.family: Style.font.family
                font.pixelSize: Style.font.body
            }
            Text {
                Layout.fillWidth: true
                text: root.room.here ? "You're here" : root.room.members.length ? root.room.members.join(", ") : "Empty"
                elide: Text.ElideRight
                color: Qt.darker(Color.popups.text, 1.5)
                font.family: Style.font.family
                font.pixelSize: Style.font.caption
            }
        }
        Row {
            spacing: Style.spacing.xxs
            Repeater {
                model: root.room.members.slice(0, 3)
                delegate: Avatar {
                    required property var modelData
                    name: modelData
                    size: Style.space(18)
                }
            }
        }
        HoppButton {
            visible: hover.hovered && !root.room.here
            text: "Join"
            onClicked: root.joinRequested(root.room.id)
        }
    }
}
