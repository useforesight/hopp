import QtQuick
import qs.Ui as Ui
import qs.Commons

Ui.Button {
    id: root
    property bool primary: false
    property bool destructive: false
    property string hint: ""
    focusable: true
    selected: primary
    tooltipText: hint
    foreground: destructive ? Color.urgent : Color.popups.text
    accent: destructive ? Color.urgent : Color.accent
    fontFamily: Style.font.family
    fontSize: Style.font.bodySmall
    horizontalPadding: Style.spacing.controlPaddingX
    verticalPadding: Style.spacing.controlPaddingY
    opacity: enabled ? 1 : 0.35
}
