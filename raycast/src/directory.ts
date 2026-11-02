import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";

export type Teammate = {
  id: string;
  first_name: string;
  last_name: string;
  avatar_url?: string | null;
  is_active?: boolean;
};

export type Directory = {
  key: string;
  teammates: Teammate[];
};

// Hopp keeps this up to date while it runs, and only this user can read it
const DIRECTORY_PATH = join(homedir(), "Library/Application Support/com.hopp.app/launcher.json");

export const readDirectory = async (path = DIRECTORY_PATH): Promise<Directory | null> => {
  try {
    const directory = JSON.parse(await readFile(path, "utf8"));
    return typeof directory.key === "string" && Array.isArray(directory.teammates) ? directory : null;
  } catch {
    return null;
  }
};

export const fullName = (teammate: Teammate) => `${teammate.first_name} ${teammate.last_name}`.trim();

export const callLink = (teammate: Teammate, key: string) =>
  `hopp:///call?${new URLSearchParams({ userId: teammate.id, key })}`;

const findOnlyOnlineMatch = (teammates: Teammate[], name: string): Teammate | null => {
  const words = name.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return null;

  const matches = teammates.filter((teammate) => {
    const teammateName = fullName(teammate).toLowerCase();
    return words.every((word) => teammateName.includes(word));
  });
  const onlyMatch = matches.length === 1 ? matches[0] : undefined;
  return onlyMatch?.is_active ? onlyMatch : null;
};

/**
 * Who a saved quicklink should ring without a second look. Any web page can open a Raycast deeplink and fill in
 * a name, so the link has to carry the token this extension put in its own quicklinks. Only when the name can't
 * be anyone else, and not when they're offline, as that call can only fail.
 */
export const teammateForQuicklink = (
  teammates: Teammate[],
  name: string,
  linkToken: string | undefined,
  savedToken: string,
): Teammate | null => (linkToken && linkToken === savedToken ? findOnlyOnlineMatch(teammates, name) : null);
