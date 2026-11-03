import QtQuick
import qs.Commons

// Initials in a circle, coloured from the theme accent rotated by name, matching
// the colours the client's windows use for the same person.
Rectangle {
    id: root
    property string name: ""
    property bool online: false
    property bool showPresence: false
    property real size: Style.space(28)
    readonly property var offsets: [0, 0.1, 0.2, 0.62, 0.72, 0.82]
    readonly property color tone: {
        let hash = 0;
        for (let i = 0; i < name.length; i++)
            hash = (hash * 31 + name.charCodeAt(i)) >>> 0;
        let hue = (Color.accent.hslHue + offsets[hash % offsets.length]) % 1;
        if (hue < 0.04 || hue > 0.93)
            hue = (hue + 0.12) % 1;
        return Qt.hsla(hue, Color.accent.hslSaturation, Color.accent.hslLightness, 1);
    }
    implicitWidth: size
    implicitHeight: size
    radius: size / 2
    color: Qt.rgba(tone.r, tone.g, tone.b, 0.22)
    Text {
        anchors.centerIn: parent
        text: root.name.split(/\s+/).filter(w => w.length).slice(0, 2).map(w => w[0]).join("").toUpperCase()
        color: root.tone
        font.family: Style.font.family
        font.pixelSize: Math.max(8, root.size * 0.38)
        font.weight: Font.DemiBold
    }
    Rectangle {
        visible: root.showPresence
        width: root.size * 0.34
        height: width
        radius: width / 2
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.margins: -1
        color: root.online ? Color.accent : Qt.darker(Color.popups.text, 1.8)
        border.width: 2
        border.color: Color.popups.background
    }
}
