import QtQuick
import QtQuick.Layouts
import qs.Ui as Ui
import qs.Commons

// A teammate: avatar with presence, name, where they are, and Pair on hover.
Ui.BorderSurface {
    id: root
    required property var person
    property bool inCall: false
    property bool busy: false
    property bool current: false
    signal pairRequested(string id)
    implicitHeight: Style.space(44)
    height: implicitHeight
    radius: Style.cornerRadius
    color: hover.hovered || current ? Style.hoverFillFor(Color.popups.text, Color.accent) : "transparent"
    borderSpec: hover.hovered || current ? Border.controlSpec("hover-cursor", Color.popups.text, Color.accent) : Border.none()
    HoverHandler {
        id: hover
    }
    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Style.spacing.lg
        anchors.rightMargin: Style.spacing.sm
        spacing: Style.spacing.rowGap
        Avatar {
            name: root.person.name
            online: root.person.online
            showPresence: true
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: Style.spacing.xxs
            Text {
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: root.person.name
                elide: Text.ElideRight
                color: Color.popups.text
                font.family: Style.font.family
                font.pixelSize: Style.font.body
            }
            Text {
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: root.person.room ? "In " + root.person.room : root.person.with ? "In a call with " + root.person.with : root.person.online ? "Available" : "Away"
                elide: Text.ElideRight
                color: Qt.darker(Color.popups.text, 1.5)
                font.family: Style.font.family
                font.pixelSize: Style.font.caption
            }
        }
        HoppButton {
            visible: hover.hovered || root.current
            text: root.inCall ? "Invite" : "Pair"
            primary: true
            enabled: !root.busy
            hint: (root.inCall ? "Invite " : "Pair with ") + root.person.name
            onClicked: root.pairRequested(root.person.id)
        }
    }
}
