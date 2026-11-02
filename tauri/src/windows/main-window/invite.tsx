import { CopiableInput } from "@/components/ui/copiable-input";
import { HoppAvatar } from "@/components/ui/hopp-avatar";
import { Constants } from "@/constants";
import { useAPI } from "@/services/query";
import useStore from "@/store/store";
import { usePostHog } from "posthog-js/react";
import { HiPlus } from "react-icons/hi2";
import logo from "../../assets/Hopp.png";

const ORBIT_RING_SIZES = [116, 196, 276, 356];

// Offsets from the centre of the rings, so teammates sit on them
const ORBIT_SLOTS = [
  { x: -92, y: -34 },
  { x: 118, y: -72 },
  { x: -62, y: 76 },
  { x: 82, y: 54 },
  { x: -116, y: 74 },
  { x: 136, y: 24 },
];

function Invite() {
  const posthog = usePostHog();
  const { useQuery } = useAPI();
  const teammates = useStore((state) => state.teammates);

  const { data: inviteData, error } = useQuery("get", "/api/auth/get-invite-uuid", undefined, {
    queryHash: `invite`,
    refetchInterval: 10000,
    select: (data) => data,
  });

  return (
    <div className="flex h-[var(--app-height)] w-full flex-col">
      <div
        aria-hidden
        className="relative min-h-0 w-full flex-1 overflow-hidden [mask-image:linear-gradient(black_80%,transparent)]"
      >
        <div className="iridescent absolute left-1/2 top-1/2 size-40 -translate-x-1/2 -translate-y-1/2 rounded-full opacity-20 blur-[48px]" />
        {ORBIT_RING_SIZES.map((ringSize) => (
          <div
            key={ringSize}
            className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 rounded-full border border-border"
            style={{ width: ringSize, height: ringSize }}
          />
        ))}
        <img
          src={logo}
          alt=""
          draggable="false"
          className="absolute left-1/2 top-1/2 w-[72px] -translate-x-1/2 -translate-y-1/2 drop-shadow-[0_10px_20px_rgb(0_0_0/0.3)]"
        />
        {ORBIT_SLOTS.map((slot, index) => {
          const teammate = teammates?.[index];
          return (
            <div
              key={index}
              className="absolute -translate-x-1/2 -translate-y-1/2 animate-in fade-in-0 zoom-in-75 fill-mode-both duration-500"
              style={{
                left: `calc(50% + ${slot.x}px)`,
                top: `calc(50% + ${slot.y}px)`,
                animationDelay: `${index * 50}ms`,
              }}
            >
              {teammate ?
                <HoppAvatar
                  src={teammate.avatar_url || undefined}
                  firstName={teammate.first_name}
                  lastName={teammate.last_name}
                  className="size-8 rounded-[10px]"
                />
              : <div className="flex size-8 items-center justify-center rounded-[10px] border border-dashed border-border-strong bg-background text-faint-foreground">
                  <HiPlus className="size-3.5" />
                </div>
              }
            </div>
          );
        })}
      </div>

      <div className="flex shrink-0 flex-col items-center px-5 pb-5 text-center">
        <h1 className="text-[19px] leading-6 font-semibold tracking-[-0.015em] text-foreground">
          Pairing is better with friends
        </h1>
        <p className="mt-1.5 text-[13px] leading-5 text-muted-foreground">
          Invite your friends to join your team and start pairing together.
        </p>
        <div className="mt-5 w-full text-left">
          {error ?
            <p className="text-center text-xs leading-[18px] text-destructive">Couldn't load your invite link</p>
          : inviteData ?
            <CopiableInput
              value={inviteData?.invite_uuid ? `${Constants.backendUrl}/invitation/${inviteData.invite_uuid}` : ""}
              readOnly
              aria-label="Invite link"
              className="text-muted-foreground"
              onCopy={() => {
                // Send posthog event for custom URL sign-in
                posthog.capture("user_click_app_invitation");
              }}
            />
          : <div className="h-9 w-full animate-pulse rounded-lg bg-subtle" />}
        </div>
      </div>
    </div>
  );
}

export default Invite;
