import { Button } from "@/components/ui/button";
import { open } from "@tauri-apps/plugin-shell";
import dotsBackground from "../../assets/dots.svg";
import logo from "../../assets/Hopp.png";
import { BlurIn } from "@/components/ui/text-effects";
import { motion } from "framer-motion";
import { CopiableInput } from "@/components/ui/copiable-input";
import { usePostHog } from "posthog-js/react";
import useStore from "@/store/store";
import { tauriUtils } from "@/windows/window-utils";

export const Login = () => {
  const posthog = usePostHog();
  const customServerUrl = useStore((state) => state.customServerUrl);

  const loginJwtUrl = `https://${customServerUrl || (import.meta.env.VITE_API_BASE_URL as string)}/login-app`;

  return (
    <div className="relative flex h-[var(--app-height)] w-full flex-col items-center justify-center overflow-hidden px-9">
      <img
        src={dotsBackground}
        alt=""
        draggable="false"
        className="pointer-events-none absolute left-1/2 top-[27%] w-[620px] max-w-none -translate-x-1/2 -translate-y-1/2 opacity-40 [mask-image:radial-gradient(closest-side,black,transparent)]"
      />
      <div
        aria-hidden
        className="iridescent pointer-events-none absolute left-1/2 top-[27%] size-44 -translate-x-1/2 -translate-y-1/2 rounded-full opacity-30 blur-[56px] dark:opacity-40"
      />

      <motion.div
        className="relative"
        initial={{ opacity: 0, scale: 0.9, y: 8 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        transition={{ type: "spring", stiffness: 260, damping: 22 }}
      >
        <motion.img
          src={logo}
          alt="Hopp"
          draggable="false"
          className="logo-hover h-auto w-[92px] drop-shadow-[0_18px_30px_rgb(0_0_0/0.35)]"
          whileHover={{
            rotate: [-0.5, 0.5, -0.5],
            transition: {
              duration: 0.5,
              repeat: Infinity,
              ease: "easeInOut",
            },
          }}
        />
      </motion.div>

      <div className="relative mt-6 flex flex-col items-center text-center">
        <BlurIn
          as="h1"
          sentence="Hopp on a call"
          className="text-[26px] leading-8 font-semibold tracking-[-0.025em] text-foreground md:text-[26px] md:leading-8"
        />
        <p className="mt-2 text-[13px] leading-5 text-muted-foreground">
          Low-latency, crystal-clear screen sharing that feels like you're coding side by side.
        </p>
      </div>

      <div className="relative mt-6 flex w-full flex-col gap-2.5">
        <Button
          size="lg"
          className="w-full"
          onClick={() => {
            // The flow for login with JWT from the app is:
            // 1. Redirect to "/login-app" inside the web app
            // 2. If the web-app is authenticated, it will fetch a JWT token and redirect to the app (deeplink)
            // 3. If the web-app is not authenticated, it will redirect to the login page, then keep track somehow of the state to redirect back to the app (deeplink)
            open(loginJwtUrl);
            posthog.capture("user_click_jwt_sign");
          }}
        >
          Sign in
        </Button>
        <p className="mt-1.5 text-center text-[11px] leading-4 text-faint-foreground">
          Browser didn't open? Copy the link instead.
        </p>
        <CopiableInput
          value={loginJwtUrl}
          readOnly
          aria-label="Sign-in link"
          className="text-muted-foreground"
          onCopy={() => {
            // Send posthog event for custom URL sign-in
            posthog.capture("user_click_custom_url_sign");
          }}
        />
        <Button
          variant="link"
          size="sm"
          className="mt-1.5 h-auto p-0 text-[11px] leading-4 font-normal text-faint-foreground"
          onClick={() => tauriUtils.openSettingsWindow()}
        >
          Using your own server? Set it in Settings, under General.
        </Button>
      </div>
    </div>
  );
};
