import { Avatar, AvatarFallback, AvatarImage } from "@radix-ui/react-avatar";
import { clsx } from "clsx";
import { cn } from "@/lib/utils";
import { LuMicOff } from "react-icons/lu";

type Status = "online" | "offline";

interface CallPeer {
  avatarUrl?: string;
  firstName: string;
  lastName: string;
}

interface HoppAvatarProps {
  src?: string;
  firstName: string;
  lastName: string;
  status?: Status;
  className?: string;
  isMuted?: boolean;
  callPeers?: CallPeer[];
}

const AvatarGradients: string[] = [
  "linear-gradient(135deg, #8f8eff, #4f46e5)",
  "linear-gradient(135deg, #34d399, #0d9488)",
  "linear-gradient(135deg, #fbbf24, #ea580c)",
  "linear-gradient(135deg, #22d3ee, #2563eb)",
  "linear-gradient(135deg, #f472b6, #9333ea)",
  "linear-gradient(135deg, #fb7185, #e11d48)",
];

// Source: https://stackoverflow.com/a/7616484
const generateHash = (string: string) => {
  let hash = 0;
  for (const char of string) {
    hash = (hash << 5) - hash + char.charCodeAt(0);
    hash |= 0; // Constrain to 32bit integer
    // Always return a positive number
    hash = Math.abs(hash);
  }
  return hash;
};

export function pickFromArray<T>(key: string, options: readonly T[]): T {
  const index = generateHash(key) % options.length;
  return options[index] as T;
}

export const avatarGradient = (name: string) => pickFromArray(name, AvatarGradients);

const PersonAvatar = ({
  src,
  firstName,
  lastName,
  className,
  fallbackClassName,
}: {
  src?: string;
  firstName: string;
  lastName: string;
  className?: string;
  fallbackClassName: string;
}) => (
  <Avatar
    className={cn("flex shrink-0 items-center justify-center overflow-hidden text-white", className)}
    style={{ background: avatarGradient(firstName + lastName) }}
  >
    <AvatarImage className="h-full object-cover" src={src || ""} />
    <AvatarFallback className={clsx("font-semibold tracking-tight", fallbackClassName)}>
      {firstName[0]}
      {lastName[0]}
    </AvatarFallback>
  </Avatar>
);

export const HoppAvatar = ({ src, firstName, lastName, status, className, isMuted, callPeers }: HoppAvatarProps) => {
  const peers = callPeers ?? [];

  if (peers.length === 0) {
    return (
      <div className="relative overflow-visible">
        <PersonAvatar
          src={src}
          firstName={firstName}
          lastName={lastName}
          className={clsx(
            "size-9 rounded-[11px] shadow-card",
            status === "offline" && "opacity-55 saturate-50",
            className,
          )}
          fallbackClassName="text-[12px]"
        />
        {/* Absolute blanket for muted indicator */}
        {isMuted && (
          <div className="absolute inset-0 flex items-center justify-center rounded-[inherit] bg-black/45">
            <LuMicOff className="size-4 text-white" />
          </div>
        )}
        {status && (
          <div
            className={clsx(
              "absolute -bottom-0.5 -right-0.5 size-[11px] rounded-full border-[2.5px] border-background",
              {
                "bg-success": status === "online",
                "bg-faint-foreground": status === "offline",
              },
            )}
          />
        )}
      </div>
    );
  }

  if (peers.length === 1 && peers[0]) {
    // Diagonal based, a bit of overlap, and no status badges
    return (
      <div className="relative size-9 overflow-visible">
        <PersonAvatar
          src={src}
          firstName={firstName}
          lastName={lastName}
          className={clsx("size-[23px] rounded-[7px]", className)}
          fallbackClassName="text-[8px]"
        />
        <PersonAvatar
          src={peers[0].avatarUrl}
          firstName={peers[0].firstName}
          lastName={peers[0].lastName}
          className={clsx("absolute bottom-0 right-0 size-[23px] rounded-[7px] ring-2 ring-background", className)}
          fallbackClassName="text-[8px]"
        />
      </div>
    );
  }

  if (peers.length >= 2 && peers.every((peer) => peer !== undefined)) {
    const isOverflow = peers.length > 3;
    const visiblePeers = peers.slice(0, isOverflow ? 2 : 3);

    // Grid based, no overlap, and no status badges
    // For >4 peers, we add a +N text element to the bottom right
    return (
      <div className="relative grid size-9 grid-cols-2 gap-0.5 overflow-visible">
        <PersonAvatar
          src={src}
          firstName={firstName}
          lastName={lastName}
          className={clsx("size-[17px] rounded-[5px]", className)}
          fallbackClassName="text-[7px]"
        />
        {visiblePeers.map((peer, i) => (
          <PersonAvatar
            key={i}
            src={peer.avatarUrl}
            firstName={peer.firstName}
            lastName={peer.lastName}
            className={clsx("size-[17px] rounded-[5px]", className)}
            fallbackClassName="text-[7px]"
          />
        ))}
        {isOverflow && (
          <div className="flex size-[17px] shrink-0 items-center justify-center overflow-hidden rounded-[5px] bg-subtle-strong text-[9px] font-semibold text-muted-foreground">
            +{peers.length - 2}
          </div>
        )}
      </div>
    );
  }

  return null;
};
