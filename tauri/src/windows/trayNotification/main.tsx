import "@/services/sentry";
import "../../App.css";
import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { Button } from "@/components/ui/button";
import { useDisableNativeContextMenu } from "@/lib/hooks";
import { initTheme } from "@/lib/theme";
import trayGif from "@/assets/tray.gif";
import { tauriUtils } from "../window-utils";

const appWindow = getCurrentWebviewWindow();

initTheme();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <InstructionalWindow />
  </React.StrictMode>,
);

function InstructionalWindow() {
  useDisableNativeContextMenu();

  const handleClose = async () => {
    await tauriUtils.hideTrayIconInstruction();
    await appWindow.close();
  };

  return (
    <div
      className="aurora flex h-full w-full flex-col overflow-hidden rounded-lg text-foreground outline-none"
      tabIndex={0}
    >
      <div data-tauri-drag-region className="title-panel titlebar h-[32px] w-full shrink-0" />
      <div className="flex min-h-0 flex-1 flex-row items-center gap-4 px-5 pb-5">
        <img
          src={trayGif}
          alt="Click the tray icon"
          draggable="false"
          className="h-auto w-44 shrink-0 rounded-xl ring-1 ring-border"
        />
        <div className="flex min-w-0 flex-1 flex-col items-start">
          <h1 className="text-[17px] leading-[22px] font-semibold tracking-[-0.015em] text-foreground">
            Hopp lives in your menu bar
          </h1>
          <p className="mt-1 text-[13px] leading-5 text-muted-foreground">Click the tray icon to open it any time.</p>
          <Button variant="outline" size="sm" onClick={handleClose} className="mt-3">
            Don't show this again
          </Button>
        </div>
      </div>
    </div>
  );
}
