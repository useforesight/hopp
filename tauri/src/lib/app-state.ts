import type { AppState } from "@/core_payloads";
import type { components } from "@/openapi";

type BaseUser = components["schemas"]["BaseUser"];
type Room = components["schemas"]["Room"];

/** The call as the `hopp` command sees it. Its mic level is left out: nothing reads it there. */
export type CallForApp = {
  room?: { name: string };
  timeStarted: Date;
  hasAudioEnabled: boolean;
  hasCameraEnabled: boolean;
  participants: { identity: string; name: string; connected: boolean; is_screensharing: boolean }[];
};

const fullName = (person: Pick<BaseUser, "first_name" | "last_name">) =>
  `${person.first_name} ${person.last_name}`.trim();

export const callForApp = (call: CallForApp | null): AppState["call"] => {
  if (!call) return null;
  const you = call.participants.find((person) => person.identity.includes("local"));
  const others = call.participants.filter((person) => person !== you && person.connected);
  return {
    room: call.room?.name ?? null,
    participants: others.map((person) => ({ name: person.name })),
    started: Math.floor(call.timeStarted.getTime() / 1000),
    microphone: call.hasAudioEnabled,
    camera: call.hasCameraEnabled,
    sharing: !!you?.is_screensharing,
    viewing: others.find((person) => person.is_screensharing)?.name ?? null,
  };
};

export const appState = (from: {
  user: Pick<BaseUser, "id" | "first_name" | "last_name"> | null;
  teammates: BaseUser[] | null;
  /** Who's in a call, by user id. */
  callsPresence: Record<string, { peerIds?: string[]; roomName?: string }>;
  rooms: Room[];
  /** Who's in a room, by room id. */
  roomsPresence: Record<string, string[]>;
  call: AppState["call"];
  calling: string | null;
  inviting: string | null;
  incomingCaller: string | null;
  incomingInviter: string | null;
}): AppState => {
  const { user, call } = from;
  if (!user) return { signed_in: false, ringing: null, dialing: null, call, teammates: [], rooms: [] };

  const teammates = from.teammates ?? [];
  const nameOf = (id: string | null) => {
    if (id === null) return null;
    if (id === user.id) return fullName(user);
    const found = teammates.find((teammate) => teammate.id === id);
    return found ? fullName(found) : "Someone";
  };

  const dialing = nameOf(from.calling ?? from.inviting);
  const caller = nameOf(from.incomingCaller);
  const inviter = nameOf(from.incomingInviter);
  const ringing =
    caller !== null ? { name: caller, kind: "call" as const }
    : inviter !== null ? { name: inviter, kind: "invite" as const }
    : null;

  return {
    signed_in: true,
    user: { name: fullName(user) },
    ringing,
    dialing: dialing === null ? null : { name: dialing },
    call,
    teammates: teammates.map((teammate) => {
      const presence = from.callsPresence[teammate.id];
      return {
        id: teammate.id,
        name: fullName(teammate),
        online: !!teammate.is_active,
        room: presence?.roomName ?? (presence?.peerIds?.length ? "a call" : null),
      };
    }),
    rooms: from.rooms.map((room) => ({
      id: room.id,
      name: room.name,
      members: (from.roomsPresence[room.id] ?? []).map((id) => nameOf(id) ?? "Someone"),
    })),
  };
};
