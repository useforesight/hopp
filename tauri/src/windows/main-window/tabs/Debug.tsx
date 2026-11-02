import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import useStore from "@/store/store";
import { socketService } from "@/services/socket";
import { Button } from "@/components/ui/button";
import { PageHeader } from "@/components/ui/page-header";
import { useEffect, useRef, useState } from "react";
import { Textarea } from "@/components/ui/textarea";
import { soundUtils } from "@/lib/sound_utils";
import { validateAndSetAuthToken } from "@/lib/authUtils";
import { invoke } from "@tauri-apps/api/core";
import { useQuery } from "@tanstack/react-query";
import { typedInvoke } from "@/core_payloads";

const debugCardClass = "flex flex-col gap-2 rounded-xl bg-surface p-3 shadow-card ring-1 ring-inset ring-border";
const debugHintClass = "text-xs leading-[18px] text-muted-foreground";

export const Debug = () => {
  const { callTokens, setCallTokens, authToken } = useStore();
  const [isPlaying, setIsPlaying] = useState(false);
  const [trayNotification, setTrayNotification] = useState(false);
  const soundRef = useRef(soundUtils.createPlayer("incoming-call"));

  const { refetch, data, isLoading, isFetching } = useQuery({
    queryKey: ["list_cameras"],
    queryFn: () => typedInvoke("list_webcams"),
  });

  const handleSetTrayNotification = async (enabled: boolean) => {
    try {
      await invoke("set_tray_notification", { enabled });
      setTrayNotification(enabled);
      console.log(`Tray notification set to: ${enabled}`);
    } catch (error) {
      console.error("Failed to set tray notification:", error);
    }
  };

  useEffect(() => {
    return () => {
      try {
        soundRef.current.stop?.();
      } catch (error) {
        console.error("Error stopping sound:", error);
      }
    };
  }, []);

  const toggleSound = async () => {
    console.log("Toggling sound");
    if (isPlaying) {
      soundRef.current.stop();
      setIsPlaying(false);
      return;
    }

    try {
      soundRef.current.play();
      setIsPlaying(true);
    } catch (error) {
      console.error("Error playing sound:", error);
    }
  };

  return (
    <div className="flex w-full flex-col">
      <PageHeader title="Debug" />
      <div className="flex flex-col gap-2.5 px-3.5 pb-4 pt-1">
        <p className="text-xs leading-[18px] text-muted-foreground">
          Something in the app broke for you to be here. Be sure to ping us so we can fix it.
        </p>

        <section className={debugCardClass}>
          <Label htmlFor="authToken">Auth token</Label>
          <span className={debugHintClass}>Paste your authentication token from the web application.</span>
          <Input
            id="authToken"
            type="text"
            placeholder="Auth token"
            value={authToken || ""}
            onChange={async (e) => {
              const newToken = e.target.value;
              await validateAndSetAuthToken(newToken);
            }}
          />
        </section>

        <section className={debugCardClass}>
          <Label htmlFor="callToken">Call tokens</Label>
          <span className={debugHintClass}>A field that you hopefully will never need to use.</span>
          <Textarea
            id="callToken"
            placeholder="Call token"
            value={JSON.stringify(callTokens, null, 2) || ""}
            onChange={(e) => {
              setCallTokens({
                ...JSON.parse(e.target.value),
                timeStarted: new Date(),
                hasAudioEnabled: false,
                hasCameraEnabled: false,
                isRemoteControlEnabled: true,
                participants: [],
              });
            }}
          />
        </section>

        <section className={debugCardClass}>
          <Label>Connection and sound</Label>
          <div className="grid grid-cols-2 gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() =>
                socketService.send({
                  type: "ping",
                  payload: {
                    message: "ping",
                  },
                })
              }
            >
              Ping websocket
            </Button>
            <Button variant="outline" size="sm" onClick={toggleSound}>
              {isPlaying ? "Stop call sound" : "Play call sound"}
            </Button>
          </div>
        </section>

        <section className={debugCardClass}>
          <Label>Core process</Label>
          <div className="grid grid-cols-2 gap-2">
            <Button variant="outline" onClick={() => refetch()} size="sm">
              {isLoading || isFetching ? "Loading…" : "List cameras"}
            </Button>
            <Button variant="outline" onClick={() => typedInvoke("open_stats_window")} size="sm">
              Open stats window
            </Button>
          </div>
          {data &&
            data.map((camera) => (
              <div key={camera.id} className="text-xs leading-[18px] text-muted-foreground">
                {camera.name}
              </div>
            ))}
        </section>

        <section className={debugCardClass}>
          <Label>Tray icon test (macOS)</Label>
          <span className={debugHintClass}>The icon should change automatically with the system theme.</span>
          <div className="grid grid-cols-2 gap-2">
            <Button
              onClick={() => handleSetTrayNotification(false)}
              variant={!trayNotification ? "default" : "outline"}
              size="sm"
            >
              Default icon
            </Button>
            <Button
              onClick={() => handleSetTrayNotification(true)}
              variant={trayNotification ? "default" : "outline"}
              size="sm"
            >
              Notification icon
            </Button>
          </div>
        </section>
      </div>
    </div>
  );
};
