import QtQuick
import QtQuick.Layouts
import qs.Ui as Ui
import qs.Commons

// The way to your transcripts, as a row like the teammates and rooms below it: an icon,
// what it is, what's there, and Open on hover. Clicking anywhere opens the captures
// window.
Ui.BorderSurface {
    id: root
    // The live capture's line count, or -1 when nothing is being captured.
    property int liveLines: -1
    signal openRequested
    implicitHeight: Style.space(44)
    height: implicitHeight
    radius: Style.cornerRadius
    color: hover.hovered ? Style.hoverFillFor(Color.popups.text, Color.accent) : "transparent"
    borderSpec: hover.hovered ? Border.controlSpec("hover-cursor", Color.popups.text, Color.accent) : Border.none()
    HoverHandler {
        id: hover
        cursorShape: Qt.PointingHandCursor
    }
    TapHandler {
        onTapped: root.openRequested()
    }
    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Style.spacing.lg
        anchors.rightMargin: Style.spacing.lg
        spacing: Style.spacing.rowGap
        Rectangle {
            Layout.preferredWidth: Style.space(28)
            Layout.preferredHeight: Style.space(28)
            radius: Style.cornerRadius
            color: Util.alpha(Color.accent, 0.14)
            Text {
                anchors.centerIn: parent
                // Material Design "text-box-outline".
                text: String.fromCodePoint(0xF09ED)
                color: Color.accent
                font.family: Style.font.family
                // Nerd Font draws these glyphs narrow; size them to the tile instead.
                font.pixelSize: Style.space(18)
            }
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: Style.spacing.xxs
            Text {
                Layout.fillWidth: true
                text: "Transcripts"
                color: Color.popups.text
                font.family: Style.font.family
                font.pixelSize: Style.font.body
            }
            Text {
                Layout.fillWidth: true
                text: root.liveLines >= 0 ? "This call is live · " + root.liveLines + (root.liveLines === 1 ? " line" : " lines") : "Every call captured on this computer"
                elide: Text.ElideRight
                color: root.liveLines >= 0 ? Color.accent : Qt.darker(Color.popups.text, 1.5)
                font.family: Style.font.family
                font.pixelSize: Style.font.caption
            }
        }
        Text {
            text: hover.hovered ? "Open" : "›"
            color: hover.hovered ? Color.accent : Qt.darker(Color.popups.text, 1.6)
            font.family: Style.font.family
            font.pixelSize: hover.hovered ? Style.font.caption : Style.font.title
        }
    }
}
