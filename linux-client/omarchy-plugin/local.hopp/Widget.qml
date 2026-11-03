pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import Quickshell
import qs.Commons
import qs.Ui as Ui

// Hopp in the Omarchy bar: the mark when idle, a live call pill during a call, and a
// keyboard-driven panel for teammates, rooms and call controls.
Ui.Panel {
    id: app
    moduleName: "local.hopp"
    manageIpc: false
    readonly property var service: bar?.shell?.serviceFor("local.hopp") ?? pendingService
    readonly property bool vertical: bar ? bar.vertical === true : false
    readonly property var call: service.call
    readonly property bool inCall: call !== null && call !== undefined
    readonly property bool capturing: inCall && call.capture !== null && call.capture !== undefined
    readonly property var capturers: inCall ? (call.participants || []).filter(person => person.capturing).map(person => person.name) : []
    readonly property bool othersCapturing: capturers.length > 0
    readonly property string captureText: {
        if (capturing) {
            const capture = call.capture;
            const state = capture.state === "downloading" ? "downloading " + capture.model + ", " + (capture.percent || 0) + "%"
                : capture.state === "failed" ? "transcription failed: " + (capture.message || "unknown error")
                : capture.state === "idle" ? capture.model + " unloaded until someone speaks"
                : capture.state === "ready" ? (capture.behind_ms >= 60000 ? Math.round(capture.behind_ms / 1000) + " s behind" : capture.lines + " lines so far")
                : "loading " + capture.model;
            return "Capturing to this computer · " + state;
        }
        return othersCapturing ? capturers.join(", ") + (capturers.length > 1 ? " are" : " is") + " capturing this call" : "";
    }
    property int cursor: 0
    readonly property var people: (service.teammates || [])
        .filter(person => person.name.toLowerCase().includes(search.text.trim().toLowerCase()))
        .sort((a, b) => Number(b.online) - Number(a.online) || a.name.localeCompare(b.name))

    QtObject {
        id: pendingService
        property bool online: false
        property bool stopped: false
        property bool signedIn: false
        property var state: ({})
        property var call: null
        property var ringing: null
        property var dialing: null
        property var teammates: []
        property var rooms: []
        property string message: ""
        property string elapsed: ""
        function send(action, value) {
        }
    }

    function send(action, value) {
        if (service)
            service.send(action, value);
    }
    function pairCursor() {
        const person = app.people[app.cursor];
        if (person && person.online)
            app.send("call", person.id);
    }

    implicitWidth: barRow.implicitWidth
    implicitHeight: button.implicitHeight
    visible: true
    onOpenedChanged: if (opened) {
        app.cursor = 0;
        search.text = "";
    }

    Row {
        id: barRow
        anchors.verticalCenter: parent.verticalCenter
        spacing: 0
        Ui.BarIconButton {
            id: button
            bar: app.bar
            tooltipText: app.service.stopped ? "Hopp · not running" : !app.service.online ? "Hopp · starting" : app.service.ringing ? app.service.ringing.name + " is calling" : app.inCall ? "Hopp · " + app.call.room : "Hopp"
            iconComponent: HoppMark {
                foreground: button.foreground
                alert: app.service.ringing !== null && app.service.ringing !== undefined
            }
            onPressed: app.toggle()
        }
        // In a call: who's here, how long, and whether you can be heard.
        Item {
            id: pill
            visible: app.inCall && !app.vertical
            width: visible ? pillContent.implicitWidth + Style.spacing.lg : 0
            height: button.height
            Row {
                id: pillContent
                anchors.verticalCenter: parent.verticalCenter
                spacing: Style.spacing.md
                Rectangle {
                    visible: app.inCall && app.call.sharing
                    anchors.verticalCenter: parent.verticalCenter
                    width: Style.space(7)
                    height: width
                    radius: width / 2
                    color: Color.urgent
                }
                // Capture: solid while you capture, faint while someone else does.
                Row {
                    visible: app.capturing || app.othersCapturing
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 2
                    opacity: app.capturing ? 1 : 0.45
                    Repeater {
                        model: [0.5, 1, 0.7]
                        delegate: Rectangle {
                            required property var modelData
                            anchors.verticalCenter: parent.verticalCenter
                            width: 2
                            height: Style.space(10) * modelData
                            radius: 1
                            color: Color.accent
                        }
                    }
                }
                Row {
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: -Style.space(4)
                    Repeater {
                        model: app.inCall ? app.call.participants.slice(0, 3) : []
                        delegate: Avatar {
                            required property var modelData
                            name: modelData.name
                            size: Style.space(16)
                            border.width: 1
                            border.color: Color.bar.background
                        }
                    }
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: app.service.elapsed
                    color: app.barForeground
                    font.family: Style.font.family
                    font.pixelSize: Style.font.bodySmall
                    font.features: { "tnum": 1 }
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: app.inCall && app.call.microphone ? "󰍬" : "󰍭"
                    color: app.inCall && app.call.microphone ? app.barForeground : Color.urgent
                    font.family: Style.font.family
                    font.pixelSize: Style.font.body
                }
            }
            MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: app.toggle()
            }
        }
    }

    Ui.KeyboardPanel {
        id: panel
        anchorItem: barRow
        owner: app
        bar: app.bar
        open: app.opened
        focusTarget: app.service.signedIn ? search : focusScope
        contentWidth: panel.fittedContentWidth(Style.space(384))
        contentHeight: panel.fittedContentHeight(content.implicitHeight, Style.space(700))
        Item {
            id: focusScope
            anchors.fill: parent
            focus: true
            Keys.onEscapePressed: app.close()
            ColumnLayout {
                id: content
                anchors.fill: parent
                spacing: Style.spacing.panelGap

                Ui.PanelHero {
                    Layout.fillWidth: true
                    title: "Hopp"
                    meta: app.service.stopped ? "Not running" : !app.service.online ? "Starting Hopp…" : !app.service.signedIn ? "Not signed in" : app.inCall ? (app.call.connected ? "In a call · " + app.service.elapsed : "Connecting…") : app.service.teammates.filter(p => p.online).length + " available"
                    iconComponent: HoppMark {
                        foreground: Color.popups.text
                    }
                    trailingControl: Row {
                        spacing: Style.spacing.xs
                        Ui.PanelActionButton {
                            iconText: "󰖯"
                            tooltipText: "Open the Hopp window"
                            focusable: true
                            foreground: Color.popups.text
                            onClicked: {
                                app.send("open");
                                app.close();
                            }
                        }
                        Ui.PanelActionButton {
                            iconText: "󰒓"
                            tooltipText: "Settings"
                            focusable: true
                            foreground: Color.popups.text
                            onClicked: {
                                app.send("settings");
                                app.close();
                            }
                        }
                        Ui.PanelActionButton {
                            visible: app.service.online
                            iconText: "󰐥"
                            tooltipText: app.inCall ? "Leave the call and quit Hopp" : "Quit Hopp"
                            focusable: true
                            foreground: Color.popups.text
                            onClicked: {
                                app.send("quit");
                                app.close();
                            }
                        }
                    }
                }

                // Not running or not signed in: one clear way forward.
                ColumnLayout {
                    visible: !app.service.signedIn
                    Layout.fillWidth: true
                    spacing: Style.spacing.lg
                    Text {
                        Layout.fillWidth: true
                        text: app.service.stopped ? "Hopp isn't running." : !app.service.online ? "Hopp is starting in the background. It'll appear here in a moment." : "Sign in to see your team, share screens and pair."
                        color: Qt.darker(Color.popups.text, 1.3)
                        font.family: Style.font.family
                        font.pixelSize: Style.font.body
                        wrapMode: Text.WordWrap
                    }
                    HoppButton {
                        visible: app.service.stopped
                        text: "Start Hopp"
                        primary: true
                        Layout.fillWidth: true
                        onClicked: app.send("open")
                    }
                    HoppButton {
                        visible: app.service.online
                        text: "Sign in"
                        primary: true
                        Layout.fillWidth: true
                        onClicked: {
                            app.send("open");
                            app.close();
                        }
                    }
                }

                // Someone is calling, or you're calling someone.
                Ui.BorderSurface {
                    visible: (app.service.ringing || app.service.dialing) ? true : false
                    Layout.fillWidth: true
                    implicitHeight: ringingRow.implicitHeight + Style.spacing.xxl * 2
                    radius: Style.cornerRadius
                    color: Util.alpha(Color.accent, 0.08)
                    borderSpec: Border.controlSpec("selected", Color.popups.text, Color.accent)
                    RowLayout {
                        id: ringingRow
                        anchors.fill: parent
                        anchors.margins: Style.spacing.xxl
                        spacing: Style.spacing.rowGap
                        Avatar {
                            name: app.service.ringing ? app.service.ringing.name : app.service.dialing ? app.service.dialing.name : ""
                            size: Style.space(32)
                        }
                        Text {
                            Layout.fillWidth: true
                            textFormat: Text.PlainText
                            text: app.service.ringing ? (app.service.ringing.kind === "invite" ? app.service.ringing.name + " invited you" : app.service.ringing.name + " wants to pair") : app.service.dialing ? "Calling " + app.service.dialing.name + "…" : ""
                            elide: Text.ElideRight
                            color: Color.popups.text
                            font.family: Style.font.family
                            font.pixelSize: Style.font.body
                            font.weight: Font.DemiBold
                        }
                        HoppButton {
                            visible: app.service.ringing ? true : false
                            text: "Accept"
                            primary: true
                            onClicked: app.send("accept")
                        }
                        HoppButton {
                            text: app.service.ringing ? "Decline" : "Cancel"
                            onClicked: app.send("decline")
                        }
                    }
                }

                // The current call.
                Ui.BorderSurface {
                    visible: app.inCall
                    Layout.fillWidth: true
                    implicitHeight: callColumn.implicitHeight + Style.spacing.xxl * 2
                    radius: Style.cornerRadius
                    color: Util.alpha(Color.accent, 0.05)
                    borderSpec: Border.controlSpec("normal", Color.popups.text, Color.accent)
                    ColumnLayout {
                        id: callColumn
                        anchors.fill: parent
                        anchors.margins: Style.spacing.xxl
                        spacing: Style.spacing.lg
                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Style.spacing.md
                            Rectangle {
                                width: Style.space(7)
                                height: width
                                radius: width / 2
                                color: app.inCall && app.call.connected ? Color.accent : Color.muted
                            }
                            Text {
                                Layout.fillWidth: true
                                textFormat: Text.PlainText
                                text: !app.inCall ? "" : app.call.participants.length ? "Pairing with " + app.call.participants.map(p => p.name.split(" ")[0]).join(", ") : app.call.room
                                elide: Text.ElideRight
                                color: Color.popups.text
                                font.family: Style.font.family
                                font.pixelSize: Style.font.body
                                font.weight: Font.DemiBold
                            }
                            Text {
                                text: app.service.elapsed
                                color: Qt.darker(Color.popups.text, 1.4)
                                font.family: Style.font.family
                                font.pixelSize: Style.font.caption
                            }
                        }
                        Text {
                            visible: app.inCall && (app.call.viewing || app.call.sharing || app.call.selecting) ? true : false
                            Layout.fillWidth: true
                            text: !app.inCall ? "" : app.call.selecting ? "Pick a screen or window in the system chooser." : app.call.sharing ? "You're sharing your screen." : app.call.viewing ? "Viewing " + app.call.viewing + "'s screen" + (app.call.controlling ? " · you have control" : "") : ""
                            color: Qt.darker(Color.popups.text, 1.3)
                            font.family: Style.font.family
                            font.pixelSize: Style.font.caption
                            wrapMode: Text.WordWrap
                        }
                        Text {
                            visible: app.capturing || app.othersCapturing
                            Layout.fillWidth: true
                            text: app.captureText
                            color: Qt.darker(Color.popups.text, 1.3)
                            font.family: Style.font.family
                            font.pixelSize: Style.font.caption
                            wrapMode: Text.WordWrap
                        }
                        Flow {
                            Layout.fillWidth: true
                            spacing: Style.spacing.md
                            HoppButton {
                                text: app.inCall && app.call.microphone ? "Mute" : "Unmute"
                                selected: app.inCall && !app.call.microphone
                                onClicked: app.send("microphone")
                            }
                            HoppButton {
                                text: app.inCall && app.call.camera ? "Camera off" : "Camera on"
                                onClicked: app.send("camera")
                            }
                            HoppButton {
                                text: app.inCall && app.call.sharing ? "Stop sharing" : app.inCall && app.call.selecting ? "Cancel" : "Share screen"
                                primary: app.inCall && app.call.sharing
                                onClicked: app.send("share")
                            }
                            HoppButton {
                                text: app.capturing ? "Stop capture" : "Capture"
                                primary: app.capturing
                                hint: "Transcribe the call to this computer · Super Alt C"
                                onClicked: app.send("capture")
                            }
                            HoppButton {
                                visible: app.capturing
                                text: "Transcript"
                                hint: "The live transcript and past captures"
                                onClicked: {
                                    app.send("captures");
                                    app.close();
                                }
                            }
                            HoppButton {
                                text: "Faces"
                                hint: "Show the call window"
                                onClicked: app.send("faces")
                            }
                            HoppButton {
                                visible: app.inCall && app.call.viewing ? true : false
                                text: "Screen"
                                hint: "Show the shared screen"
                                onClicked: app.send("viewer")
                            }
                            HoppButton {
                                visible: app.inCall && app.call.viewing ? true : false
                                text: "Peek"
                                hint: "Whole screen · Super H"
                                onClicked: {
                                    app.send("peek");
                                    app.close();
                                }
                            }
                            HoppButton {
                                text: "Leave"
                                destructive: true
                                onClicked: app.send("leave")
                            }
                        }
                    }
                }

                Ui.TextField {
                    id: search
                    visible: app.service.signedIn
                    Layout.fillWidth: true
                    implicitHeight: Style.space(34)
                    placeholderText: "Search teammates"
                    font.family: Style.font.family
                    font.pixelSize: Style.font.body
                    selectByMouse: true
                    onTextChanged: app.cursor = 0
                    onAccepted: app.pairCursor()
                    Keys.onDownPressed: app.cursor = Math.min(app.cursor + 1, app.people.length - 1)
                    Keys.onUpPressed: app.cursor = Math.max(app.cursor - 1, 0)
                    Keys.onEscapePressed: app.close()
                }

                RowLayout {
                    visible: app.service.signedIn
                    Layout.fillWidth: true
                    Text {
                        text: "TEAMMATES"
                        color: Qt.darker(Color.popups.text, 1.4)
                        font.family: Style.font.family
                        font.pixelSize: Style.font.caption
                        font.weight: Font.DemiBold
                        font.letterSpacing: 1
                    }
                    Item {
                        Layout.fillWidth: true
                    }
                    Text {
                        text: app.people.filter(p => p.online).length + " available"
                        color: Qt.darker(Color.popups.text, 1.4)
                        font.family: Style.font.family
                        font.pixelSize: Style.font.caption
                    }
                }
                ListView {
                    visible: app.service.signedIn
                    Layout.fillWidth: true
                    Layout.preferredHeight: Math.min(Style.space(264), Math.max(Style.space(44), contentHeight))
                    model: app.people
                    clip: true
                    spacing: 2
                    currentIndex: app.cursor
                    Controls.ScrollBar.vertical: Controls.ScrollBar {}
                    delegate: PersonRow {
                        required property var modelData
                        required property int index
                        width: ListView.view.width
                        person: modelData
                        current: index === app.cursor
                        inCall: app.inCall
                        busy: app.service.dialing ? true : false
                        onPairRequested: id => app.send("call", id)
                    }
                    Text {
                        anchors.centerIn: parent
                        visible: app.people.length === 0
                        text: search.text ? "No teammates match" : "Invite your team to Hopp to pair here"
                        color: Qt.darker(Color.popups.text, 1.4)
                        font.family: Style.font.family
                        font.pixelSize: Style.font.body
                    }
                }

                Text {
                    visible: app.service.signedIn && app.service.rooms.length > 0
                    text: "ROOMS"
                    color: Qt.darker(Color.popups.text, 1.4)
                    font.family: Style.font.family
                    font.pixelSize: Style.font.caption
                    font.weight: Font.DemiBold
                    font.letterSpacing: 1
                }
                Column {
                    visible: app.service.signedIn
                    Layout.fillWidth: true
                    spacing: 2
                    Repeater {
                        model: app.service.rooms
                        delegate: RoomRow {
                            required property var modelData
                            width: parent ? parent.width : 0
                            room: modelData
                            onJoinRequested: id => app.send("join", id)
                        }
                    }
                }

                Text {
                    visible: app.service.message.length > 0
                    Layout.fillWidth: true
                    text: app.service.message
                    textFormat: Text.PlainText
                    color: Color.urgent
                    font.family: Style.font.family
                    font.pixelSize: Style.font.bodySmall
                    wrapMode: Text.WordWrap
                    maximumLineCount: 3
                    elide: Text.ElideRight
                }
                RowLayout {
                    Layout.fillWidth: true
                    Text {
                        visible: app.service.signedIn
                        text: "↑↓ choose · ⏎ pair · Super H peek"
                        color: Qt.darker(Color.popups.text, 1.6)
                        font.family: Style.font.family
                        font.pixelSize: Style.font.caption
                    }
                    Item {
                        Layout.fillWidth: true
                    }
                    Text {
                        text: "Esc to close"
                        color: Qt.darker(Color.popups.text, 1.6)
                        font.family: Style.font.family
                        font.pixelSize: Style.font.caption
                    }
                }
            }
        }
    }
}
