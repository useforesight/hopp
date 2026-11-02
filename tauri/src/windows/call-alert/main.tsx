import "@/services/sentry";
import "../../App.css";
import React, { useEffect, useState } from "react";
import ReactDOM from "react-dom/client";
import { emitTo, listen } from "@tauri-apps/api/event";
import { CallAlertCard } from "@/components/ui/call-alert-card";
import { initTheme } from "@/lib/theme";
import {
  CALL_ALERT_ACTION_EVENT,
  CALL_ALERT_READY_EVENT,
  CALL_ALERT_STATE_EVENT,
  CallAlertAction,
  CallAlertActionEvent,
  CallAlertEnvelope,
} from "@/lib/call-alert-types";

initTheme();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <CallAlertWindow />
  </React.StrictMode>,
);

// All state lives in the main window. This one stays free of the store and the socket
// service, which would otherwise open a second websocket connection from here.
function CallAlertWindow() {
  const [envelope, setEnvelope] = useState<CallAlertEnvelope | null>(null);

  useEffect(() => {
    const unlisten = listen<CallAlertEnvelope | null>(CALL_ALERT_STATE_EVENT, (event) => setEnvelope(event.payload));
    unlisten.then(() => emitTo("main", CALL_ALERT_READY_EVENT));
    return () => {
      unlisten.then((stopListening) => stopListening());
    };
  }, []);

  useEffect(() => {
    const preventContextMenu = (event: MouseEvent) => event.preventDefault();
    document.addEventListener("contextmenu", preventContextMenu);
    return () => document.removeEventListener("contextmenu", preventContextMenu);
  }, []);

  if (!envelope) return null;

  const sendAction = (action: CallAlertAction) => {
    const actionEvent: CallAlertActionEvent = { id: envelope.id, action };
    emitTo("main", CALL_ALERT_ACTION_EVENT, actionEvent);
  };

  return <CallAlertCard alert={envelope.alert} onAction={sendAction} />;
}
