import "@/services/sentry";
import "../../App.css";
import React, { useState, useEffect } from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { toast, Toaster } from "react-hot-toast";
import { Button } from "@/components/ui/button";
import { useDisableNativeContextMenu } from "@/lib/hooks";
import { initTheme, toastStyle } from "@/lib/theme";
import { tauriUtils } from "../window-utils";
import logo from "@/assets/Hopp.png";
import clsx from "clsx";
import {
  PiMicrophoneDuotone,
  PiMouseDuotone,
  PiMonitorArrowUpDuotone,
  PiCheckCircleDuotone,
  PiCameraDuotone,
} from "react-icons/pi";

initTheme();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Window />
  </React.StrictMode>,
);

interface PermissionStatus {
  mic: boolean;
  screenShare: boolean;
  accessibility: boolean;
  camera: boolean;
}

function Window() {
  useDisableNativeContextMenu();
  const [permissions, setPermissions] = useState<PermissionStatus>({
    mic: false,
    screenShare: false,
    accessibility: false,
    camera: false,
  });
  const [loading, setLoading] = useState<string | null>(null);

  // Get initial permission values from backend
  useEffect(() => {
    const fetchPermissions = async () => {
      const micPermission = await getMicPermission();
      const screenSharePermission = await getScreenSharePermission();
      const accessibilityPermission = await getAccessibilityPermission();
      const cameraPermission = await getCameraPermission();

      setPermissions({
        mic: micPermission,
        screenShare: screenSharePermission,
        accessibility: accessibilityPermission,
        camera: cameraPermission,
      });
    };

    fetchPermissions();
  }, []);

  const getMicPermission = async (): Promise<boolean> => {
    return tauriUtils.getMicPermission();
  };

  const getScreenSharePermission = async (): Promise<boolean> => {
    return tauriUtils.getScreenSharePermission();
  };

  const getAccessibilityPermission = async (): Promise<boolean> => {
    return tauriUtils.getControlPermission();
  };

  const getCameraPermission = async (): Promise<boolean> => {
    return tauriUtils.getCameraPermission();
  };

  const handlePermissionRequest = async (permission: keyof PermissionStatus) => {
    setLoading(permission);

    try {
      console.log(`Requesting ${permission} permission...`);

      let success = false;

      switch (permission) {
        case "mic":
          await tauriUtils.openMicrophoneSettings();

          // Keep polling until permission is granted
          let micCheckCount = 0;
          while (!success && micCheckCount < 15) {
            await new Promise((resolve) => setTimeout(resolve, 1000)); // Wait 1 second
            success = await getMicPermission();
            micCheckCount++;
          }
          break;
        case "screenShare":
          success = await tauriUtils.triggerScreenSharePermission();
          if (!success) {
            // We sleep for 2 seconds to allow the user to click the open
            // setting prompt the first time. If the permission has not been granted
            // the prompt won't be shown and this is why we have to open the settings
            // manually.
            await new Promise((resolve) => setTimeout(resolve, 2000));
            await tauriUtils.openScreenShareSettings();
          }

          // Keep polling until permission is granted
          // Instead of checking the permission we do the trigger because on our
          // testing the permission wasn't updated in CGPreflightScreenCaptureAccess
          // immediately and it expected a restart (probably)
          let screenShareCheckCount = 0;
          while (!success && screenShareCheckCount < 5) {
            await new Promise((resolve) => setTimeout(resolve, 3000)); // Wait 3 seconds
            success = await tauriUtils.triggerScreenSharePermission();
            screenShareCheckCount++;
          }
          break;
        case "accessibility":
          await tauriUtils.openAccessibilitySettings();
          // Keep polling until permission is granted
          let accessibilityCheckCount = 0;
          while (!success && accessibilityCheckCount < 15) {
            await new Promise((resolve) => setTimeout(resolve, 1000)); // Wait 1 second
            success = await getAccessibilityPermission();
            accessibilityCheckCount++;
          }
          break;
        case "camera":
          await tauriUtils.openCameraSettings();

          // Keep polling until permission is granted
          let cameraCheckCount = 0;
          while (!success && cameraCheckCount < 15) {
            await new Promise((resolve) => setTimeout(resolve, 1000)); // Wait 1 second
            success = await getCameraPermission();
            cameraCheckCount++;
          }
          break;
      }

      if (success) {
        setPermissions((prev) => ({
          ...prev,
          [permission]: true,
        }));
        toast.success(`${getPermissionTitle(permission)} granted!`);
      } else {
        toast.error(`Failed to request ${getPermissionTitle(permission)}`);
      }
    } catch (error) {
      console.error(`Failed to request ${permission} permission:`, error);
      toast.error(`Failed to request ${getPermissionTitle(permission)}`);
    } finally {
      setLoading(null);
    }
  };

  const getPermissionIcon = (permission: keyof PermissionStatus) => {
    const icons = {
      mic: <PiMicrophoneDuotone className="size-6" />,
      camera: <PiCameraDuotone className="size-6" />,
      screenShare: <PiMonitorArrowUpDuotone className="size-6" />,
      accessibility: <PiMouseDuotone className="size-6" />,
    };
    return icons[permission];
  };

  const getPermissionTitle = (permission: keyof PermissionStatus) => {
    const titles = {
      mic: "Microphone access",
      camera: "Camera access",
      screenShare: "Screen sharing access",
      accessibility: "Remote control access",
    };
    return titles[permission];
  };

  const getPermissionDescription = (permission: keyof PermissionStatus) => {
    const descriptions = {
      mic: "So your teammates can hear you. If System Settings opens, go to Privacy & Security, Microphone and allow Hopp.",
      camera:
        "So your teammates can see you. If System Settings opens, go to Privacy & Security, Camera and allow Hopp. No camera? You can ignore this one.",
      screenShare:
        "So you can share your screen. If System Settings opens, go to Privacy & Security, Screen & System Audio Recording and allow Hopp.",
      accessibility:
        "For seamless remote pairing. If System Settings opens, go to Privacy & Security, Accessibility and allow Hopp to control your computer.",
    };
    return descriptions[permission];
  };

  const allPermissionsGranted = Object.values(permissions).every(Boolean);
  const grantedCount = Object.values(permissions).filter(Boolean).length;
  const permissionCount = Object.values(permissions).length;

  // Close window when all permissions are granted (with delay)
  useEffect(() => {
    if (allPermissionsGranted) {
      const timer = setTimeout(async () => {
        try {
          const currentWindow = getCurrentWebviewWindow();
          await currentWindow.close();
        } catch (error) {
          console.error("Failed to close permissions window:", error);
        }
      }, 2000);

      return () => clearTimeout(timer);
    }
  }, [allPermissionsGranted]);

  return (
    <div
      className="aurora flex h-full flex-col overflow-hidden rounded-[12px] text-foreground outline-none"
      tabIndex={0}
    >
      <Toaster position="top-center" toastOptions={{ style: toastStyle }} />
      <div data-tauri-drag-region className="title-panel titlebar h-[28px] w-full shrink-0" />

      <div className="flex min-h-0 flex-1 flex-col gap-5 overflow-y-auto px-8 pb-8 pt-4">
        <div className="flex items-center gap-4">
          <img
            src={logo}
            alt=""
            draggable="false"
            className="size-14 shrink-0 drop-shadow-[0_8px_16px_rgb(0_0_0/0.3)]"
          />
          <div className="min-w-0 flex-1">
            <h1 className="text-[26px] leading-8 font-semibold tracking-[-0.025em] text-foreground">
              Grant permissions
            </h1>
            <p className="mt-0.5 text-[13px] leading-5 text-muted-foreground">
              Hopp needs these to work properly. You can change them later in System Settings.
            </p>
          </div>
        </div>

        <div className="flex items-center gap-3">
          <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-subtle-strong">
            <div
              className="h-full rounded-full bg-success transition-[width] duration-500 ease-out"
              style={{ width: `${(grantedCount / permissionCount) * 100}%` }}
            />
          </div>
          <span className="text-xs text-muted-foreground tabular-nums">
            {allPermissionsGranted ? "All set" : `${grantedCount} of ${permissionCount}`}
          </span>
        </div>

        <div className="grid flex-1 grid-cols-2 gap-3">
          {(["mic", "camera", "accessibility", "screenShare"] as Array<keyof PermissionStatus>).map((permission) => (
            <div
              key={permission}
              className={clsx(
                "flex flex-col gap-3 rounded-2xl p-5 shadow-card ring-1 ring-inset transition-[background-color,box-shadow] duration-300",
                permissions[permission] ? "bg-success-soft ring-success/25" : "bg-surface ring-border",
              )}
            >
              <div className="flex items-center justify-between">
                <div
                  className={clsx(
                    "flex size-11 items-center justify-center rounded-[13px] transition-colors duration-300",
                    permissions[permission] ? "bg-success-soft text-success" : "bg-accent-soft text-accent",
                  )}
                >
                  {getPermissionIcon(permission)}
                </div>
                {permissions[permission] && (
                  <div className="flex flex-row items-center gap-1 text-[13px] font-medium text-success animate-in fade-in-0 zoom-in-95 duration-300">
                    <PiCheckCircleDuotone className="size-4" />
                    <span>Granted</span>
                  </div>
                )}
              </div>

              <div className="flex-1">
                <h2 className="border-none pb-0 text-[15px] leading-5 font-semibold tracking-[-0.01em] text-foreground">
                  {getPermissionTitle(permission)}
                </h2>
                <p className="mt-1 text-[13px] leading-[19px] text-muted-foreground">
                  {getPermissionDescription(permission)}
                </p>
              </div>

              {!permissions[permission] && (
                <Button
                  onClick={() => handlePermissionRequest(permission)}
                  disabled={loading === permission}
                  isLoading={loading === permission}
                  variant="default"
                  className="w-full"
                >
                  {loading === permission ? "Waiting for permission…" : "Grant access"}
                </Button>
              )}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
