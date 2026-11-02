import { components } from "@/openapi";
import { ParticipantRow } from "./participant-row-wo-livekit";
import { useState, useEffect, useRef, KeyboardEvent as ReactKeyboardEvent } from "react";
import Fuse from "fuse.js";
import { Input } from "./input";
import { Button } from "./button";
import { EmptyState, PageHeader, SectionLabel } from "./page-header";
import { HiMagnifyingGlass, HiOutlineUserPlus } from "react-icons/hi2";
import { useAPI } from "@/services/query";
import useStore from "@/store/store";

type Teammate = components["schemas"]["BaseUser"];

interface ParticipantsProps {
  teammates: Teammate[];
}

const fuseSearch = (teammates: Teammate[], searchQuery: string) => {
  const fuse = new Fuse(teammates, {
    keys: ["first_name", "last_name"],
    threshold: 0.3,
    shouldSort: true,
  });
  return fuse.search(searchQuery).map((result) => result.item);
};

export const Participants = ({ teammates }: ParticipantsProps) => {
  const setTab = useStore((state) => state.setTab);
  const [searchQuery, setSearchQuery] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const [filteredTeammates, setFilteredTeammates] = useState(teammates);

  const { useQuery } = useAPI();
  const { data: rooms } = useQuery("get", "/api/auth/rooms", undefined, {
    staleTime: 10_000,
  });

  useEffect(() => {
    if (searchQuery === "") {
      setFilteredTeammates(teammates);
      return;
    }
    const filteredTeammates = fuseSearch(teammates, searchQuery);
    setFilteredTeammates(filteredTeammates);
  }, [teammates, searchQuery]);

  // Open the window, type a name, press return
  useEffect(() => {
    const focusSearch = () => searchInputRef.current?.focus();
    const focusSearchOnTyping = (event: KeyboardEvent) => {
      const isCharacter =
        event.key.length === 1 && event.key !== " " && !event.metaKey && !event.ctrlKey && !event.altKey;
      // Only when nothing has focus, so buttons keep their Space and open menus keep their typeahead
      if (isCharacter && event.target === document.body) focusSearch();
    };

    focusSearch();
    window.addEventListener("focus", focusSearch);
    window.addEventListener("keydown", focusSearchOnTyping);
    return () => {
      window.removeEventListener("focus", focusSearch);
      window.removeEventListener("keydown", focusSearchOnTyping);
    };
  }, []);

  const onlineTeammates = filteredTeammates?.filter((teammate) => teammate.is_active) || [];
  const offlineTeammates = filteredTeammates?.filter((teammate) => !teammate.is_active) || [];
  const orderedTeammates = [...onlineTeammates, ...offlineTeammates];

  // A selection that scrolled out of the results no longer counts. With a query, the best match is the target.
  const selectionStillListed = orderedTeammates.some((teammate) => teammate.id === selectedId);
  const highlightedId =
    selectionStillListed ? selectedId
    : searchQuery ? (orderedTeammates[0]?.id ?? null)
    : null;

  const findRow = (id: string) => listRef.current?.querySelector<HTMLElement>(`[data-row-id="${CSS.escape(id)}"]`);

  useEffect(() => {
    if (highlightedId) findRow(highlightedId)?.scrollIntoView({ block: "nearest" });
  }, [highlightedId]);

  const handleSearchKeyDown = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (orderedTeammates.length === 0) return;
      const step = event.key === "ArrowDown" ? 1 : -1;
      const current = orderedTeammates.findIndex((teammate) => teammate.id === highlightedId);
      const next =
        current === -1 ?
          step === 1 ?
            0
          : orderedTeammates.length - 1
        : (current + step + orderedTeammates.length) % orderedTeammates.length;
      setSelectedId(orderedTeammates[next]?.id ?? null);
    } else if (event.key === "Enter") {
      if (!highlightedId) return;
      event.preventDefault();
      findRow(highlightedId)?.querySelector<HTMLButtonElement>("[data-row-action]")?.click();
    } else if (event.key === "Escape") {
      setSearchQuery("");
      setSelectedId(null);
    }
  };

  const renderRows = (rows: Teammate[], firstRowIndex: number) => (
    <div className="flex flex-col gap-0.5">
      {rows.map((teammate, index) => (
        <div
          key={teammate.id}
          className="animate-in fade-in-0 slide-in-from-bottom-1 fill-mode-both duration-300"
          style={{ animationDelay: `${Math.min(firstRowIndex + index, 10) * 22}ms` }}
        >
          <ParticipantRow user={teammate} rooms={rooms || []} highlighted={teammate.id === highlightedId} />
        </div>
      ))}
    </div>
  );

  return (
    <div className="flex w-full flex-col">
      <PageHeader
        title="People"
        aside={
          teammates.length > 0 && (
            <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
              <span className="size-1.5 rounded-full bg-success" />
              {teammates.filter((teammate) => teammate.is_active).length} online
            </span>
          )
        }
      >
        <div className="relative">
          <HiMagnifyingGlass className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-faint-foreground" />
          <Input
            ref={searchInputRef}
            type="text"
            aria-label="Search teammates"
            placeholder="Search teammates…"
            value={searchQuery}
            onChange={(e) => {
              setSearchQuery(e.target.value);
              setSelectedId(null);
            }}
            onKeyDown={handleSearchKeyDown}
            className="pl-8"
          />
        </div>
      </PageHeader>

      <div ref={listRef} className="flex flex-col px-1.5 pb-3">
        {teammates.length === 0 ?
          <EmptyState
            icon={<HiOutlineUserPlus className="size-5" />}
            title="It's quiet in here"
            description="Hopp is better with your team. Invite someone to start pairing."
          >
            <Button size="sm" onClick={() => setTab("invite")}>
              Invite a teammate
            </Button>
          </EmptyState>
        : orderedTeammates.length === 0 ?
          <EmptyState
            icon={<HiMagnifyingGlass className="size-5" />}
            title="No one by that name"
            description="Check the spelling, or invite them to your team."
          />
        : <>
            {onlineTeammates.length > 0 && (
              <section>
                <SectionLabel>Online · {onlineTeammates.length}</SectionLabel>
                {renderRows(onlineTeammates, 0)}
              </section>
            )}
            {offlineTeammates.length > 0 && (
              <section className={onlineTeammates.length > 0 ? "mt-1.5" : undefined}>
                <SectionLabel>Offline · {offlineTeammates.length}</SectionLabel>
                {renderRows(offlineTeammates, onlineTeammates.length)}
              </section>
            )}
          </>
        }
      </div>
    </div>
  );
};
