import { components } from "@/openapi";
import { useAPI } from "@/services/query";
import { Input } from "@/components/ui/input";
import Fuse from "fuse.js";
import { FormEvent } from "react";
import { Plus, MoreHorizontal } from "lucide-react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { RoomButton } from "@/components/ui/room-button";
import useStore from "@/store/store";
import { useCallback, useEffect } from "react";
import toast from "react-hot-toast";
import { Button } from "@/components/ui/button";
import { avatarGradient } from "@/components/ui/hopp-avatar";
import { EmptyState, PageHeader } from "@/components/ui/page-header";
import { HiMiniUser } from "react-icons/hi2";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { HiMagnifyingGlass, HiOutlinePencil, HiOutlineTrash } from "react-icons/hi2";
import { useState } from "react";
import doorImage from "@/assets/door.png";
import { useJoinRoom } from "@/lib/hooks";

type Room = components["schemas"]["Room"];

const fuseSearch = (rooms: Room[], searchQuery: string) => {
  const fuse = new Fuse(rooms, {
    keys: ["name"],
    threshold: 0.3,
    shouldSort: true,
  });
  return fuse.search(searchQuery).map((result) => result.item);
};

// Maximum number of avatars to display before showing +N overflow. More won't fit beside the Live label.
const MAX_VISIBLE_AVATARS = 3;

const presenceTileClass =
  "flex size-5 shrink-0 items-center justify-center overflow-hidden rounded-[6px] ring-1 ring-inset ring-white/15";

type BaseUser = components["schemas"]["BaseUser"];

// Component to render room presence avatars with tooltips
const RoomPresenceAvatars = ({
  participantIds,
  getParticipantInfo,
}: {
  participantIds: string[];
  getParticipantInfo: (id: string) => BaseUser | null;
}) => {
  if (participantIds.length === 0) return null;

  const visibleParticipants = participantIds.slice(0, MAX_VISIBLE_AVATARS);
  const overflowCount = participantIds.length - MAX_VISIBLE_AVATARS;
  const overflowParticipants = participantIds.slice(MAX_VISIBLE_AVATARS);

  return (
    <TooltipProvider delayDuration={100} skipDelayDuration={50}>
      <div className="flex min-w-0 flex-row items-center gap-0.5">
        {visibleParticipants.map((participantId) => {
          const info = getParticipantInfo(participantId);
          if (info) {
            return (
              <Tooltip key={participantId}>
                <TooltipTrigger asChild>
                  <div
                    className={presenceTileClass}
                    style={{ background: avatarGradient(info.first_name + info.last_name) }}
                  >
                    {info.avatar_url ?
                      <img
                        src={info.avatar_url}
                        alt={`${info.first_name} ${info.last_name}`}
                        className="size-full object-cover"
                      />
                    : <span className="text-[8px] font-semibold text-white">
                        {info.first_name[0]}
                        {info.last_name[0]}
                      </span>
                    }
                  </div>
                </TooltipTrigger>
                <TooltipContent side="bottom" className="text-xs">
                  {info.first_name} {info.last_name}
                </TooltipContent>
              </Tooltip>
            );
          }
          // Unknown user fallback
          return (
            <Tooltip key={participantId}>
              <TooltipTrigger asChild>
                <div className={`${presenceTileClass} bg-subtle-strong`}>
                  <HiMiniUser className="size-3 text-muted-foreground" />
                </div>
              </TooltipTrigger>
              <TooltipContent side="bottom" className="text-xs">
                Unknown user
              </TooltipContent>
            </Tooltip>
          );
        })}
        {overflowCount > 0 && (
          <Tooltip>
            <TooltipTrigger asChild>
              <div className={`${presenceTileClass} bg-subtle-strong`}>
                <span className="text-[8px] font-medium text-muted-foreground">+{overflowCount}</span>
              </div>
            </TooltipTrigger>
            <TooltipContent side="bottom" className="text-xs">
              <div className="flex flex-col gap-0.5">
                {overflowParticipants.map((participantId) => {
                  const info = getParticipantInfo(participantId);
                  return (
                    <span key={participantId}>{info ? `${info.first_name} ${info.last_name}` : "Unknown user"}</span>
                  );
                })}
              </div>
            </TooltipContent>
          </Tooltip>
        )}
      </div>
    </TooltipProvider>
  );
};

