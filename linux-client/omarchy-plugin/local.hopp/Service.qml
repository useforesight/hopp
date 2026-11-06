import QtQuick
import Quickshell.Io

// Long-lived connection to the Hopp client through backend.py.
Item {
    id: service
    property var shell: null
    property bool online: false
    property bool stopped: false
    property var state: ({})
    property string message: ""
    property real now: Date.now() / 1000
    readonly property bool signedIn: online && state.signed_in === true
    // available, away or dnd; empty on servers without statuses.
    readonly property string status: signedIn && state.user && state.user.status ? state.user.status : ""
    readonly property var call: online && state.call ? state.call : null
    readonly property var ringing: online && state.ringing ? state.ringing : null
    readonly property var dialing: online && state.dialing ? state.dialing : null
    readonly property var missed: online && state.missed ? state.missed : []
    readonly property var teammates: online && state.teammates ? state.teammates : []
    readonly property var rooms: online && state.rooms ? state.rooms : []
    readonly property string elapsed: {
        if (!call || !call.started)
            return "";
        const total = Math.max(0, Math.floor(service.now - call.started));
        const h = Math.floor(total / 3600), m = Math.floor(total / 60) % 60, s = total % 60;
        const pad = n => (n < 10 ? "0" : "") + n;
        return h > 0 ? h + ":" + pad(m) + ":" + pad(s) : pad(m) + ":" + pad(s);
    }

    function send(action, value) {
        service.message = "";
        if (!bridge.running) {
            service.restart();
            return;
        }
        bridge.write(JSON.stringify({ action: action, value: value || "" }) + "\n");
    }
    function restart() {
        bridge.exec(["python", decodeURIComponent(Qt.resolvedUrl("backend.py").toString().replace(/^file:\/\//, ""))]);
    }
    function receive(data) {
        if (data.type === "state") {
            service.state = data.state;
            service.online = true;
            service.stopped = false;
        } else if (data.type === "offline") {
            service.online = false;
            service.stopped = data.stopped === true;
        } else if (data.type === "error") {
            service.message = data.message;
        }
    }

    // The call timer only needs to tick while a call is up.
    Timer {
        interval: 1000
        repeat: true
        running: service.call !== null
        triggeredOnStart: true
        onTriggered: service.now = Date.now() / 1000
    }
    IpcHandler {
        target: "hopp"
        function status(): string {
            return JSON.stringify({ online: service.online, signedIn: service.signedIn, inCall: service.call !== null });
        }
        function reconnect(): void {
            service.restart();
        }
    }
    Process {
        id: bridge
        stdinEnabled: true
        stdout: SplitParser {
            onRead: data => {
                try {
                    service.receive(JSON.parse(data));
                } catch (error) {
                    service.message = "Could not read Hopp state";
                }
            }
        }
        stderr: StdioCollector {
            onStreamFinished: if (text.trim()) service.message = text.trim().slice(-300)
        }
        onRunningChanged: if (!running) service.online = false
    }
    Component.onCompleted: service.restart()
}
