import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { invoke } from "@tauri-apps/api/core";
import { open as openInBrowser } from "@tauri-apps/plugin-shell";
import { platform, version } from "@tauri-apps/plugin-os";
import { appVersion } from "@/windows/window-utils.ts";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { toast } from "react-hot-toast";
import { readTextFile } from "@tauri-apps/plugin-fs";
import { Clipboard } from "lucide-react";
import { PageHeader } from "@/components/ui/page-header";
import failGif from "@/assets/fail.gif";

const OWNER = "gethopp";
const REPO = "hopp";
const TEMPLATE = "bug_report.yml";

const getLogs = async (): Promise<string> => {
  try {
    const logPath = await invoke<string | null>("get_logs");
    if (!logPath) {
      return "Log file path could not be found";
    }
    const logContent = await readTextFile(logPath);
    if (!logContent.trim()) {
      return "The log file is empty";
    }
    return logContent;
  } catch (error) {
    return `An error occurred while trying to read the log file: ${error}`;
  }
};

const deactivateHiding = async (value: boolean) => {
  await invoke("set_deactivate_hiding", { deactivate: value });
};

export function Report() {
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [isCopying, setIsCopying] = useState(false);

  useEffect(() => {
    deactivateHiding(true);
    return () => {
      deactivateHiding(false);
    };
  }, []);

  const handleCopyLogs = async () => {
    setIsCopying(true);
    try {
      const logs = await getLogs();
      await writeText(logs);
      toast.success("App logs copied to clipboard!");
    } catch (error) {
      toast.error("Could not copy logs.");
    } finally {
      setIsCopying(false);
    }
  };

  const handleOpenTemplate = async () => {
    setIsSubmitting(true);
    try {
      const osPlatform = await platform();
      const osVersion = await version();

      const base = `https://github.com/${OWNER}/${REPO}/issues/new`;
      const params = new URLSearchParams({
        template: TEMPLATE,
        os: `${osPlatform} ${osVersion}`,
        ver: `${appVersion}`,
      });
      const url = `${base}?${params.toString()}`;

      try {
        await openInBrowser(url);
      } catch {
        window.open(url, "_blank", "noopener");
      }
    } catch (error) {
      console.error("Failed to open bug report form:", error);
      toast.error("Could not open bug report form.");
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <div className="flex w-full flex-col">
      <PageHeader title="Report an issue" />
      <div className="flex flex-col gap-3.5 px-3.5 pb-4 pt-1">
        <img
          src={failGif}
          alt=""
          className="h-[176px] w-full rounded-xl object-cover ring-1 ring-border"
          draggable="false"
        />
        <p className="text-[13px] leading-5 text-muted-foreground">
          This opens a GitHub bug report form in your browser. Copy your logs first so you can paste them in.
        </p>
        <div className="flex w-full items-center gap-2">
          <Button onClick={handleOpenTemplate} className="min-w-0 flex-1" disabled={isSubmitting}>
            {isSubmitting ? "Opening GitHub…" : "Open bug report"}
          </Button>
          <Button className="shrink-0 gap-1.5" onClick={handleCopyLogs} variant="outline" disabled={isCopying}>
            <Clipboard className="size-3.5" />
            Copy logs
          </Button>
        </div>
        <p className="text-xs leading-[18px] text-faint-foreground">
          Learn more in the{" "}
          <a
            href="https://docs.gethopp.app/"
            target="_blank"
            rel="noreferrer"
            className="text-muted-foreground underline underline-offset-2 transition-colors hover:text-foreground"
          >
            documentation
          </a>
          .
        </p>
      </div>
    </div>
  );
}

export default Report;