export const Rooms = () => {
  const { authToken, callTokens, teammates, user } = useStore();
  const [searchQuery, setSearchQuery] = useState("");
  const [filteredRooms, setFilteredRooms] = useState<Room[]>([]);
  const [isDeleteDialogOpen, setIsDeleteDialogOpen] = useState(false);
  const [isUpdateDialogOpen, setIsUpdateDialogOpen] = useState(false);
  const [isCreateDialogOpen, setIsCreateDialogOpen] = useState(false);
  const [selectedRoom, setSelectedRoom] = useState<Room | null>(null);
  const { joinRoom, isJoiningRoom } = useJoinRoom();

  const { useQuery } = useAPI();

  // Get current user's rooms
  const {
    data: rooms,
    refetch,
    isLoading: isLoadingRooms,
  } = useQuery("get", "/api/auth/rooms", undefined, {
    enabled: !!authToken,
    refetchInterval: 30_000,
    retry: true,
    queryHash: `rooms-${authToken}`,
    // Avoid refetching on tab change
    staleTime: 30_000,
    select: (data) => data.sort((a, b) => a.name.localeCompare(b.name)),
  });

  // Poll for room presence every 10 seconds
  const { data: roomsPresence } = useQuery("get", "/api/auth/rooms/presence", undefined, {
    enabled: !!authToken,
    refetchInterval: 10_000,
    retry: true,
    queryHash: `rooms-presence-${authToken}`,
    select: (data) => data.rooms || {},
  });

  // Helper to get user info from participant ID
  const getParticipantInfo = useCallback(
    (participantId: string): BaseUser | null => {
      // Check if it's the current user
      if (user && user.id === participantId) {
        return user;
      }

      if (teammates) {
        const teammate = teammates.find((t) => t.id === participantId);
        if (teammate) {
          return teammate;
        }
      }
      // Unknown user
      return null;
    },
    [user, teammates],
  );

  const { useMutation } = useAPI();

  const { mutateAsync: createRoom } = useMutation("post", "/api/auth/room", undefined);

  const handleCreateRoom = async (roomName: string) => {
    try {
      await createRoom({
        body: { name: roomName },
      });
      refetch();
    } catch (error) {
      console.error("Failed to create room:", error);
      toast.error("Failed to create room");
    }
  };

  const { mutateAsync: deleteRoom } = useMutation("delete", "/api/auth/room/{id}", undefined);

  const handleDeleteRoom = async (room: Room) => {
    try {
      // Send JSON body as specified in OpenAPI
      await deleteRoom({
        params: {
          path: {
            id: room.id,
          },
        },
      });

      refetch();
    } catch (error) {
      // Handle 401, 500, or other errors
      console.error("Failed to delete room:", error);
    }
  };

  const { mutateAsync: updateRoom } = useMutation("put", "/api/auth/room/{id}", undefined);

  const handleUpdateRoom = async (room: Room, e: FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    try {
      const formData = new FormData(e.currentTarget);
      const roomName = formData.get("name") as string;

      if (!roomName) {
        toast.error("Provide room name");
        return;
      }

      await updateRoom({
        body: { name: roomName },
        params: {
          path: {
            id: room.id,
          },
        },
      });

      setIsUpdateDialogOpen(false);
      setSelectedRoom(null);
      refetch();
      toast.success("Room renamed successfully");
    } catch (error) {
      // Handle 401, 500, or other errors
      console.error("Failed to update room:", error);
      toast.error("Failed to rename room");
    }
  };

  useEffect(() => {
    if (searchQuery == "") {
      // Set rooms from the fetch response
      if (rooms) {
        setFilteredRooms(rooms);
      }
    } else {
      // Filter rooms based on search query
      if (rooms) {
        const filteredRooms = fuseSearch(rooms, searchQuery);
        setFilteredRooms(filteredRooms);
      }
    }
  }, [rooms, searchQuery]);

  return (
    <div className="flex w-full flex-col">
      <PageHeader
        title="Rooms"
        aside={
          <Dialog open={isCreateDialogOpen} onOpenChange={setIsCreateDialogOpen}>
            <DialogTrigger asChild>
              <Button variant="outline" size="sm" className="-my-1 h-7 gap-1 pl-2 pr-2.5">
                <Plus className="size-3.5" />
                New
              </Button>
            </DialogTrigger>
            <DialogContent container={document.getElementById("app-body")}>
              <form
                className="grid gap-4"
                onSubmit={(e) => {
                  e.preventDefault();
                  const roomName = new FormData(e.currentTarget).get("roomName") as string;
                  if (!roomName.trim()) {
                    toast.error("Provide room name");
                    return;
                  }
                  handleCreateRoom(roomName);
                  setIsCreateDialogOpen(false);
                }}
              >
                <DialogHeader>
                  <DialogTitle>New room</DialogTitle>
                  <DialogDescription>A permanent place for your team to meet.</DialogDescription>
                </DialogHeader>
                <div className="grid gap-2">
                  <Label htmlFor="room-name">Name</Label>
                  <Input id="room-name" name="roomName" placeholder="Watercooler" />
                </div>
                <DialogDescription>Anyone in your team can modify or remove this room.</DialogDescription>
                <DialogFooter>
                  <Button type="submit">Create room</Button>
                </DialogFooter>
              </form>
            </DialogContent>
          </Dialog>
        }
      >
        {(rooms?.length ?? 0) > 0 && (
          <div className="relative">
            <HiMagnifyingGlass className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-faint-foreground" />
            <Input
              type="text"
              aria-label="Search rooms"
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder="Search rooms…"
              className="pl-8"
            />
          </div>
        )}
      </PageHeader>

      <div className="px-3.5 pb-3.5 pt-1">
        {isLoadingRooms ?
          <div className="grid grid-cols-2 gap-2">
            {[0, 1, 2, 3].map((placeholder) => (
              <div key={placeholder} className="h-[78px] animate-pulse rounded-xl bg-subtle" />
            ))}
          </div>
        : !rooms?.length ?
          <EmptyRoomsState onCreateRoomClick={() => setIsCreateDialogOpen(true)} />
        : filteredRooms.length === 0 ?
          <EmptyState
            icon={<HiMagnifyingGlass className="size-5" />}
            title="No room by that name"
            description="Check the spelling, or create a new one."
          />
        : <div className="grid grid-cols-2 gap-2">
            {filteredRooms.map((room, index) => {
              const presenceIds = roomsPresence?.[room.id] ?? [];
              return (
                <RoomButton
                  key={room.id}
                  onClick={() => joinRoom(room)}
                  disabled={!!callTokens?.isInitialisingCall || isJoiningRoom}
                  title={room.name}
                  isCurrentRoom={callTokens?.room?.id === room.id}
                  className="animate-in fade-in-0 slide-in-from-bottom-1 fill-mode-both duration-300"
                  style={{ animationDelay: `${Math.min(index, 10) * 22}ms` }}
                  presenceAvatars={
                    presenceIds.length > 0 ?
                      <RoomPresenceAvatars participantIds={presenceIds} getParticipantInfo={getParticipantInfo} />
                    : undefined
                  }
                  cornerIcon={
                    <DropdownMenu>
                      <DropdownMenuTrigger
                        aria-label={`More options for ${room.name}`}
                        className="flex size-6 items-center justify-center rounded-md text-muted-foreground transition-colors outline-none hover:bg-subtle-strong hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring data-[state=open]:bg-subtle-strong data-[state=open]:text-foreground"
                      >
                        <MoreHorizontal className="size-3.5" />
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end">
                        <DropdownMenuItem
                          className="[&>svg]:size-3.5"
                          onClick={() => {
                            setSelectedRoom(room);
                            setIsUpdateDialogOpen(true);
                          }}
                        >
                          <HiOutlinePencil />
                          Rename room
                        </DropdownMenuItem>
                        <DropdownMenuItem
                          className="text-destructive focus:bg-destructive-soft focus:text-destructive [&>svg]:size-3.5"
                          onClick={() => {
                            setSelectedRoom(room);
                            setIsDeleteDialogOpen(true);
                          }}
                        >
                          <HiOutlineTrash />
                          Delete room
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  }
                />
              );
            })}
          </div>
        }
      </div>

      <Dialog open={isDeleteDialogOpen} onOpenChange={setIsDeleteDialogOpen}>
        <DialogContent container={document.getElementById("app-body")}>
          <DialogHeader>
            <DialogTitle>Delete {selectedRoom?.name ?? "room"}?</DialogTitle>
            <DialogDescription>It's gone for your whole team, and this can't be undone.</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <DialogClose asChild>
              <Button variant="ghost">Cancel</Button>
            </DialogClose>
            <Button
              variant="destructive"
              onClick={() => {
                // Handle delete logic here
                if (selectedRoom) {
                  handleDeleteRoom(selectedRoom);
                  setIsDeleteDialogOpen(false);
                }
              }}
            >
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <Dialog open={isUpdateDialogOpen} onOpenChange={setIsUpdateDialogOpen}>
        <DialogContent container={document.getElementById("app-body")}>
          <form className="grid gap-4" onSubmit={(e) => selectedRoom && handleUpdateRoom(selectedRoom, e)}>
            <DialogHeader>
              <DialogTitle>Rename room</DialogTitle>
              <DialogDescription>Anyone in your team can modify or remove this room.</DialogDescription>
            </DialogHeader>
            <Input id="room-name" name="name" aria-label="Room name" defaultValue={selectedRoom?.name} />
            <DialogFooter>
              <Button type="submit">Save</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </div>
  );
};

const EmptyRoomsState = ({ onCreateRoomClick }: { onCreateRoomClick: () => void }) => {
  return (
    <div className="flex flex-col items-center px-3 pb-6 text-center animate-in fade-in-0 duration-300">
      <img
        src={doorImage}
        alt=""
        className="mb-3 size-32 drop-shadow-[0_12px_24px_rgb(0_0_0/0.25)]"
        draggable="false"
      />
      <p className="text-[15px] leading-5 font-semibold tracking-[-0.01em] text-foreground">
        A door that's always open
      </p>
      <p className="mt-1.5 text-xs leading-[18px] text-muted-foreground">
        Rooms are permanent, named meeting spots for your team's regulars, like daily stand-ups or mob programming.
      </p>
      <Button onClick={onCreateRoomClick} size="sm" className="mt-4">
        Create a room
      </Button>
      <a
        href="https://docs.hopp.so/rooms"
        target="_blank"
        className="mt-2.5 text-xs text-muted-foreground transition-colors hover:text-foreground"
      >
        Read the docs
      </a>
    </div>
  );
};
