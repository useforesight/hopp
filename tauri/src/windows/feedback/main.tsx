import "@/services/sentry";
import "../../App.css";
import React, { useState } from "react";
import ReactDOM from "react-dom/client";
import { useDisableNativeContextMenu } from "@/lib/hooks";
import { initTheme } from "@/lib/theme";
import { tauriUtils } from "../window-utils";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Textarea } from "@/components/ui/textarea";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { Constants } from "@/constants";
import clsx from "clsx";
import createFetchClient from "openapi-fetch";
import type { paths } from "@/openapi";

initTheme();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <FeedbackWindow />
  </React.StrictMode>,
);

const EMOJI_RATINGS = [
  { emoji: "😡", label: "Awful", score: 1 },
  { emoji: "😕", label: "Poor", score: 2 },
  { emoji: "😐", label: "Okay", score: 3 },
  { emoji: "😊", label: "Good", score: 4 },
  { emoji: "🤩", label: "Great", score: 5 },
];

function FeedbackWindow() {
  useDisableNativeContextMenu();

  const [selectedScore, setSelectedScore] = useState<number | null>(null);
  const [feedbackText, setFeedbackText] = useState("");
  const [neverShowAgain, setNeverShowAgain] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);

  // Get params from URL
  const urlParams = new URLSearchParams(window.location.search);
  const teamId = urlParams.get("teamId") || "";
  const roomId = urlParams.get("roomId") || "";
  const participantId = urlParams.get("participantId") || "";

  const handleClose = async () => {
    if (neverShowAgain) {
      await tauriUtils.setCallFeedbackPopup(false);
    }
    await getCurrentWebviewWindow().close();
  };

  const handleSubmit = async () => {
    if (selectedScore === null) return;

    setIsSubmitting(true);

    try {
      // Get stored token for authentication
      const token = await tauriUtils.getStoredToken();

      const client = createFetchClient<paths>({
        baseUrl: Constants.backendUrl,
        headers: token ? { Authorization: `Bearer ${token}` } : undefined,
      });

      const { error } = await client.POST("/api/auth/feedback", {
        body: {
          team_id: teamId,
          room_id: roomId,
          score: selectedScore,
          feedback: feedbackText || undefined,
          metadata: {
            participant_id: participantId,
            submitted_at: new Date().toISOString(),
          },
        },
      });

      if (error) {
        console.error("Failed to submit feedback:", error);
      }
    } catch (error) {
      console.error("Error submitting feedback:", error);
    } finally {
      setIsSubmitting(false);
      await handleClose();
    }
  };

  return (
    <div className="flex h-screen flex-col overflow-hidden window-bg aurora text-foreground">
      <div data-tauri-drag-region className="h-[32px] w-full shrink-0" />

      <div className="flex min-h-0 flex-1 flex-col px-7 pb-5">
        <h1 className="mb-4 text-center text-[22px] leading-7 font-semibold tracking-[-0.02em] text-foreground">
          How was your call?
        </h1>

        <div className="mb-4 grid grid-cols-5 gap-2" role="radiogroup" aria-label="Rating">
          {EMOJI_RATINGS.map(({ emoji, label, score }) => (
            <button
              key={score}
              role="radio"
              aria-checked={selectedScore === score}
              onClick={() => setSelectedScore(score)}
              className={clsx(
                "group flex flex-col items-center gap-1 rounded-xl py-2.5 ring-1 ring-inset transition-[background-color,box-shadow,transform] duration-150 outline-none focus-visible:ring-2 focus-visible:ring-ring active:scale-[0.97]",
                selectedScore === score ? "bg-accent-soft ring-accent" : (
                  "bg-surface ring-border hover:ring-border-strong"
                ),
              )}
            >
              <span
                className={clsx(
                  "text-[28px] leading-9 transition-[filter,opacity,transform] duration-150 select-none",
                  selectedScore === score ? "scale-110" : (
                    "opacity-80 grayscale-30 group-hover:scale-110 group-hover:opacity-100 group-hover:grayscale-0"
                  ),
                )}
              >
                {emoji}
              </span>
              <span
                className={clsx(
                  "text-[11px] leading-4 font-medium transition-colors",
                  selectedScore === score ? "text-foreground" : "text-muted-foreground",
                )}
              >
                {label}
              </span>
            </button>
          ))}
        </div>

        <label htmlFor="feedback-text" className="mb-1.5 block text-[13px] leading-[18px] font-medium text-foreground">
          Did you face any issue?
        </label>
        <Textarea
          id="feedback-text"
          value={feedbackText}
          onChange={(e) => setFeedbackText(e.target.value)}
          placeholder="To err is human, to report bugs is divine"
          className="min-h-0 w-full flex-1 resize-none"
        />

        <div className="mt-4 flex w-full flex-row items-center justify-between">
          <label className="flex items-center gap-2 select-none">
            <Checkbox checked={neverShowAgain} onCheckedChange={(checked) => setNeverShowAgain(checked === true)} />
            <span className="text-xs text-muted-foreground">Never show again</span>
          </label>

          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={handleClose}>
              Close
            </Button>
            <Button
              variant="default"
              onClick={handleSubmit}
              disabled={selectedScore === null || isSubmitting}
              isLoading={isSubmitting}
            >
              Send
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
