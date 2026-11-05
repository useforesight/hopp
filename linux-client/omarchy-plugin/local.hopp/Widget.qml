pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls as Controls
import QtQuick.Layouts
import Quickshell
import qs.Commons
import qs.Ui as Ui

// Hopp in the Omarchy bar: the mark when idle, a live call pill during a call, and a
// keyboard-driven panel: the call (what's happening, one row of controls), the way to
// your transcripts, teammates and rooms.
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
                : capture.state === "idle" ? capture.lines + " lines · " + capture.model + " unloaded until someone speaks"
                : capture.state === "ready" ? (capture.behind_ms >= 60000 ? Math.round(capture.behind_ms / 1000) + " s behind" : capture.lines + (capture.lines === 1 ? " line" : " lines"))
                : "loading " + capture.model;
            return "Capturing · " + state;
        }
        return othersCapturing ? capturers.join(", ") + (capturers.length > 1 ? " are" : " is") + " capturing this call" : "Not capturing";
    }
    // Who else is here, by first name.
    readonly property string withText: {
        if (!inCall)
            return "";
        if (!call.connected)
            return "Connecting…";
        const names = (call.participants || []).map(person => person.name.split(" ")[0]);
        if (names.length === 0)
            return "Waiting for others";
        return "with " + (names.length === 1 ? names[0] : names.slice(0, -1).join(", ") + " and " + names[names.length - 1]);
    }
    // Nerd Font icons (Material Design), by name.
    readonly property var glyph: ({
            microphone: String.fromCodePoint(0xF036C),
            microphoneOff: String.fromCodePoint(0xF036D),
            video: String.fromCodePoint(0xF0567),
            videoOff: String.fromCodePoint(0xF0568),
            share: String.fromCodePoint(0xF1483),
            faces: String.fromCodePoint(0xF0849),
            peek: String.fromCodePoint(0xF0293),
            hangUp: String.fromCodePoint(0xF03F5)
        })
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
        property var missed: []
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
            tooltipText: app.service.stopped ? "Hopp · not running" : !app.service.online ? "Hopp · starting" : app.service.ringing ? app.service.ringing.name + " is calling" : app.inCall ? "Hopp · " + app.call.room : (app.service.missed || []).length > 0 ? "Hopp · missed call from " + app.service.missed[0].name : "Hopp"
            iconComponent: HoppMark {
                foreground: button.foreground
                alert: app.service.ringing !== null && app.service.ringing !== undefined
                missed: (app.service.missed || []).length > 0
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
        contentHeight: panel.fittedContentHeight(content.implicitHeight, Style.space(720))
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
                    meta: app.service.stopped ? "Not running" : !app.service.online ? "Starting Hopp…" : !app.service.signedIn ? "Not signed in" : app.service.ringing ? "Incoming call" : app.inCall ? (app.call.connected ? "In a call · " + app.service.elapsed : "Connecting…") : app.service.teammates.filter(p => p.online).length + " available"
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
                            hoverColor: Color.urgent
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

                // Someone is calling, or you're calling someone: answering comes first.
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
                            text: app.service.ringing ? "Decline" : "Cancel"
                            onClicked: app.send("decline")
                        }
                        HoppButton {
                            visible: app.service.ringing ? true : false
                            text: "Accept"
                            primary: true
                            onClicked: app.send("accept")
                        }
                    }
                }

                // Calls nobody answered, until you call back or dismiss them.
                Repeater {
                    model: app.service.missed || []
                    delegate: Ui.BorderSurface {
                        required property var modelData
                        Layout.fillWidth: true
                        implicitHeight: missedRow.implicitHeight + Style.spacing.xxl * 2
                        radius: Style.cornerRadius
                        color: Util.alpha(Color.urgent, 0.08)
                        borderSpec: Border.controlSpec("selected", Color.popups.text, Color.urgent)
                        RowLayout {
                            id: missedRow
                            anchors.fill: parent
                            anchors.margins: Style.spacing.xxl
                            spacing: Style.spacing.rowGap
                            Avatar {
                                name: modelData.name
                                size: Style.space(32)
                            }
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 0
                                Text {
                                    Layout.fillWidth: true
                                    textFormat: Text.PlainText
                                    text: modelData.name
                                    elide: Text.ElideRight
                                    color: Color.popups.text
                                    font.family: Style.font.family
                                    font.pixelSize: Style.font.body
                                    font.weight: Font.DemiBold
                                }
                                Text {
                                    Layout.fillWidth: true
                                    textFormat: Text.PlainText
                                    text: modelData.label
                                    elide: Text.ElideRight
                                    color: Color.urgent
                                    font.family: Style.font.family
                                    font.pixelSize: Style.font.body
                                }
                            }
                            HoppButton {
                                text: "Dismiss"
                                onClicked: app.send("dismiss_missed", modelData.id)
                            }
                            HoppButton {
                                text: "Call back"
                                primary: true
                                enabled: !app.service.ringing && !app.service.dialing
                                onClicked: app.send("call", modelData.id)
                            }
                        }
                    }
                }

                // The current call: who and how long, what's happening, then the controls.
                Ui.BorderSurface {
                    visible: app.inCall
                    Layout.fillWidth: true
                    implicitHeight: callColumn.implicitHeight + Style.spacing.xxl * 2
                    radius: Style.cornerRadius
                    color: Util.alpha(Color.accent, 0.06)
                    borderSpec: Border.controlSpec("selected", Color.popups.text, Color.accent)
                    ColumnLayout {
                        id: callColumn
                        anchors.fill: parent
                        anchors.margins: Style.spacing.xxl
                        spacing: Style.spacing.lg

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Style.spacing.rowGap
                            Row {
                                visible: app.inCall && app.call.participants.length > 0
                                spacing: -Style.space(6)
                                Repeater {
                                    model: app.inCall ? app.call.participants.slice(0, 3) : []
                                    delegate: Avatar {
                                        required property var modelData
                                        name: modelData.name
                                        size: Style.space(26)
                                        border.width: 1
                                        border.color: Color.popups.background
                                    }
                                }
                            }
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: Style.spacing.xxs
                                Text {
                                    Layout.fillWidth: true
                                    textFormat: Text.PlainText
                                    text: app.inCall ? app.call.room : ""
                                    elide: Text.ElideRight
                                    color: Color.popups.text
                                    font.family: Style.font.family
                                    font.pixelSize: Style.font.body
                                    font.weight: Font.DemiBold
                                }
                                Text {
                                    Layout.fillWidth: true
                                    textFormat: Text.PlainText
                                    text: app.withText
                                    elide: Text.ElideRight
                                    color: Qt.darker(Color.popups.text, 1.5)
                                    font.family: Style.font.family
                                    font.pixelSize: Style.font.caption
                                }
                            }
                            Text {
                                text: app.service.elapsed
                                color: Qt.darker(Color.popups.text, 1.4)
                                font.family: Style.font.family
                                font.pixelSize: Style.font.caption
                                font.features: { "tnum": 1 }
                            }
                        }

                        // What's happening, one line each, with its action beside it.
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: Style.spacing.sm
                            StatusLine {
                                visible: app.inCall && app.call.selecting ? true : false
                                dot: Color.accent
                                text: "Pick a screen or window in the system chooser"
                                action: "Cancel"
                                onActivated: app.send("share")
                            }
                            StatusLine {
                                visible: app.inCall && app.call.sharing ? true : false
                                dot: Color.urgent
                                text: "You're sharing your screen"
                                action: "Draw"
                                actionHint: "Draw on your own screen · Super Alt D"
                                onActivated: {
                                    app.send("draw");
                                    app.close();
                                }
                            }
                            StatusLine {
                                visible: app.inCall && app.call.viewing ? true : false
                                dot: Color.accent
                                text: app.inCall && app.call.viewing ? "Viewing " + app.call.viewing.split(" ")[0] + "'s screen" + (app.call.controlling ? " · you have control" : "") : ""
                                action: "Show"
                                actionHint: "Show the shared screen"
                                onActivated: app.send("viewer")
                            }
                            StatusLine {
                                visible: app.inCall
                                live: app.capturing
                                dot: app.othersCapturing ? Color.accent : Color.muted
                                text: app.captureText
                                action: app.capturing ? "Transcript" : "Capture"
                                actionHint: app.capturing ? "The live transcript, with drawings and screens" : "Transcribe this call to this computer · Super Alt C"
                                onActivated: {
                                    if (app.capturing) {
                                        app.send("captures");
                                        app.close();
                                    } else {
                                        app.send("capture");
                                    }
                                }
                            }
                        }

                        // The controls, as in the call window.
                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Style.spacing.sm
                            CallControl {
                                iconText: app.inCall && app.call.microphone ? app.glyph.microphone : app.glyph.microphoneOff
                                tooltipText: app.inCall && app.call.microphone ? "Mute" : "Unmute"
                                warn: !(app.inCall && app.call.microphone)
                                onClicked: app.send("microphone")
                            }
                            CallControl {
                                iconText: app.inCall && app.call.camera ? app.glyph.video : app.glyph.videoOff
                                tooltipText: app.inCall && app.call.camera ? "Turn camera off" : "Turn camera on"
                                onClicked: app.send("camera")
                            }
                            CallControl {
                                iconText: app.glyph.share
                                tooltipText: app.inCall && app.call.sharing ? "Stop sharing" : app.inCall && app.call.selecting ? "Cancel sharing" : "Share your screen"
                                selected: app.inCall && (app.call.sharing || app.call.selecting) ? true : false
                                onClicked: app.send("share")
                            }
                            CallControl {
                                iconText: app.glyph.faces
                                tooltipText: "Show the call window"
                                onClicked: app.send("faces")
                            }
                            CallControl {
                                visible: app.inCall && app.call.viewing ? true : false
                                iconText: app.glyph.peek
                                tooltipText: "Peek at the whole screen · Super H"
                                onClicked: {
                                    app.send("peek");
                                    app.close();
                                }
                            }
                            Item {
                                Layout.fillWidth: true
                            }
                            Ui.Button {
                                iconText: app.glyph.hangUp
                                text: "Leave"
                                tooltipText: "Leave the call"
                                focusable: true
                                bordered: true
                                foreground: Color.urgent
                                accent: Color.urgent
                                fontFamily: Style.font.family
                                fontSize: Style.font.bodySmall
                                onClicked: app.send("leave")
                            }
                        }
                    }
                }

                // Transcripts: every captured call, in a call or not.
                TranscriptsRow {
                    visible: app.service.signedIn
                    Layout.fillWidth: true
                    liveLines: app.capturing ? (app.call.capture.lines || 0) : -1
                    onOpenRequested: {
                        app.send("captures");
                        app.close();
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
                    Ui.PanelSectionHeader {
                        text: "Teammates"
                        foreground: Color.popups.text
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
                    Layout.preferredHeight: Math.min(Style.space(232), Math.max(Style.space(44), contentHeight))
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

                Ui.PanelSectionHeader {
                    visible: app.service.signedIn && app.service.rooms.length > 0
                    text: "Rooms"
                    foreground: Color.popups.text
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
                        text: app.inCall ? "Super Alt C capture · Super H peek" : "↑↓ choose · ⏎ pair"
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

    // One thing happening in the call, with what you can do about it.
    component StatusLine: RowLayout {
        id: line
        property color dot: Color.muted
        property bool live: false
        property string text: ""
        property string action: ""
        property string actionHint: ""
        signal activated
        Layout.fillWidth: true
        spacing: Style.spacing.md
        Item {
            implicitWidth: Style.space(10)
            implicitHeight: Style.space(10)
            Rectangle {
                visible: !line.live
                anchors.centerIn: parent
                width: Style.space(7)
                height: width
                radius: width / 2
                color: line.dot
            }
            // Capturing: three bars that move, as on the bar's call pill.
            Row {
                visible: line.live
                anchors.centerIn: parent
                spacing: 2
                Repeater {
                    model: [0.5, 1, 0.7]
                    delegate: Rectangle {
                        id: bar
                        required property var modelData
                        required property int index
                        anchors.verticalCenter: parent.verticalCenter
                        width: 2
                        height: Style.space(10) * modelData
                        radius: 1
                        color: Color.accent
                        SequentialAnimation on height {
                            running: line.live && line.visible
                            loops: Animation.Infinite
                            PauseAnimation {
                                duration: bar.index * 160
                            }
                            NumberAnimation {
                                to: Style.space(10) * 0.35
                                duration: 420
                                easing.type: Easing.InOutSine
                            }
                            NumberAnimation {
                                to: Style.space(10) * bar.modelData
                                duration: 420
                                easing.type: Easing.InOutSine
                            }
                        }
                    }
                }
            }
        }
        Text {
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: line.text
            elide: Text.ElideRight
            color: Qt.darker(Color.popups.text, 1.2)
            font.family: Style.font.family
            font.pixelSize: Style.font.caption
        }
        HoppButton {
            visible: line.action.length > 0
            text: line.action
            hint: line.actionHint
            onClicked: line.activated()
        }
    }

    // A call control: an icon, bordered, lit when it's on.
    component CallControl: Ui.Button {
        property bool warn: false
        focusable: true
        bordered: true
        foreground: warn ? Color.urgent : Color.popups.text
        accent: warn ? Color.urgent : Color.accent
        iconSize: Style.font.icon
        horizontalPadding: Style.spacing.md
        verticalPadding: Style.spacing.sm
    }
}
